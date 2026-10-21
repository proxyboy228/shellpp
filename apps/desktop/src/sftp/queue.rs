//! The transfer queue of the SFTP tab (UI 5.6, 5.7; D46): the runner on
//! top of `shellpp_ssh::transfer_queue` and its panel at the bottom of the
//! tab.
//!
//! - A transfer copies one entry of a panel into the folder the other panel
//!   shows: upload, download, between two hosts (files), or a local copy.
//!   Folders go with everything inside (`transfer::upload_tree`,
//!   `download_tree`; links by D56, and what was left out is counted on the
//!   row).
//! - At most `sftp.parallel_transfers` run at once (3 by default).
//! - A name that exists in the target folder follows `sftp.name_conflict`;
//!   "ask" shows the conflict dialog (Replace, Skip, Keep both, "Apply to
//!   all").
//! - Pause stops a running transfer where it is (its `.part` stays);
//!   Resume and Retry continue from the `.part`; Cancel ends it.
//! - Progress is read four times a second from the transfer's counter.
//!
//! A remote transfer runs on the session runtime of its panel's SFTP
//! session; a local copy on a background thread. Nothing here logs names or
//! paths.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use gpui::prelude::*;
use gpui::{
    App, ClickEvent, Context, Entity, EventEmitter, FontWeight, IntoElement, SharedString, Task,
    Window, div, px,
};
use gpui_kit::TestSupportExt as _;
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Sizable as _, WindowExt as _, h_flex,
    v_flex,
};
use shellpp_ssh::transfer_queue::{
    ConflictChoice, ConflictOutcome, ConflictPolicy, DEFAULT_PARALLEL, Direction, PARALLEL_RANGE,
    State, TransferId, TransferQueue, renamed,
};
use shellpp_ssh::{Sftp, SftpError};

use super::listing::{self, Entry};
use super::panel::{self, Side};
use super::strings;
use crate::hosts::controls::{ControlButton, ControlKind, ICON_SIZE};
use crate::sessions::SessionHandle;
use crate::state::AppState;
use crate::theme::metrics;

/// How often running transfers report their progress to the panel.
const TICK: Duration = Duration::from_millis(250);
/// Height of a row of the queue.
const ROW_HEIGHT: gpui::Pixels = px(36.);
/// Rows shown before the queue scrolls.
const VISIBLE_ROWS: f32 = 5.;

/// One side of a transfer: a local folder, or a folder of a host.
#[derive(Clone)]
pub enum Place {
    Local(PathBuf),
    Remote {
        handle: SessionHandle,
        sftp: Arc<Sftp>,
        dir: String,
    },
}

impl std::fmt::Debug for Place {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Place::Local(_) => f.write_str("Place::Local"),
            Place::Remote { .. } => f.write_str("Place::Remote"),
        }
    }
}

impl std::fmt::Debug for Transfers {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Transfers")
            .field("transfers", &self.queue.transfers().len())
            .field("running", &self.running.len())
            .finish_non_exhaustive()
    }
}

impl Place {
    fn is_remote(&self) -> bool {
        matches!(self, Place::Remote { .. })
    }

    /// `name` inside this folder.
    fn child(&self, name: &str) -> Spot {
        match self {
            Place::Local(dir) => Spot::Local(dir.join(name)),
            Place::Remote { sftp, dir, .. } => Spot::Remote(
                sftp.clone(),
                panel::join(dir, name, listing::PathStyle::Posix),
            ),
        }
    }

    /// The runtime that runs this side's requests.
    fn handle(&self) -> Option<&SessionHandle> {
        match self {
            Place::Local(_) => None,
            Place::Remote { handle, .. } => Some(handle),
        }
    }
}

/// One file or folder of a transfer.
#[derive(Clone)]
enum Spot {
    Local(PathBuf),
    Remote(Arc<Sftp>, String),
}

/// What a queued transfer does.
struct Job {
    entry: Entry,
    from: Place,
    to: Place,
    /// The side the target folder shows (to list it again when done).
    to_side: Side,
    /// The name in the target folder (a "Keep both" answer changes it).
    name: String,
    /// The answer to a name conflict, once given.
    choice: Option<ConflictChoice>,
    /// Continue from the `.part` (Resume, Retry).
    resume: bool,
    /// What a folder transfer left out (D56).
    skipped: usize,
    /// The size and date of the file the target folder already holds (the
    /// conflict dialog).
    existing: Option<(u64, Option<u64>)>,
}

/// A running transfer.
struct Running {
    done: Arc<AtomicU64>,
    abort: Option<tokio::task::AbortHandle>,
    _task: Task<()>,
}

/// What the queue tells the tab.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransfersEvent {
    /// A transfer into the folder of `side` ended: list it again.
    Finished(Side),
}

/// The queue of one SFTP tab.
pub struct Transfers {
    state: Entity<AppState>,
    queue: TransferQueue,
    jobs: HashMap<TransferId, Job>,
    running: HashMap<TransferId, Running>,
    collapsed: bool,
    /// The conflict dialog is open for this transfer.
    asking: Option<TransferId>,
    ticker: Option<Task<()>>,
}

impl EventEmitter<TransfersEvent> for Transfers {}

/// The direction of a copy from `from` to `to`.
fn direction(from: &Place, to: &Place) -> Direction {
    match (from.is_remote(), to.is_remote()) {
        (false, true) => Direction::Upload,
        (true, false) => Direction::Download,
        (true, true) => Direction::RemoteToRemote,
        // A local copy is shown as a download (both ends are here).
        (false, false) => Direction::Download,
    }
}

impl Transfers {
    pub fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        let (parallel, policy) = settings(&state, cx);
        Self {
            state,
            queue: TransferQueue::new(parallel, policy),
            jobs: HashMap::new(),
            running: HashMap::new(),
            collapsed: false,
            asking: None,
            ticker: None,
        }
    }

    pub fn transfers(&self) -> &[shellpp_ssh::transfer_queue::Transfer] {
        self.queue.transfers()
    }

    pub fn is_empty(&self) -> bool {
        self.queue.transfers().is_empty()
    }

    pub fn active(&self) -> usize {
        self.queue
            .transfers()
            .iter()
            .filter(|transfer| transfer.state.is_open())
            .count()
    }

    /// Queues a copy of `entries` from `from` into `to`.
    pub fn add(
        &mut self,
        entries: Vec<Entry>,
        from: Place,
        to: Place,
        to_side: Side,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (parallel, _) = settings(&self.state, cx);
        self.queue.set_parallel(parallel);
        for entry in entries {
            // Only one plain name goes into a path (never a separator, "."
            // or ".."), whatever a listing held.
            if !super::panel::valid_name(&entry.name) {
                continue;
            }
            let direction = direction(&from, &to);
            let total = (!entry.is_folder_like()).then_some(entry.size);
            let id = self.queue.add(&entry.name, direction, total);
            // A name this system's files cannot have gets the suggestion of
            // D46 (Windows: reserved names and characters).
            let name = match (&to, cfg!(windows)) {
                (Place::Local(_), true) => shellpp_ssh::local_names::windows_safe_name(&entry.name)
                    .unwrap_or_else(|| entry.name.clone()),
                _ => entry.name.clone(),
            };
            self.jobs.insert(
                id,
                Job {
                    name,
                    entry,
                    from: from.clone(),
                    to: to.clone(),
                    to_side,
                    choice: None,
                    resume: false,
                    skipped: 0,
                    existing: None,
                },
            );
        }
        self.collapsed = false;
        self.pump(window, cx);
        cx.notify();
    }

    /// Starts what the queue lets start.
    fn pump(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for id in self.queue.start_next() {
            self.start(id, window, cx);
        }
        if self.ticker.is_none() && !self.running.is_empty() {
            self.ticker = Some(cx.spawn(async move |this, cx| {
                loop {
                    cx.background_executor().timer(TICK).await;
                    let alive = this
                        .update(cx, |this, cx| {
                            let now = Instant::now();
                            for (id, running) in &this.running {
                                this.queue
                                    .progress(*id, running.done.load(Ordering::Relaxed), now);
                            }
                            cx.notify();
                            !this.running.is_empty()
                        })
                        .unwrap_or(false);
                    if !alive {
                        let _ = this.update(cx, |this, _| this.ticker = None);
                        break;
                    }
                }
            }));
        }
    }

    /// Checks the target name, then copies.
    fn start(&mut self, id: TransferId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(job) = self.jobs.get(&id) else {
            return;
        };
        let target = job.to.child(&job.name);
        let source = source_spot(job);
        let folder = job.entry.is_folder_like();
        let choice = job.choice;
        let resume = job.resume;
        let done = Arc::new(AtomicU64::new(0));
        let counter = done.clone();
        let taken_place = job.to.clone();
        // Where the work runs: the target's session, else the source's, else
        // a background thread.
        let handle = job.to.handle().or_else(|| job.from.handle()).cloned();
        let work = async move {
            // A name that exists, unless already answered.
            if choice.is_none()
                && let Some(existing) = existing(&target).await
            {
                return Outcome::Conflict(existing);
            }
            let target = match choice {
                Some(ConflictChoice::Rename) => match free_name(&taken_place, &target).await {
                    Some(target) => target,
                    None => return Outcome::Failed(strings::LIST_FAILED.to_owned()),
                },
                _ => target,
            };
            let mut progress = move |bytes: u64| counter.store(bytes, Ordering::Relaxed);
            copy(
                &source,
                &target,
                folder,
                resume && choice != Some(ConflictChoice::Replace),
                &mut progress,
            )
            .await
        };
        let (task, abort): (Task<Outcome>, Option<tokio::task::AbortHandle>) = match handle {
            Some(handle) => {
                let join = handle.spawn(work);
                let abort = join.abort_handle();
                (
                    cx.background_spawn(async move { join.await.unwrap_or(Outcome::Stopped) }),
                    Some(abort),
                )
            }
            None => {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build();
                (
                    cx.background_spawn(async move {
                        match runtime {
                            Ok(runtime) => runtime.block_on(work),
                            Err(error) => Outcome::Failed(error.to_string()),
                        }
                    }),
                    None,
                )
            }
        };
        let running = cx.spawn_in(window, async move |this, cx| {
            let outcome = task.await;
            let _ = this.update_in(cx, |this, window, cx| this.ended(id, outcome, window, cx));
        });
        self.running.insert(
            id,
            Running {
                done,
                abort,
                _task: running,
            },
        );
    }

    fn ended(
        &mut self,
        id: TransferId,
        outcome: Outcome,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.running.remove(&id);
        let side = self.jobs.get(&id).map(|job| job.to_side);
        match outcome {
            Outcome::Done { skipped } => {
                if let Some(job) = self.jobs.get_mut(&id) {
                    job.skipped = skipped;
                }
                self.queue.finish(id);
                if let Some(side) = side {
                    cx.emit(TransfersEvent::Finished(side));
                }
            }
            Outcome::Failed(reason) => self.queue.fail(id, &reason),
            // Paused or cancelled: the state is already set.
            Outcome::Stopped => {}
            Outcome::Conflict(existing) => match self.queue.conflict(id) {
                ConflictOutcome::Decided(ConflictChoice::Skip) => {}
                ConflictOutcome::Decided(choice) => {
                    // The policy answered: the transfer still holds its
                    // slot and runs again with the answer.
                    if let Some(job) = self.jobs.get_mut(&id) {
                        job.choice = Some(choice);
                    }
                    self.start(id, window, cx);
                }
                ConflictOutcome::Ask => {
                    if let Some(job) = self.jobs.get_mut(&id) {
                        job.existing = Some(existing);
                    }
                    self.ask_conflict(id, window, cx)
                }
            },
        }
        self.pump(window, cx);
        cx.notify();
    }

    /// The answer of the conflict dialog.
    pub fn answer(
        &mut self,
        id: TransferId,
        choice: ConflictChoice,
        for_all: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.asking = None;
        if let Some(job) = self.jobs.get_mut(&id) {
            job.choice = Some(choice);
        }
        self.queue.answer_conflict(id, choice, for_all);
        for (other, choice) in self.queue.decided_by_all() {
            if let Some(job) = self.jobs.get_mut(&other) {
                job.choice = Some(choice);
            }
        }
        // The next waiting conflict, if any.
        if let Some(next) = self
            .queue
            .transfers()
            .iter()
            .find(|transfer| transfer.state == State::Conflict)
            .map(|transfer| transfer.id)
        {
            self.ask_conflict(next, window, cx);
        }
        self.pump(window, cx);
        cx.notify();
    }

    pub fn pause(&mut self, id: TransferId, cx: &mut Context<Self>) {
        self.queue.pause(id);
        if let Some(running) = self.running.remove(&id)
            && let Some(abort) = running.abort
        {
            abort.abort();
        }
        cx.notify();
    }

    /// Resume a paused transfer, or Retry a failed one.
    pub fn resume(&mut self, id: TransferId, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(job) = self.jobs.get_mut(&id) {
            job.resume = true;
        }
        self.queue.resume(id);
        self.pump(window, cx);
        cx.notify();
    }

    pub fn cancel(&mut self, id: TransferId, cx: &mut Context<Self>) {
        self.queue.cancel(id);
        if let Some(running) = self.running.remove(&id)
            && let Some(abort) = running.abort
        {
            abort.abort();
        }
        cx.notify();
    }

    /// Cancels every open transfer and empties the queue (the screenshot
    /// run starts each state from an empty queue).
    pub fn clear_all(&mut self, cx: &mut Context<Self>) {
        let ids: Vec<TransferId> = self.queue.transfers().iter().map(|t| t.id).collect();
        for id in ids {
            self.cancel(id, cx);
        }
        self.asking = None;
        self.clear_finished(cx);
    }

    pub fn clear_finished(&mut self, cx: &mut Context<Self>) {
        self.queue.clear_finished();
        let kept: Vec<TransferId> = self.queue.transfers().iter().map(|t| t.id).collect();
        self.jobs.retain(|id, _| kept.contains(id));
        cx.notify();
    }

    pub fn set_collapsed(&mut self, collapsed: bool, cx: &mut Context<Self>) {
        self.collapsed = collapsed;
        cx.notify();
    }

    /// The conflict dialog (UI 5.7) for `id`.
    fn ask_conflict(&mut self, id: TransferId, window: &mut Window, cx: &mut Context<Self>) {
        if self.asking.is_some() {
            return;
        }
        let Some(job) = self.jobs.get(&id) else {
            return;
        };
        self.asking = Some(id);
        let name: SharedString = job.name.clone().into();
        let copied = format!(
            "{}, {}",
            job.entry.size_text(),
            panel::date_text(job.entry.modified)
        );
        let here = job.existing.map(|(size, modified)| {
            format!(
                "{}, {}",
                listing::human_size(size),
                panel::date_text(modified)
            )
        });
        let runner = cx.entity().downgrade();
        let all = Arc::new(std::sync::atomic::AtomicBool::new(false));
        window.open_dialog(cx, move |dialog, _, cx| {
            let answer = |choice: ConflictChoice| {
                let runner = runner.clone();
                let all = all.clone();
                move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                    window.close_dialog(cx);
                    let for_all = all.load(Ordering::Relaxed);
                    let _ = runner.update(cx, |runner, cx| {
                        runner.answer(id, choice, for_all, window, cx)
                    });
                }
            };
            let toggle = all.clone();
            let checked = all.load(Ordering::Relaxed);
            dialog
                .w(px(440.))
                .bg(cx.theme().popover)
                .rounded(metrics::RADIUS_OVERLAY)
                .close_button(false)
                .title(
                    div()
                        .text_size(metrics::FONT_SIZE_TITLE)
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(strings::CONFLICT_TITLE),
                )
                .child(
                    v_flex()
                        .id("sftp-conflict")
                        .test_support()
                        .gap(metrics::SPACE_2)
                        .child(
                            div()
                                .font_family(crate::terminal::fonts::JETBRAINS_MONO)
                                .text_size(metrics::FONT_SIZE_BODY)
                                .child(name.clone()),
                        )
                        .when_some(here.clone(), |this, here| {
                            this.child(
                                div()
                                    .text_size(metrics::FONT_SIZE_DENSE)
                                    .text_color(cx.theme().muted_foreground)
                                    .child(format!("{}: {here}", strings::CONFLICT_HERE)),
                            )
                        })
                        .child(
                            div()
                                .text_size(metrics::FONT_SIZE_DENSE)
                                .text_color(cx.theme().muted_foreground)
                                .child(format!("{}: {copied}", strings::CONFLICT_COPIED)),
                        )
                        .child(
                            Checkbox::new("sftp-conflict-all")
                                .checked(checked)
                                .label(strings::CONFLICT_ALL)
                                .on_click(move |checked: &bool, window, _| {
                                    toggle.store(*checked, Ordering::Relaxed);
                                    window.refresh();
                                }),
                        ),
                )
                .footer(
                    h_flex()
                        .w_full()
                        .justify_end()
                        .gap(metrics::SPACE_2)
                        .child(
                            ControlButton::new("sftp-conflict-skip", ControlKind::Neutral)
                                .label(strings::CONFLICT_SKIP)
                                .on_click(answer(ConflictChoice::Skip)),
                        )
                        .child(
                            ControlButton::new("sftp-conflict-rename", ControlKind::Neutral)
                                .label(strings::CONFLICT_RENAME)
                                .on_click(answer(ConflictChoice::Rename)),
                        )
                        .child(
                            ControlButton::new("sftp-conflict-replace", ControlKind::Accent)
                                .label(strings::CONFLICT_REPLACE)
                                .on_click(answer(ConflictChoice::Replace)),
                        ),
                )
        });
    }
}

/// `sftp.parallel_transfers` and `sftp.name_conflict`.
fn settings(state: &Entity<AppState>, cx: &App) -> (usize, ConflictPolicy) {
    let values = state
        .read(cx)
        .core()
        .and_then(|core| core.settings_get().ok());
    let parallel = values
        .as_ref()
        .and_then(|values| values.get("sftp.parallel_transfers"))
        .and_then(serde_json::Value::as_u64)
        .map(|value| (value as usize).clamp(*PARALLEL_RANGE.start(), *PARALLEL_RANGE.end()))
        .unwrap_or(DEFAULT_PARALLEL);
    let policy = values
        .as_ref()
        .and_then(|values| values.get("sftp.name_conflict"))
        .and_then(serde_json::Value::as_str)
        .map(ConflictPolicy::from_setting)
        .unwrap_or(ConflictPolicy::Ask);
    (parallel, policy)
}

/// How a transfer ended.
enum Outcome {
    Done {
        skipped: usize,
    },
    Failed(String),
    /// The target name exists (its size and date) and nothing was answered
    /// yet.
    Conflict((u64, Option<u64>)),
    /// Paused or cancelled (the task was aborted).
    Stopped,
}

fn source_spot(job: &Job) -> Spot {
    job.from.child(&job.entry.name)
}

fn reason(error: SftpError) -> String {
    error.to_string()
}

/// The size and modification time of `spot` when it exists.
async fn existing(spot: &Spot) -> Option<(u64, Option<u64>)> {
    match spot {
        Spot::Local(path) => {
            let meta = tokio::fs::symlink_metadata(path).await.ok()?;
            let modified = meta
                .modified()
                .ok()
                .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|since| since.as_secs());
            Some((meta.len(), modified))
        }
        Spot::Remote(sftp, path) => sftp
            .stat(path)
            .await
            .ok()
            .map(|entry| (entry.size, entry.modified)),
    }
}

/// A free name next to `target` (`report (2).txt`, …) for "Keep both".
async fn free_name(place: &Place, target: &Spot) -> Option<Spot> {
    let (taken, name): (Vec<String>, String) = match (place, target) {
        (Place::Local(dir), Spot::Local(path)) => {
            let mut names = Vec::new();
            let mut read = tokio::fs::read_dir(dir).await.ok()?;
            while let Ok(Some(entry)) = read.next_entry().await {
                names.push(entry.file_name().to_string_lossy().into_owned());
            }
            (names, path.file_name()?.to_string_lossy().into_owned())
        }
        (Place::Remote { sftp, dir, .. }, Spot::Remote(_, path)) => {
            let names = sftp
                .list(dir)
                .await
                .ok()?
                .into_iter()
                .map(|entry| entry.name)
                .collect();
            (names, path.rsplit('/').next()?.to_owned())
        }
        _ => return None,
    };
    let free = renamed(&name, |candidate| {
        taken.iter().any(|name| name == candidate)
    });
    Some(place.child(&free))
}

/// Copies `source` to `target` (a folder with everything inside).
async fn copy(
    source: &Spot,
    target: &Spot,
    folder: bool,
    resume: bool,
    progress: &mut (dyn FnMut(u64) + Send),
) -> Outcome {
    use shellpp_ssh::transfer;
    let result: Result<usize, String> = match (source, target) {
        (Spot::Local(from), Spot::Remote(sftp, to)) => {
            if folder {
                transfer::upload_tree(sftp, from, to, progress)
                    .await
                    .map(|summary| summary.skipped.len())
                    .map_err(reason)
            } else {
                sftp.upload(from, to, resume, progress)
                    .await
                    .map(|_| 0)
                    .map_err(reason)
            }
        }
        (Spot::Remote(sftp, from), Spot::Local(to)) => {
            if folder {
                transfer::download_tree(sftp, from, to, progress)
                    .await
                    .map(|summary| summary.skipped.len())
                    .map_err(reason)
            } else {
                sftp.download(from, to, resume, progress)
                    .await
                    .map(|_| 0)
                    .map_err(reason)
            }
        }
        (Spot::Remote(from_sftp, from), Spot::Remote(to_sftp, to)) => {
            if folder {
                Err(strings::BETWEEN_HOSTS_FOLDERS.to_owned())
            } else {
                transfer::copy_between(from_sftp, from, to_sftp, to, progress)
                    .await
                    .map(|_| 0)
                    .map_err(reason)
            }
        }
        (Spot::Local(from), Spot::Local(to)) => local_copy(from, to, progress).map(|_| 0),
    };
    match result {
        Ok(skipped) => Outcome::Done { skipped },
        Err(reason) => Outcome::Failed(reason),
    }
}

/// A local copy (both panels local): files, folders with everything
/// inside; links are copied as what they point to only for files.
fn local_copy(
    from: &Path,
    to: &Path,
    progress: &mut (dyn FnMut(u64) + Send),
) -> Result<u64, String> {
    let meta = std::fs::metadata(from).map_err(|error| error.to_string())?;
    if meta.is_dir() {
        std::fs::create_dir_all(to).map_err(|error| error.to_string())?;
        let mut done = 0;
        for entry in std::fs::read_dir(from).map_err(|error| error.to_string())? {
            let entry = entry.map_err(|error| error.to_string())?;
            let kind = entry.file_type().map_err(|error| error.to_string())?;
            // A link to a folder is left out, as in a folder transfer (D56).
            if kind.is_symlink() && entry.path().is_dir() {
                continue;
            }
            done += local_copy(&entry.path(), &to.join(entry.file_name()), progress)?;
            progress(done);
        }
        Ok(done)
    } else {
        let bytes = std::fs::copy(from, to).map_err(|error| error.to_string())?;
        progress(bytes);
        Ok(bytes)
    }
}

// -- Rendering ----------------------------------------------------------------------

fn direction_text(direction: Direction) -> &'static str {
    match direction {
        Direction::Upload => strings::UPLOAD,
        Direction::Download => strings::DOWNLOAD,
        Direction::RemoteToRemote => strings::BETWEEN_HOSTS,
    }
}

fn state_text(state: &State) -> SharedString {
    match state {
        State::Queued => strings::QUEUED.into(),
        State::Running => SharedString::default(),
        State::Conflict => strings::CONFLICT_TITLE.into(),
        State::Paused => strings::PAUSED.into(),
        State::Failed(reason) => reason.clone().into(),
        State::Done => strings::DONE.into(),
        State::Skipped => strings::CONFLICT_SKIP.into(),
        State::Cancelled => strings::CANCELLED.into(),
    }
}

/// "1.5 MB/s · 12 s left".
fn speed_text(transfer: &shellpp_ssh::transfer_queue::Transfer) -> String {
    let Some(speed) = transfer.speed() else {
        return String::new();
    };
    let speed = format!("{}/s", listing::human_size(speed as u64));
    match transfer.time_left() {
        Some(left) => format!("{speed} · {} s", left.as_secs()),
        None => speed,
    }
}

impl Render for Transfers {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let active = self.active();
        let collapsed = self.collapsed;
        let header = h_flex()
            .id("sftp-queue-header")
            .test_support()
            .w_full()
            .h(px(32.))
            .px(metrics::SPACE_3)
            .gap(metrics::SPACE_2)
            .child(
                ControlButton::new("sftp-queue-toggle", ControlKind::Ghost)
                    .icon(if collapsed {
                        IconName::ChevronUp
                    } else {
                        IconName::ChevronDown
                    })
                    .height(px(28.))
                    .tooltip(strings::QUEUE_TITLE)
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.set_collapsed(!collapsed, cx)
                    })),
            )
            .child(
                div()
                    .text_size(metrics::FONT_SIZE_BODY)
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(theme.foreground)
                    .child(strings::QUEUE_TITLE),
            )
            .child(
                div()
                    .id("sftp-queue-active")
                    .test_support()
                    .flex_1()
                    .text_size(metrics::FONT_SIZE_DENSE)
                    .text_color(theme.muted_foreground)
                    .child(strings::active_count(active)),
            )
            .child(
                ControlButton::new("sftp-queue-clear", ControlKind::Ghost)
                    .label(strings::QUEUE_CLEAR)
                    .height(px(28.))
                    .disabled(self.queue.transfers().iter().all(|t| t.state.is_open()))
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.clear_finished(cx))),
            );
        let rows = self.queue.transfers().iter().map(|transfer| {
            let id = transfer.id;
            let state = transfer.state.clone();
            let skipped = self.jobs.get(&id).map_or(0, |job| job.skipped);
            let fraction = if state == State::Done {
                1.
            } else {
                transfer.fraction().unwrap_or(0.) as f32
            };
            let local_copy = self.jobs.get(&id).is_some_and(|job| {
                matches!((&job.from, &job.to), (Place::Local(_), Place::Local(_)))
            });
            let row_id = SharedString::from(format!("sftp-transfer-{}", transfer.name));
            let button = |suffix: &str, icon: Lucide, tooltip: &'static str| {
                ControlButton::new(
                    SharedString::from(format!("sftp-transfer-{}-{suffix}", transfer.name)),
                    ControlKind::Ghost,
                )
                .icon(icon)
                .height(px(28.))
                .tooltip(tooltip)
            };
            h_flex()
                .id(row_id)
                .test_support()
                .w_full()
                .h(ROW_HEIGHT)
                .px(metrics::SPACE_3)
                .gap(metrics::SPACE_3)
                .child(
                    Icon::new(match transfer.direction {
                        Direction::Upload => Lucide::Upload,
                        Direction::Download => Lucide::Download,
                        Direction::RemoteToRemote => Lucide::ArrowLeftRight,
                    })
                    .with_size(ICON_SIZE)
                    .text_color(theme.muted_foreground),
                )
                .child(
                    div()
                        .w(px(220.))
                        .flex_none()
                        .truncate()
                        .text_size(metrics::FONT_SIZE_BODY)
                        .text_color(theme.foreground)
                        .child(transfer.name.clone()),
                )
                .child(
                    div()
                        .w(px(90.))
                        .flex_none()
                        .text_size(metrics::FONT_SIZE_CAPTION)
                        .text_color(theme.muted_foreground)
                        .child(if local_copy {
                            strings::COPY_LOCAL
                        } else {
                            direction_text(transfer.direction)
                        }),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w(px(80.))
                        .h(px(6.))
                        .rounded(px(3.))
                        .bg(theme.muted)
                        .child(
                            div()
                                .h_full()
                                .rounded(px(3.))
                                .w(gpui::relative(fraction))
                                .bg(match state {
                                    State::Failed(_) => theme.danger,
                                    _ => theme.primary,
                                }),
                        ),
                )
                .child(
                    div()
                        .w(px(200.))
                        .flex_none()
                        .truncate()
                        .text_size(metrics::FONT_SIZE_CAPTION)
                        .text_color(match state {
                            State::Failed(_) => theme.danger,
                            _ => theme.muted_foreground,
                        })
                        .child(if state == State::Running {
                            SharedString::from(speed_text(transfer))
                        } else if state == State::Done && skipped > 0 {
                            strings::skipped(skipped).into()
                        } else {
                            state_text(&state)
                        }),
                )
                .child(
                    h_flex()
                        .flex_none()
                        .w(px(64.))
                        .justify_end()
                        .gap(px(2.))
                        .when(matches!(state, State::Running | State::Queued), |this| {
                            this.child(button("pause", Lucide::Pause, strings::PAUSE).on_click(
                                cx.listener(move |this, _: &ClickEvent, _, cx| this.pause(id, cx)),
                            ))
                        })
                        .when(state == State::Paused, |this| {
                            this.child(button("resume", Lucide::Play, strings::RESUME).on_click(
                                cx.listener(move |this, _: &ClickEvent, window, cx| {
                                    this.resume(id, window, cx)
                                }),
                            ))
                        })
                        .when(matches!(state, State::Failed(_)), |this| {
                            this.child(button("retry", Lucide::RotateCw, strings::RETRY).on_click(
                                cx.listener(move |this, _: &ClickEvent, window, cx| {
                                    this.resume(id, window, cx)
                                }),
                            ))
                        })
                        .when(state.is_open(), |this| {
                            this.child(button("cancel", Lucide::X, strings::CANCEL).on_click(
                                cx.listener(move |this, _: &ClickEvent, _, cx| this.cancel(id, cx)),
                            ))
                        }),
                )
        });
        let count = self.queue.transfers().len() as f32;
        v_flex()
            .id("sftp-queue")
            .test_support()
            .w_full()
            .flex_none()
            .border_t_1()
            .border_color(theme.border)
            .bg(theme.background)
            .child(header)
            .when(!collapsed, |this| {
                this.child(
                    v_flex()
                        .id("sftp-queue-rows")
                        .test_support()
                        .h(ROW_HEIGHT * count.clamp(1., VISIBLE_ROWS))
                        .overflow_y_scroll()
                        .children(rows),
                )
            })
    }
}
