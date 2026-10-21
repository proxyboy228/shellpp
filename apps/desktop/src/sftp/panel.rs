//! One file panel of the SFTP tab (UI 5.2, 5.3, 5.5; stage 4): the empty
//! state with its host selection, the local file system, or a saved host's
//! files over an SFTP session of the session layer
//! (`SessionManager::open_sftp`: the same route, prompts and host key
//! dialogs as a terminal tab, shown inside the panel).
//!
//! The rows come from [`super::listing`] (sorted, filtered, hidden files,
//! "..", breadcrumbs, back and forward); the panel owns the selection
//! (click, Ctrl/Cmd+click, Shift+click, by name) and the file operations of
//! its source: a remote request runs on the session runtime
//! ([`SessionHandle::spawn`]), a local one on a background thread. A result
//! that arrives after the panel moved on (another folder, another source) is
//! dropped by its generation. Nothing here logs file names or paths.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, UNIX_EPOCH};

use gpui::prelude::*;
use gpui::{
    AnyElement, App, ClickEvent, Context, Entity, EventEmitter, FocusHandle, Focusable, FontWeight,
    IntoElement, Pixels, SharedString, Size, Subscription, Task, Window, div, px, size,
};
use gpui_kit::TestSupportExt as _;
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::menu::{ContextMenuExt as _, DropdownMenu as _, PopupMenu};
use gpui_kit::component::scroll::Scrollbar;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Sizable as _, VirtualListScrollHandle,
    h_flex, v_flex, v_virtual_list,
};
use shellpp_core::service::dto::{RecordRef, SessionTarget};
use shellpp_ssh::Sftp;

use super::listing::{self, Column, Entry, History, Kind, PathStyle, Sort};
use super::local;
use super::strings;
use crate::hosts::controls::{ControlButton, ControlKind, ICON_SIZE};
use crate::panels::{menus, widgets};
use crate::session_ui::host_key::{self, AnswersHostKey};
use crate::session_ui::model::{HostKeyAsk, HostKeyReply, PromptReply as FormReply};
use crate::session_ui::prompt::PromptForm;
use crate::session_ui::{bridge, model};
use crate::sessions::{
    self, HostKeyChoice, PromptKind, PromptReply, SessionHandle, SessionState, SessionView,
};
use crate::state::AppState;
use crate::state::selection::SelectMode;
use crate::theme::metrics;

/// Height of a row (a remote file's permissions sit under its name).
pub const ROW_HEIGHT: Pixels = px(40.);
/// Widths of the fixed columns.
const MODIFIED_COLUMN: Pixels = px(150.);
const SIZE_COLUMN: Pixels = px(80.);
const KIND_COLUMN: Pixels = px(80.);
/// The filter field of the header.
const FILTER_WIDTH: Pixels = px(160.);
/// The host selection list.
const PICKER_WIDTH: Pixels = px(360.);
const PICKER_HEIGHT: Pixels = px(320.);

/// Which panel: the ids of its elements start with its side.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
}

impl Side {
    fn name(self) -> &'static str {
        match self {
            Side::Left => "left",
            Side::Right => "right",
        }
    }
}

/// What a panel asks of the SFTP tab.
#[derive(Clone, Debug)]
pub enum PanelEvent {
    /// The folder, the source or the selection changed.
    Changed,
    /// "Copy to target directory": these entries of the current folder.
    Copy(Vec<Entry>),
    /// A double click on a file, or Open (UI 5.4).
    Open(Entry),
    /// Open with…: the program is chosen first.
    OpenWith(Entry),
}

/// A connected (or connecting) host.
pub struct Remote {
    pub host: RecordRef,
    pub title: SharedString,
    handle: SessionHandle,
    view: SessionView,
    ask: Option<Ask>,
    _events: Task<()>,
}

impl std::fmt::Debug for Remote {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Remote")
            .field("host", &self.host)
            .field("state", &self.view.state)
            .finish_non_exhaustive()
    }
}

impl std::fmt::Debug for Source {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Source::Empty => f.write_str("Empty"),
            Source::Local => f.write_str("Local"),
            Source::Remote(remote) => remote.fmt(f),
        }
    }
}

impl std::fmt::Debug for FilePanel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FilePanel")
            .field("side", &self.side)
            .field("source", &self.source)
            .field("rows", &self.rows.len())
            .field("load", &self.load)
            .finish_non_exhaustive()
    }
}

/// A remote panel that goes (another source, a closed tab) ends its
/// session.
impl Drop for Remote {
    fn drop(&mut self) {
        self.handle.close();
    }
}

/// The question of a connecting session, inside the panel.
enum Ask {
    /// A text prompt; its form is made at the next frame (it needs the
    /// window).
    Prompt {
        id: u64,
        prompt: model::Prompt,
        form: Option<Entity<PromptForm>>,
    },
    HostKey {
        id: u64,
        ask: HostKeyAsk,
        focus: FocusHandle,
    },
}

/// What the panel shows.
pub enum Source {
    /// The empty state with "Choose host…" (UI 5.2).
    Empty,
    Local,
    Remote(Box<Remote>),
}

/// The state of the folder listing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Load {
    Idle,
    Loading,
    Failed(SharedString),
}

/// The host selection of the empty panel.
struct Picker {
    search: Entity<InputState>,
    _subscription: Subscription,
}

/// One file panel.
pub struct FilePanel {
    side: Side,
    state: Entity<AppState>,
    source: Source,
    style: PathStyle,
    history: History,
    /// The folder's entries as listed.
    entries: Vec<Entry>,
    /// What the table shows: sorted, filtered, hidden files left out.
    rows: Vec<Entry>,
    sort: Sort,
    filter: Entity<InputState>,
    show_hidden: bool,
    selected: BTreeSet<String>,
    anchor: Option<String>,
    load: Load,
    /// Counts the listings: a late result of an older one is dropped.
    generation: u64,
    listing: Option<Task<()>>,
    scroll: VirtualListScrollHandle,
    row_sizes: Rc<Vec<Size<Pixels>>>,
    focus: FocusHandle,
    picker: Option<Picker>,
    /// The drive selection is open (Windows).
    drives_open: bool,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<PanelEvent> for FilePanel {}

impl Focusable for FilePanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl AnswersHostKey for FilePanel {
    fn answer_host_key(&mut self, reply: HostKeyReply, cx: &mut Context<Self>) {
        let Source::Remote(remote) = &mut self.source else {
            return;
        };
        let Some(Ask::HostKey { id, .. }) = remote.ask.take() else {
            return;
        };
        let choice = match reply {
            HostKeyReply::AcceptAndSave => HostKeyChoice::AcceptAndSave,
            HostKeyReply::ConnectOnce => HostKeyChoice::ConnectOnce,
            HostKeyReply::Replace => HostKeyChoice::Replace,
            HostKeyReply::Cancel => HostKeyChoice::Cancel,
        };
        if let Err(error) = remote.handle.reply(id, PromptReply::HostKey(choice)) {
            tracing::debug!(%error, "host key answer not taken");
        }
        cx.notify();
    }
}

/// The local folder a local panel opens first: the user's home folder,
/// else the first drive.
pub fn local_start() -> PathBuf {
    let home = if cfg!(windows) {
        std::env::var_os("USERPROFILE")
    } else {
        std::env::var_os("HOME")
    };
    home.map(PathBuf::from)
        .filter(|path| path.is_dir())
        .or_else(|| local::drives().into_iter().next())
        .unwrap_or_else(|| PathBuf::from("/"))
}

/// The path style of the local file system.
fn local_style() -> PathStyle {
    if cfg!(windows) {
        PathStyle::Windows
    } else {
        PathStyle::Posix
    }
}

/// "2026-10-21 14:02 UTC" for a modification time (the format of Known
/// Hosts; D65).
pub fn date_text(modified: Option<u64>) -> String {
    match modified {
        Some(seconds) => crate::known_hosts::date_text(
            &shellpp_core::model::time::format_timestamp(UNIX_EPOCH + Duration::from_secs(seconds)),
        ),
        None => "—".to_owned(),
    }
}

/// Whether `name` can be a new name in a folder (no separator, not `.` or
/// `..`, not blank).
pub fn valid_name(name: &str) -> bool {
    let name = name.trim();
    !name.is_empty() && name != "." && name != ".." && !name.contains(['/', '\\', '\0'])
}

impl FilePanel {
    pub fn new(
        side: Side,
        state: Entity<AppState>,
        local: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let filter = cx.new(|cx| InputState::new(window, cx).placeholder(strings::FILTER));
        let subscriptions =
            vec![
                cx.subscribe(&filter, |this: &mut Self, _, event: &InputEvent, cx| {
                    if matches!(event, InputEvent::Change) {
                        this.rebuild(cx);
                    }
                }),
            ];
        let mut panel = Self {
            side,
            state,
            source: Source::Empty,
            style: PathStyle::Posix,
            history: History::new(""),
            entries: Vec::new(),
            rows: Vec::new(),
            sort: Sort::default(),
            filter,
            show_hidden: false,
            selected: BTreeSet::new(),
            anchor: None,
            load: Load::Idle,
            generation: 0,
            listing: None,
            scroll: VirtualListScrollHandle::new(),
            row_sizes: Rc::new(Vec::new()),
            focus: cx.focus_handle(),
            picker: None,
            drives_open: false,
            _subscriptions: subscriptions,
        };
        if local {
            panel.open_local(local_start(), cx);
        }
        panel
    }

    // -- Readers ---------------------------------------------------------------

    pub fn side(&self) -> Side {
        self.side
    }

    pub fn source(&self) -> &Source {
        &self.source
    }

    pub fn is_local(&self) -> bool {
        matches!(self.source, Source::Local)
    }

    /// The SFTP channel of a connected remote panel.
    pub fn sftp(&self) -> Option<Arc<Sftp>> {
        match &self.source {
            Source::Remote(remote) if remote.view.state == SessionState::Connected => {
                remote.handle.sftp()
            }
            _ => None,
        }
    }

    /// The session of a remote panel (its runtime runs the transfers).
    pub fn handle(&self) -> Option<&SessionHandle> {
        match &self.source {
            Source::Remote(remote) => Some(&remote.handle),
            _ => None,
        }
    }

    /// Whether the panel can list files now (local, or a connected host).
    pub fn is_ready(&self) -> bool {
        match &self.source {
            Source::Local => true,
            Source::Remote(remote) => remote.view.state == SessionState::Connected,
            Source::Empty => false,
        }
    }

    /// The current folder ("" until a host's home folder is known).
    pub fn path(&self) -> &str {
        self.history.current()
    }

    pub fn style(&self) -> PathStyle {
        self.style
    }

    pub fn rows(&self) -> &[Entry] {
        &self.rows
    }

    pub fn load(&self) -> &Load {
        &self.load
    }

    pub fn sort(&self) -> Sort {
        self.sort
    }

    pub fn show_hidden(&self) -> bool {
        self.show_hidden
    }

    /// The selected entries, in the order of the table.
    pub fn selection(&self) -> Vec<Entry> {
        self.rows
            .iter()
            .filter(|entry| self.selected.contains(&entry.name))
            .cloned()
            .collect()
    }

    /// The path of `name` in the current folder.
    pub fn child_path(&self, name: &str) -> String {
        join(self.path(), name, self.style)
    }

    /// `sftp.confirm_delete` (UI 12.6, on by default): Delete asks first.
    pub fn confirm_delete(&self, cx: &App) -> bool {
        self.state
            .read(cx)
            .core()
            .and_then(|core| core.settings_get().ok())
            .and_then(|values| values.get("sftp.confirm_delete").and_then(|v| v.as_bool()))
            .unwrap_or(true)
    }

    /// The title of the source (the header).
    pub fn title(&self) -> SharedString {
        match &self.source {
            Source::Empty => SharedString::default(),
            Source::Local => strings::LOCAL.into(),
            Source::Remote(remote) => remote.title.clone(),
        }
    }

    fn id(&self, part: &str) -> SharedString {
        format!("sftp-{}-{part}", self.side.name()).into()
    }

    // -- Sources -----------------------------------------------------------------

    /// Shows the local file system at `start`.
    pub fn open_local(&mut self, start: PathBuf, cx: &mut Context<Self>) {
        self.close_remote();
        self.source = Source::Local;
        self.picker = None;
        self.style = local_style();
        let start = start.to_string_lossy().into_owned();
        self.history = History::new(&start);
        self.list(cx);
    }

    /// Connects `host` (a saved host) and shows its home folder once the
    /// session is connected.
    pub fn connect(
        &mut self,
        host: RecordRef,
        title: SharedString,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(manager) = sessions::ui::manager(cx) else {
            tracing::warn!("the session layer is not installed");
            return;
        };
        let opened = manager.open_sftp(SessionTarget::Host {
            vault_id: host.vault_id,
            record_id: host.record_id,
        });
        let opened = match opened {
            Ok(opened) => opened,
            Err(error) => {
                crate::shell::show_toast(bridge::failure_of(error).text(), window, cx);
                return;
            }
        };
        self.close_remote();
        self.picker = None;
        let view = opened.handle.view();
        let events =
            sessions::ui::forward_events(opened.events, cx, |this: &mut Self, view, cx| {
                this.apply_view(view, cx)
            });
        self.source = Source::Remote(Box::new(Remote {
            host,
            title,
            handle: opened.handle,
            view,
            ask: None,
            _events: events,
        }));
        self.style = PathStyle::Posix;
        self.history = History::new("");
        self.entries.clear();
        self.selected.clear();
        self.load = Load::Idle;
        self.rebuild(cx);
        cx.emit(PanelEvent::Changed);
    }

    /// Close (UI 5.5): the connection ends and the panel is empty again.
    pub fn close(&mut self, cx: &mut Context<Self>) {
        self.close_remote();
        self.source = Source::Empty;
        self.picker = None;
        self.history = History::new("");
        self.entries.clear();
        self.selected.clear();
        self.load = Load::Idle;
        self.generation += 1;
        self.listing = None;
        self.rebuild(cx);
        cx.emit(PanelEvent::Changed);
    }

    fn close_remote(&mut self) {
        if let Source::Remote(remote) = &self.source {
            remote.handle.close();
        }
    }

    /// Reconnect or Retry of a dropped or failed host.
    pub fn reconnect(&mut self, cx: &mut Context<Self>) {
        if let Source::Remote(remote) = &self.source {
            remote.handle.reconnect();
        }
        cx.notify();
    }

    /// A view of the panel's SFTP session.
    fn apply_view(&mut self, view: SessionView, cx: &mut Context<Self>) {
        let Source::Remote(remote) = &mut self.source else {
            return;
        };
        if view.state == SessionState::Closed {
            self.source = Source::Empty;
            self.history = History::new("");
            self.entries.clear();
            self.selected.clear();
            self.load = Load::Idle;
            self.rebuild(cx);
            cx.emit(PanelEvent::Changed);
            return;
        }
        let shown = remote.ask.as_ref().map(|ask| match ask {
            Ask::Prompt { id, .. } | Ask::HostKey { id, .. } => *id,
        });
        match &view.prompt {
            None => remote.ask = None,
            Some(prompt) if Some(prompt.id) == shown => {}
            Some(prompt) => {
                remote.ask = Some(match &prompt.kind {
                    PromptKind::HostKeyUnknown { key } => Ask::HostKey {
                        id: prompt.id,
                        ask: HostKeyAsk::Unknown(bridge::host_key_of(key)),
                        focus: cx.focus_handle(),
                    },
                    PromptKind::HostKeyChanged {
                        key,
                        old_key_type,
                        old_fingerprint,
                    } => Ask::HostKey {
                        id: prompt.id,
                        ask: HostKeyAsk::Changed {
                            key: bridge::host_key_of(key),
                            old_key_type: old_key_type.clone(),
                            old_fingerprint: old_fingerprint.clone(),
                        },
                        focus: cx.focus_handle(),
                    },
                    kind => Ask::Prompt {
                        id: prompt.id,
                        prompt: bridge::prompt_of(kind),
                        form: None,
                    },
                });
            }
        }
        let connected = view.state == SessionState::Connected;
        let was_connected = remote.view.state == SessionState::Connected;
        remote.view = view;
        if connected && !was_connected {
            self.open_home(cx);
        } else if !connected && was_connected {
            self.listing = None;
            self.load = Load::Idle;
        }
        cx.notify();
        cx.emit(PanelEvent::Changed);
    }

    /// A connected host: its home folder (or the folder it showed before a
    /// reconnect).
    fn open_home(&mut self, cx: &mut Context<Self>) {
        if !self.path().is_empty() {
            self.list(cx);
            return;
        }
        let Some(sftp) = self.sftp() else {
            return;
        };
        let Some(handle) = self.handle().cloned() else {
            return;
        };
        self.generation += 1;
        let generation = self.generation;
        self.load = Load::Loading;
        let work = handle.spawn(async move { sftp.canonicalize(".").await });
        self.listing = Some(cx.spawn(async move |this, cx| {
            let home = work.await;
            let _ = this.update(cx, |this, cx| {
                if generation != this.generation {
                    return;
                }
                match home {
                    Ok(Ok(home)) => {
                        this.history = History::new(&home);
                        this.list(cx);
                    }
                    Ok(Err(error)) => this.failed(error.to_string(), cx),
                    Err(_) => this.failed(strings::LIST_FAILED.to_owned(), cx),
                }
            });
        }));
        cx.notify();
    }

    // -- Listing -----------------------------------------------------------------

    /// Lists the current folder again.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        self.list(cx);
    }

    fn list(&mut self, cx: &mut Context<Self>) {
        self.generation += 1;
        let generation = self.generation;
        let path = self.path().to_owned();
        self.load = Load::Loading;
        match &self.source {
            Source::Empty => {
                self.load = Load::Idle;
            }
            Source::Local => {
                let work = cx.background_spawn(async move {
                    local::list(Path::new(&path)).map_err(|error| error.to_string())
                });
                self.listing = Some(cx.spawn(async move |this, cx| {
                    let listed = work.await;
                    let _ = this.update(cx, |this, cx| this.listed(generation, listed, cx));
                }));
            }
            Source::Remote(remote) => {
                let Some(sftp) = remote.handle.sftp() else {
                    self.load = Load::Idle;
                    cx.notify();
                    return;
                };
                let work = remote.handle.spawn(async move {
                    sftp.list(&path)
                        .await
                        .map(|entries| {
                            entries
                                .iter()
                                // A name a server should never send (a
                                // separator, "." or ".."): it would lead
                                // every operation outside this folder.
                                .filter(|entry| {
                                    valid_name(&entry.name)
                                        && entry.name == entry.name.trim()
                                        && !entry.name.chars().any(char::is_control)
                                })
                                .map(local::from_remote)
                                .collect::<Vec<_>>()
                        })
                        .map_err(|error| error.to_string())
                });
                self.listing = Some(cx.spawn(async move |this, cx| {
                    let listed = work
                        .await
                        .unwrap_or_else(|_| Err(strings::LIST_FAILED.to_owned()));
                    let _ = this.update(cx, |this, cx| this.listed(generation, listed, cx));
                }));
            }
        }
        cx.notify();
    }

    fn listed(
        &mut self,
        generation: u64,
        listed: Result<Vec<Entry>, String>,
        cx: &mut Context<Self>,
    ) {
        if generation != self.generation {
            return;
        }
        self.listing = None;
        match listed {
            Ok(entries) => {
                self.entries = entries;
                self.load = Load::Idle;
                let names: BTreeSet<String> = self
                    .entries
                    .iter()
                    .map(|entry| entry.name.clone())
                    .collect();
                self.selected.retain(|name| names.contains(name));
            }
            Err(reason) => self.failed(reason, cx),
        }
        self.rebuild(cx);
        cx.emit(PanelEvent::Changed);
    }

    fn failed(&mut self, reason: String, cx: &mut Context<Self>) {
        self.entries.clear();
        self.selected.clear();
        self.load = Load::Failed(strings::failed(strings::LIST_FAILED, &reason).into());
        self.rebuild(cx);
    }

    /// The table's rows from the entries: filter, hidden files, sort.
    fn rebuild(&mut self, cx: &mut Context<Self>) {
        let filter = self.filter.read(cx).value().to_string();
        let shown: Vec<Entry> = listing::visible(&self.entries, &filter, self.show_hidden)
            .into_iter()
            .cloned()
            .collect();
        self.rows = listing::sorted(&shown, self.sort);
        let parent = usize::from(listing::parent(self.path(), self.style).is_some());
        self.row_sizes = Rc::new(vec![size(px(1.), ROW_HEIGHT); self.rows.len() + parent]);
        cx.notify();
    }

    // -- Navigation --------------------------------------------------------------

    /// Opens `path` (a folder of this source).
    pub fn go(&mut self, path: &str, cx: &mut Context<Self>) {
        if !self.is_ready() {
            return;
        }
        self.history.go(path);
        self.selected.clear();
        self.anchor = None;
        self.filter_reset(cx);
        self.list(cx);
    }

    pub fn back(&mut self, cx: &mut Context<Self>) {
        if self.history.back().is_some() {
            self.selected.clear();
            self.list(cx);
        }
    }

    pub fn forward(&mut self, cx: &mut Context<Self>) {
        if self.history.forward().is_some() {
            self.selected.clear();
            self.list(cx);
        }
    }

    /// The ".." row.
    pub fn up(&mut self, cx: &mut Context<Self>) {
        if let Some(parent) = listing::parent(self.path(), self.style) {
            self.go(&parent, cx);
        }
    }

    fn filter_reset(&mut self, cx: &mut Context<Self>) {
        let _ = cx;
    }

    /// A double click or Open on `entry`: a folder opens, a file is opened
    /// by the tab (UI 5.4).
    pub fn open_entry(&mut self, entry: &Entry, cx: &mut Context<Self>) {
        if entry.is_folder_like() {
            let path = self.child_path(&entry.name);
            self.go(&path, cx);
        } else {
            cx.emit(PanelEvent::Open(entry.clone()));
        }
    }

    pub fn set_sort(&mut self, column: Column, cx: &mut Context<Self>) {
        self.sort = self.sort.clicked(column);
        self.rebuild(cx);
    }

    pub fn set_show_hidden(&mut self, show: bool, cx: &mut Context<Self>) {
        self.show_hidden = show;
        let names: BTreeSet<String> = listing::visible(&self.entries, "", show)
            .into_iter()
            .map(|entry| entry.name.clone())
            .collect();
        self.selected.retain(|name| names.contains(name));
        self.rebuild(cx);
        cx.emit(PanelEvent::Changed);
    }

    // -- Selection ----------------------------------------------------------------

    /// A click on the row of `name`.
    pub fn click(&mut self, name: &str, mode: SelectMode, cx: &mut Context<Self>) {
        match mode {
            SelectMode::Replace => {
                self.selected.clear();
                self.selected.insert(name.to_owned());
                self.anchor = Some(name.to_owned());
            }
            SelectMode::Toggle => {
                if !self.selected.remove(name) {
                    self.selected.insert(name.to_owned());
                }
                self.anchor = Some(name.to_owned());
            }
            SelectMode::Range => {
                let position = |wanted: &str| self.rows.iter().position(|row| row.name == wanted);
                match (self.anchor.as_deref().and_then(position), position(name)) {
                    (Some(from), Some(to)) => {
                        let (from, to) = (from.min(to), from.max(to));
                        self.selected = self.rows[from..=to]
                            .iter()
                            .map(|row| row.name.clone())
                            .collect();
                    }
                    _ => {
                        self.selected.clear();
                        self.selected.insert(name.to_owned());
                        self.anchor = Some(name.to_owned());
                    }
                }
            }
        }
        cx.notify();
        cx.emit(PanelEvent::Changed);
    }

    pub fn select_all(&mut self, cx: &mut Context<Self>) {
        self.selected = self.rows.iter().map(|row| row.name.clone()).collect();
        cx.notify();
        cx.emit(PanelEvent::Changed);
    }

    pub fn is_selected(&self, name: &str) -> bool {
        self.selected.contains(name)
    }

    // -- Host selection (UI 5.2) ------------------------------------------------

    pub fn open_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder(strings::PICKER_SEARCH));
        let subscription = cx.subscribe(&search, |_: &mut Self, _, _: &InputEvent, cx| cx.notify());
        search.update(cx, |input, cx| input.focus(window, cx));
        self.picker = Some(Picker {
            search,
            _subscription: subscription,
        });
        cx.notify();
    }

    pub fn close_picker(&mut self, cx: &mut Context<Self>) {
        self.picker = None;
        cx.notify();
    }

    pub fn is_picking(&self) -> bool {
        self.picker.is_some()
    }

    /// The saved hosts the picker lists for its search: (host, title).
    pub fn picker_hosts(&self, cx: &App) -> Vec<(RecordRef, SharedString)> {
        let text = self
            .picker
            .as_ref()
            .map(|picker| picker.search.read(cx).value().trim().to_lowercase())
            .unwrap_or_default();
        self.state
            .read(cx)
            .hosts()
            .iter()
            .filter(|host| {
                text.is_empty()
                    || host.address.to_lowercase().contains(&text)
                    || host
                        .label
                        .as_deref()
                        .is_some_and(|label| label.to_lowercase().contains(&text))
            })
            .map(|host| {
                (
                    RecordRef {
                        vault_id: host.vault_id,
                        record_id: host.record_id,
                    },
                    SharedString::from(host.label.clone().unwrap_or_else(|| host.address.clone())),
                )
            })
            .collect()
    }

    // -- File operations (UI 5.4, 5.5) ------------------------------------------

    /// Runs `remote` on the session runtime or `local` on a background
    /// thread, then lists the folder again; a failure shows a toast.
    fn operate(
        &mut self,
        failure: &'static str,
        local: impl FnOnce() -> Result<(), String> + Send + 'static,
        remote: impl FnOnce(Arc<Sftp>) -> futures::future::BoxFuture<'static, Result<(), String>>
        + Send
        + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let task: Task<Result<(), String>> = match &self.source {
            Source::Local => cx.background_spawn(async move { local() }),
            Source::Remote(remote_source) => {
                let Some(sftp) = remote_source.handle.sftp() else {
                    return;
                };
                let work = remote_source.handle.spawn(remote(sftp));
                cx.background_spawn(async move {
                    work.await
                        .unwrap_or_else(|_| Err(strings::LIST_FAILED.to_owned()))
                })
            }
            Source::Empty => return,
        };
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |this, window, cx| {
                if let Err(reason) = result {
                    crate::shell::show_toast(strings::failed(failure, &reason), window, cx);
                }
                this.refresh(cx);
            });
        })
        .detach();
    }

    /// New Folder: `name` in the current folder.
    pub fn new_folder(&mut self, name: &str, window: &mut Window, cx: &mut Context<Self>) {
        if !valid_name(name) {
            return;
        }
        let path = self.child_path(name.trim());
        let remote_path = path.clone();
        self.operate(
            strings::CREATE_FAILED,
            move || std::fs::create_dir(&path).map_err(|error| error.to_string()),
            move |sftp| {
                Box::pin(async move {
                    sftp.create_dir(&remote_path)
                        .await
                        .map_err(|error| error.to_string())
                })
            },
            window,
            cx,
        );
    }

    /// Rename: `from` to `to` in the current folder.
    pub fn rename(&mut self, from: &str, to: &str, window: &mut Window, cx: &mut Context<Self>) {
        if !valid_name(to) || from == to.trim() {
            return;
        }
        let old = self.child_path(from);
        let new = self.child_path(to.trim());
        let (remote_old, remote_new) = (old.clone(), new.clone());
        if self.entries.iter().any(|entry| entry.name == to.trim()) {
            crate::shell::show_toast(
                strings::failed(strings::RENAME_FAILED, strings::NAME_TAKEN),
                window,
                cx,
            );
            return;
        }
        self.selected.remove(from);
        self.operate(
            strings::RENAME_FAILED,
            move || {
                // Not over an item that appeared meanwhile.
                if std::fs::symlink_metadata(&new).is_ok() {
                    return Err(strings::NAME_TAKEN.to_owned());
                }
                std::fs::rename(&old, &new).map_err(|error| error.to_string())
            },
            move |sftp| {
                Box::pin(async move {
                    sftp.rename(&remote_old, &remote_new)
                        .await
                        .map_err(|error| error.to_string())
                })
            },
            window,
            cx,
        );
    }

    /// Delete: the entries, folders with everything inside.
    pub fn delete(&mut self, entries: Vec<Entry>, window: &mut Window, cx: &mut Context<Self>) {
        let targets: Vec<(String, bool)> = entries
            .iter()
            .map(|entry| {
                (
                    self.child_path(&entry.name),
                    matches!(entry.kind, Kind::Folder),
                )
            })
            .collect();
        let remote_targets = targets.clone();
        for entry in &entries {
            self.selected.remove(&entry.name);
        }
        self.operate(
            strings::DELETE_FAILED,
            move || {
                for (path, folder) in &targets {
                    // A link (to a folder too) is removed itself, never what
                    // it points to.
                    let result = if *folder {
                        std::fs::remove_dir_all(path)
                    } else {
                        std::fs::remove_file(path).or_else(|error| {
                            // A Windows link to a folder is a directory entry.
                            std::fs::remove_dir(path).map_err(|_| error)
                        })
                    };
                    result.map_err(|error| error.to_string())?;
                }
                Ok(())
            },
            move |sftp| {
                Box::pin(async move {
                    for (path, folder) in &remote_targets {
                        let result = if *folder {
                            sftp.remove_tree(path).await.map(drop)
                        } else {
                            sftp.remove_file(path).await
                        };
                        result.map_err(|error| error.to_string())?;
                    }
                    Ok(())
                })
            },
            window,
            cx,
        );
    }

    /// Edit Permissions (remote): `mode` on the entries, into folders with
    /// `recursive`.
    pub fn set_permissions(
        &mut self,
        entries: Vec<Entry>,
        mode: u32,
        recursive: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let targets: Vec<(String, bool)> = entries
            .iter()
            .map(|entry| {
                (
                    self.child_path(&entry.name),
                    recursive && matches!(entry.kind, Kind::Folder),
                )
            })
            .collect();
        let local_targets = targets.clone();
        self.operate(
            strings::PERMISSIONS_FAILED,
            move || set_local_permissions(&local_targets, mode),
            move |sftp| {
                Box::pin(async move {
                    for (path, tree) in &targets {
                        let result = if *tree {
                            sftp.set_permissions_tree(path, mode).await.map(drop)
                        } else {
                            sftp.set_permissions(path, mode).await
                        };
                        result.map_err(|error| error.to_string())?;
                    }
                    Ok(())
                })
            },
            window,
            cx,
        );
    }
}

/// Edit Permissions in a local panel (Unix; the item is hidden on
/// Windows): `mode` on each target, into folders when it says so.
fn set_local_permissions(targets: &[(String, bool)], mode: u32) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fn apply(path: &Path, mode: u32, tree: bool) -> std::io::Result<()> {
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))?;
            if tree {
                for entry in std::fs::read_dir(path)? {
                    let entry = entry?;
                    let kind = entry.file_type()?;
                    // Links are never followed.
                    if !kind.is_symlink() {
                        apply(&entry.path(), mode, kind.is_dir())?;
                    }
                }
            }
            Ok(())
        }
        for (path, tree) in targets {
            apply(Path::new(path), mode, *tree).map_err(|error| error.to_string())?;
        }
        Ok(())
    }
    #[cfg(not(unix))]
    {
        let _ = (targets, mode);
        Ok(())
    }
}

/// `dir` and `name` joined in `style`.
pub fn join(dir: &str, name: &str, style: PathStyle) -> String {
    match style {
        PathStyle::Posix => shellpp_ssh::sftp::join(dir, name),
        PathStyle::Windows => Path::new(dir).join(name).to_string_lossy().into_owned(),
    }
}

// -- Rendering ----------------------------------------------------------------------

impl FilePanel {
    fn render_empty(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        if self.picker.is_some() {
            return self.render_picker(window, cx);
        }
        v_flex()
            .id(self.id("empty"))
            .test_support()
            .size_full()
            .items_center()
            .justify_center()
            .gap(metrics::SPACE_3)
            .px(metrics::SPACE_5)
            .child(
                Icon::new(Lucide::FolderOpen)
                    .with_size(px(32.))
                    .text_color(theme.muted_foreground),
            )
            .child(
                div()
                    .text_size(metrics::FONT_SIZE_HEADING)
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme.foreground)
                    .child(strings::EMPTY_TITLE),
            )
            .child(
                div()
                    .text_size(metrics::FONT_SIZE_BODY)
                    .text_color(theme.muted_foreground)
                    .child(strings::EMPTY_TEXT),
            )
            .child(
                ControlButton::new(self.id("choose-host"), ControlKind::Neutral)
                    .icon(Lucide::Server)
                    .label(strings::CHOOSE_HOST)
                    .on_click(
                        cx.listener(|this, _: &ClickEvent, window, cx| {
                            this.open_picker(window, cx)
                        }),
                    ),
            )
            .into_any_element()
    }

    fn render_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(picker) = &self.picker else {
            return div().into_any_element();
        };
        let search = picker.search.clone();
        let hosts = self.picker_hosts(cx);
        let none = self.state.read(cx).hosts().is_empty();
        let theme = cx.theme();
        let list = v_flex()
            .id(self.id("picker-list"))
            .test_support()
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .gap(px(2.))
            .children(hosts.into_iter().map(|(host, title)| {
                let picked = title.clone();
                h_flex()
                    .id(SharedString::from(format!(
                        "sftp-{}-pick-{}",
                        self.side.name(),
                        host.record_id
                    )))
                    .test_support()
                    .h(metrics::HEIGHT_NAV_ITEM)
                    .px(metrics::SPACE_3)
                    .gap(metrics::SPACE_2)
                    .rounded(metrics::RADIUS_CONTROL)
                    .hover(|style| style.bg(cx.theme().list_hover))
                    .child(
                        Icon::new(Lucide::Server)
                            .with_size(ICON_SIZE)
                            .text_color(theme.muted_foreground),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(metrics::FONT_SIZE_BODY)
                            .text_color(theme.foreground)
                            .child(title),
                    )
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        this.connect(host, picked.clone(), window, cx)
                    }))
            }))
            .when(self.picker_hosts(cx).is_empty(), |this| {
                this.child(
                    div()
                        .id(self.id("picker-none"))
                        .test_support()
                        .p(metrics::SPACE_3)
                        .text_size(metrics::FONT_SIZE_BODY)
                        .text_color(theme.muted_foreground)
                        .child(if none {
                            strings::PICKER_NO_HOSTS
                        } else {
                            strings::PICKER_EMPTY
                        }),
                )
            });
        v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .child(
                v_flex()
                    .id(self.id("picker"))
                    .test_support()
                    .w(PICKER_WIDTH)
                    .h(PICKER_HEIGHT)
                    .p(metrics::SPACE_3)
                    .gap(metrics::SPACE_2)
                    .rounded(metrics::RADIUS_CARD)
                    .border_1()
                    .border_color(theme.border)
                    .bg(theme.popover)
                    .child(widgets::text_input(
                        &search,
                        self.id("picker-search"),
                        false,
                        window,
                        cx,
                    ))
                    .child(list)
                    .child(
                        h_flex().justify_end().child(
                            ControlButton::new(self.id("picker-cancel"), ControlKind::Neutral)
                                .label(strings::CANCEL)
                                .on_click(
                                    cx.listener(|this, _: &ClickEvent, _, cx| {
                                        this.close_picker(cx)
                                    }),
                                ),
                        ),
                    ),
            )
            .into_any_element()
    }

    /// The question a remote panel shows, to fill in its form.
    fn source_ask(&mut self) -> Option<&mut Ask> {
        match &mut self.source {
            Source::Remote(remote) => remote.ask.as_mut(),
            _ => None,
        }
    }

    /// The connection of a remote panel that is not connected: its step, a
    /// question, or the reason with Retry / Reconnect and Close.
    fn render_connection(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let (handle, target) = match &self.source {
            Source::Remote(remote) => (remote.handle.clone(), remote.title.clone()),
            _ => return div().into_any_element(),
        };
        if let Some(Ask::Prompt { id, prompt, form }) = self.source_ask()
            && form.is_none()
        {
            let id = *id;
            let reply: crate::session_ui::prompt::ReplyHandler = Rc::new(move |reply, _, _| {
                let reply = match reply {
                    FormReply::Answer { value, keep } => PromptReply::Text { value, save: keep },
                    FormReply::Cancel => PromptReply::Cancel,
                    FormReply::Allow => PromptReply::Allow,
                };
                if let Err(error) = handle.reply(id, reply) {
                    tracing::debug!(%error, "answer not taken");
                }
            });
            let prompt = prompt.clone();
            let made = cx.new(|cx| PromptForm::new(prompt, target, reply, window, cx));
            made.update(cx, |form, cx| form.focus(window, cx));
            *form = Some(made);
        }
        let Source::Remote(remote) = &self.source else {
            return div().into_any_element();
        };
        let theme = cx.theme();
        match &remote.ask {
            Some(Ask::Prompt {
                form: Some(form), ..
            }) => {
                let form = form.clone();
                return v_flex()
                    .id(self.id("prompt"))
                    .test_support()
                    .size_full()
                    .items_center()
                    .justify_center()
                    .px(metrics::SPACE_4)
                    .child(form)
                    .into_any_element();
            }
            Some(Ask::Prompt { form: None, .. }) => return div().into_any_element(),
            Some(Ask::HostKey { ask, focus, .. }) => {
                let (ask, focus) = (ask.clone(), focus.clone());
                return v_flex()
                    .id(self.id("host-key"))
                    .test_support()
                    .size_full()
                    .items_center()
                    .justify_center()
                    .px(metrics::SPACE_4)
                    .child(host_key::render(&ask, &focus, cx))
                    .into_any_element();
            }
            None => {}
        }
        let view = remote.view.clone();
        let _ = window;
        match view.state {
            SessionState::Connecting => v_flex()
                .id(self.id("connecting"))
                .test_support()
                .size_full()
                .items_center()
                .justify_center()
                .gap(metrics::SPACE_2)
                .child(
                    div()
                        .text_size(metrics::FONT_SIZE_HEADING)
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme.foreground)
                        .child(strings::CONNECTING),
                )
                .when_some(view.step, |this, step| {
                    this.child(
                        div()
                            .text_size(metrics::FONT_SIZE_BODY)
                            .text_color(theme.muted_foreground)
                            .child(bridge::step_of(step).label()),
                    )
                })
                .child(
                    ControlButton::new(self.id("connect-cancel"), ControlKind::Neutral)
                        .label(strings::CANCEL)
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                            if let Some(handle) = this.handle() {
                                handle.cancel();
                            }
                            cx.notify();
                        })),
                )
                .into_any_element(),
            _ => {
                let reason = view
                    .error
                    .map(|error| bridge::failure_of(error).text())
                    .unwrap_or(model::Failure::Other.text());
                let retry = if view.state == SessionState::Dropped {
                    strings::RECONNECT
                } else {
                    strings::RETRY
                };
                v_flex()
                    .id(self.id("failure"))
                    .test_support()
                    .size_full()
                    .items_center()
                    .justify_center()
                    .gap(metrics::SPACE_3)
                    .px(metrics::SPACE_5)
                    .child(
                        div()
                            .id(self.id("failure-reason"))
                            .test_support()
                            .text_size(metrics::FONT_SIZE_BODY)
                            .text_color(theme.foreground)
                            .child(reason),
                    )
                    .child(
                        h_flex()
                            .gap(metrics::SPACE_2)
                            .child(
                                ControlButton::new(self.id("failure-close"), ControlKind::Neutral)
                                    .label(strings::CLOSE)
                                    .on_click(
                                        cx.listener(|this, _: &ClickEvent, _, cx| this.close(cx)),
                                    ),
                            )
                            .child(
                                ControlButton::new(self.id("failure-retry"), ControlKind::Accent)
                                    .label(retry)
                                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                        this.reconnect(cx)
                                    })),
                            ),
                    )
                    .into_any_element()
            }
        }
    }

    fn render_header(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let foreground = cx.theme().foreground;
        let icon = if self.is_local() {
            Lucide::Monitor
        } else {
            Lucide::Server
        };
        let panel = cx.entity().downgrade();
        let source = ControlButton::new(self.id("source"), ControlKind::Ghost)
            .icon(icon)
            .label(self.title())
            .trailing_icon(IconName::ChevronDown)
            .dropdown_menu(move |menu: PopupMenu, _, _| {
                let local = panel.clone();
                let host = panel.clone();
                menu.min_w(px(200.))
                    .item(menus::menu_item(
                        strings::SOURCE_LOCAL,
                        false,
                        false,
                        move |_, cx| {
                            let _ =
                                local.update(cx, |panel, cx| panel.open_local(local_start(), cx));
                        },
                    ))
                    .item(menus::menu_item(
                        strings::SOURCE_HOST,
                        false,
                        false,
                        move |window, cx| {
                            let _ = host.update(cx, |panel, cx| {
                                panel.close(cx);
                                panel.open_picker(window, cx);
                            });
                        },
                    ))
            });
        h_flex()
            .id(self.id("header"))
            .test_support()
            .w_full()
            .flex_none()
            .h(metrics::HEIGHT_CONTROL)
            .gap(metrics::SPACE_2)
            .child(h_flex().flex_1().min_w_0().justify_start().child(source))
            .when(self.is_ready(), |this| {
                this.child(div().w(FILTER_WIDTH).flex_none().child(widgets::text_input(
                    &self.filter,
                    self.id("filter"),
                    false,
                    window,
                    cx,
                )))
                .child(self.render_actions(cx))
            })
            .text_color(foreground)
            .into_any_element()
    }

    /// The Actions menu (UI 5.5): the items of the context menu acting on
    /// the selection, then Show Hidden Files, Select All and Close.
    fn render_actions(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let panel = cx.entity().downgrade();
        ControlButton::new(self.id("actions"), ControlKind::Ghost)
            .icon(Lucide::Ellipsis)
            .tooltip(strings::ACTIONS)
            .dropdown_menu(move |menu: PopupMenu, window, cx| {
                let Some(panel) = panel.upgrade() else {
                    return menu;
                };
                super::menu::panel_menu(menu, &panel, None, window, cx)
            })
    }

    fn render_nav(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let crumbs = listing::breadcrumbs(self.path(), self.style);
        let last = crumbs.len().saturating_sub(1);
        h_flex()
            .id(self.id("nav"))
            .test_support()
            .w_full()
            .flex_none()
            .gap(metrics::SPACE_1)
            .child(
                ControlButton::new(self.id("back"), ControlKind::Ghost)
                    .icon(Lucide::ArrowLeft)
                    .tooltip(strings::BACK)
                    .disabled(!self.history.can_back())
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.back(cx))),
            )
            .child(
                ControlButton::new(self.id("forward"), ControlKind::Ghost)
                    .icon(Lucide::ArrowRight)
                    .tooltip(strings::FORWARD)
                    .disabled(!self.history.can_forward())
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.forward(cx))),
            )
            .child(
                h_flex()
                    .id(self.id("crumbs"))
                    .test_support()
                    .flex_1()
                    .min_w_0()
                    .overflow_x_hidden()
                    .gap(px(2.))
                    .children(crumbs.into_iter().enumerate().map(|(index, crumb)| {
                        let path = crumb.path.clone();
                        let drive = crumb.drive;
                        let panel = cx.entity().downgrade();
                        h_flex()
                            .flex_none()
                            .gap(px(2.))
                            .when(index > 0, |this| {
                                this.child(
                                    Icon::new(IconName::ChevronRight)
                                        .with_size(px(12.))
                                        .text_color(theme.muted_foreground),
                                )
                            })
                            .child(
                                div()
                                    .id(SharedString::from(format!(
                                        "sftp-{}-crumb-{index}",
                                        self.side.name()
                                    )))
                                    .test_support()
                                    .px(metrics::SPACE_1)
                                    .rounded(metrics::RADIUS_CONTROL)
                                    .text_size(metrics::FONT_SIZE_DENSE)
                                    .text_color(if index == last {
                                        theme.foreground
                                    } else {
                                        theme.muted_foreground
                                    })
                                    .hover(|style| style.bg(cx.theme().list_hover))
                                    .child(crumb.label.clone())
                                    .when(!drive, |this| {
                                        this.on_click(cx.listener(
                                            move |this, _: &ClickEvent, _, cx| this.go(&path, cx),
                                        ))
                                    })
                                    .when(drive, |this| {
                                        this.on_click(move |_, window, cx| {
                                            let _ = panel.update(cx, |panel, cx| {
                                                panel.open_drives(window, cx)
                                            });
                                        })
                                    }),
                            )
                    })),
            )
            .child(
                ControlButton::new(self.id("refresh"), ControlKind::Ghost)
                    .icon(Lucide::RefreshCw)
                    .tooltip(strings::REFRESH)
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.refresh(cx))),
            )
            .into_any_element()
    }

    /// The drive selection (Windows, D55): a click on the drive crumb shows
    /// the drives under the path bar.
    fn open_drives(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.drives_open = !self.drives_open;
        cx.notify();
    }

    /// The drives, under the path bar while the selection is open.
    fn render_drives(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.drives_open {
            return None;
        }
        let theme = cx.theme();
        let current = self.path().to_owned();
        Some(
            h_flex()
                .id(self.id("drives"))
                .test_support()
                .flex_wrap()
                .gap(metrics::SPACE_1)
                .children(local::drives().into_iter().map(|drive| {
                    let path = drive.to_string_lossy().into_owned();
                    let selected = current.starts_with(path.as_str());
                    let label = path.trim_end_matches(['\\', '/']).to_owned();
                    let target = path.clone();
                    div()
                        .id(SharedString::from(format!(
                            "sftp-{}-drive-{label}",
                            self.side.name()
                        )))
                        .test_support()
                        .px(metrics::SPACE_2)
                        .py(px(2.))
                        .rounded(metrics::RADIUS_CONTROL)
                        .border_1()
                        .border_color(theme.border)
                        .when(selected, |this| this.bg(cx.theme().list_active))
                        .hover(|style| style.bg(cx.theme().list_hover))
                        .text_size(metrics::FONT_SIZE_DENSE)
                        .child(label)
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.drives_open = false;
                            this.go(&target, cx);
                        }))
                }))
                .into_any_element(),
        )
    }

    fn render_columns(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let sort = self.sort;
        let header = |column: Column, label: &'static str, width: Option<Pixels>| {
            let arrow = (sort.column == column).then(|| {
                Icon::new(if sort.descending {
                    IconName::ChevronDown
                } else {
                    IconName::ChevronUp
                })
                .with_size(px(12.))
                .text_color(theme.muted_foreground)
            });
            let cell = h_flex()
                .id(SharedString::from(format!(
                    "sftp-{}-column-{}",
                    self.side.name(),
                    label.to_lowercase().replace(' ', "-")
                )))
                .test_support()
                .gap(metrics::SPACE_1)
                .px(metrics::SPACE_2)
                .h_full()
                .text_size(metrics::FONT_SIZE_CAPTION)
                .font_weight(FontWeight::MEDIUM)
                .text_color(theme.muted_foreground)
                .hover(|style| style.text_color(cx.theme().foreground))
                .child(label)
                .children(arrow)
                .on_click(
                    cx.listener(move |this, _: &ClickEvent, _, cx| this.set_sort(column, cx)),
                );
            match width {
                Some(width) => cell.flex_none().w(width),
                None => cell.flex_1().min_w_0(),
            }
        };
        h_flex()
            .id(self.id("columns"))
            .test_support()
            .w_full()
            .flex_none()
            .h(px(28.))
            .border_b_1()
            .border_color(theme.border)
            .child(header(Column::Name, strings::COLUMN_NAME, None))
            .child(header(
                Column::Modified,
                strings::COLUMN_MODIFIED,
                Some(MODIFIED_COLUMN),
            ))
            .child(header(
                Column::Size,
                strings::COLUMN_SIZE,
                Some(SIZE_COLUMN),
            ))
            .child(header(
                Column::Kind,
                strings::COLUMN_KIND,
                Some(KIND_COLUMN),
            ))
            .into_any_element()
    }

    fn render_row(&mut self, index: usize, cx: &mut Context<Self>) -> AnyElement {
        let has_parent = listing::parent(self.path(), self.style).is_some();
        if has_parent && index == 0 {
            return self.render_parent_row(cx);
        }
        let Some(entry) = self.rows.get(index - usize::from(has_parent)).cloned() else {
            return div().into_any_element();
        };
        let theme = cx.theme();
        let selected = self.selected.contains(&entry.name);
        let icon = match entry.kind {
            Kind::Folder | Kind::Link { to_folder: true } => Lucide::Folder,
            Kind::Link { .. } => Lucide::FileSymlink,
            _ => Lucide::File,
        };
        let permissions = (!self.is_local())
            .then_some(entry.permissions)
            .flatten()
            .map(|mode| listing::mode_text(mode, entry.kind));
        let name = entry.name.clone();
        let opened = entry.clone();
        let menu_entry = entry.clone();
        let panel = cx.entity().downgrade();
        let row_id = SharedString::from(format!("sftp-{}-row-{}", self.side.name(), entry.name));
        // A drag takes the selection when it starts on a selected row.
        let dragged = super::DraggedFiles {
            side: self.side,
            entries: if selected {
                self.selection()
            } else {
                vec![entry.clone()]
            },
        };
        let label: SharedString = match dragged.entries.as_slice() {
            [one] => one.name.clone().into(),
            many => format!("{} items", many.len()).into(),
        };
        let muted = theme.muted_foreground;
        h_flex()
            .id(row_id)
            .test_support()
            .w_full()
            .h(ROW_HEIGHT)
            .rounded(metrics::RADIUS_CONTROL)
            .when(selected, |this| this.bg(cx.theme().list_active))
            .when(!selected, |this| {
                this.hover(|style| style.bg(cx.theme().list_hover))
            })
            .child(
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .gap(metrics::SPACE_2)
                    .px(metrics::SPACE_2)
                    .child(Icon::new(icon).with_size(ICON_SIZE).text_color(
                        if entry.is_folder_like() {
                            theme.primary
                        } else {
                            muted
                        },
                    ))
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .child(
                                div()
                                    .truncate()
                                    .text_size(metrics::FONT_SIZE_BODY)
                                    .text_color(theme.foreground)
                                    .child(super::display_name(&entry.name)),
                            )
                            .when_some(permissions, |this, permissions| {
                                this.child(
                                    div()
                                        .font_family(crate::terminal::fonts::JETBRAINS_MONO)
                                        .text_size(metrics::FONT_SIZE_CAPTION)
                                        .text_color(muted)
                                        .child(permissions),
                                )
                            }),
                    ),
            )
            .child(cell(date_text(entry.modified), MODIFIED_COLUMN, muted))
            .child(cell(entry.size_text(), SIZE_COLUMN, muted))
            .child(cell(entry.kind_text(), KIND_COLUMN, muted))
            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                this.focus.focus(window, cx);
                if event.click_count() >= 2
                    && !event.modifiers().shift
                    && !event.modifiers().secondary()
                {
                    this.open_entry(&opened, cx);
                    return;
                }
                let mode = if event.modifiers().shift {
                    SelectMode::Range
                } else if event.modifiers().secondary() {
                    SelectMode::Toggle
                } else {
                    SelectMode::Replace
                };
                this.click(&name, mode, cx);
            }))
            .on_drag(dragged, move |_: &super::DraggedFiles, _, _, cx| {
                cx.new(|_| super::DragLabel::new(label.clone()))
            })
            .context_menu(move |menu, window, cx| {
                let Some(panel) = panel.upgrade() else {
                    return menu;
                };
                panel.update(cx, |panel, cx| {
                    if !panel.is_selected(&menu_entry.name) {
                        panel.click(&menu_entry.name, SelectMode::Replace, cx);
                    }
                });
                super::menu::panel_menu(menu, &panel, Some(menu_entry.clone()), window, cx)
            })
            .into_any_element()
    }

    fn render_parent_row(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        h_flex()
            .id(self.id("parent"))
            .test_support()
            .w_full()
            .h(ROW_HEIGHT)
            .px(metrics::SPACE_2)
            .gap(metrics::SPACE_2)
            .rounded(metrics::RADIUS_CONTROL)
            .hover(|style| style.bg(cx.theme().list_hover))
            .child(
                Icon::new(Lucide::CornerLeftUp)
                    .with_size(ICON_SIZE)
                    .text_color(theme.muted_foreground),
            )
            .child(
                div()
                    .text_size(metrics::FONT_SIZE_BODY)
                    .text_color(theme.foreground)
                    .child(strings::PARENT_ROW),
            )
            .on_click(cx.listener(|this, event: &ClickEvent, _, cx| {
                if event.click_count() >= 2 {
                    this.up(cx);
                }
            }))
            .into_any_element()
    }

    fn render_table(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let empty_text = match &self.load {
            Load::Loading if self.rows.is_empty() => Some(strings::LOADING.into()),
            Load::Failed(reason) => Some(reason.clone()),
            _ if self.rows.is_empty() && self.entries.is_empty() => {
                Some(strings::FOLDER_EMPTY.into())
            }
            _ if self.rows.is_empty() => Some(strings::NOTHING_MATCHES.into()),
            _ => None,
        };
        let list = v_virtual_list(
            cx.entity(),
            self.id("rows-list"),
            self.row_sizes.clone(),
            |this: &mut Self, range, _, cx| {
                range
                    .map(|row| this.render_row(row, cx))
                    .collect::<Vec<_>>()
            },
        )
        .track_scroll(&self.scroll)
        .size_full();
        v_flex()
            .id(self.id("rows"))
            .test_support()
            .relative()
            .flex_1()
            .min_h_0()
            .w_full()
            .child(list)
            .child(Scrollbar::vertical(&self.scroll))
            .when_some(empty_text, |this, text: SharedString| {
                this.child(
                    div()
                        .id(self.id("note"))
                        .test_support()
                        .absolute()
                        .top(ROW_HEIGHT * 2.)
                        .left_0()
                        .right_0()
                        .flex()
                        .justify_center()
                        .px(metrics::SPACE_4)
                        .text_size(metrics::FONT_SIZE_BODY)
                        .text_color(theme.muted_foreground)
                        .child(text),
                )
            })
            .into_any_element()
    }
}

/// A fixed-width cell of a row.
fn cell(text: String, width: Pixels, color: gpui::Hsla) -> impl IntoElement {
    div()
        .flex_none()
        .w(width)
        .px(metrics::SPACE_2)
        .truncate()
        .text_size(metrics::FONT_SIZE_DENSE)
        .text_color(color)
        .child(text)
}

impl Render for FilePanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let background = cx.theme().background;
        let content = match &self.source {
            Source::Empty => self.render_empty(window, cx),
            Source::Remote(remote) if remote.view.state != SessionState::Connected => {
                self.render_connection(window, cx)
            }
            _ => v_flex()
                .size_full()
                .gap(metrics::SPACE_2)
                .child(self.render_nav(cx))
                .children(self.render_drives(cx))
                .child(self.render_columns(cx))
                .child(self.render_table(cx))
                .into_any_element(),
        };
        v_flex()
            .id(self.id("panel"))
            .test_support()
            .key_context("SftpPanel")
            .track_focus(&self.focus)
            .size_full()
            .min_w_0()
            .p(metrics::SPACE_3)
            .gap(metrics::SPACE_2)
            .bg(background)
            .when(!matches!(self.source, Source::Empty), |this| {
                this.child(self.render_header(window, cx))
            })
            .child(content)
    }
}
