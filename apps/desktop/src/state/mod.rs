//! The app state: one GPUI entity that owns the `shellpp-core` facade, the
//! lock state, the loaded hosts and groups, the current location and the
//! selection. Views never talk to the core directly; there is no IPC layer.
//!
//! Reading: [`AppState::hosts`], [`AppState::groups`], [`AppState::hosts_in`],
//! [`AppState::groups_in`], [`AppState::host`], [`AppState::group`], or
//! [`AppState::query`] for the core's own search and sorting.
//!
//! Writing: [`AppState::save_host`], [`AppState::save_group`],
//! [`AppState::delete_records`], [`AppState::move_records`],
//! [`AppState::duplicate_record`], and in [`hosts_view`]
//! [`AppState::move_records_to`] and [`AppState::copy_hosts_to`]. Each runs
//! synchronously (one record: encryption plus one SQLite write), reloads the
//! lists and emits [`AppStateEvent::RecordsChanged`]. Validation errors come
//! back as `CoreError::Validation` with the failing fields. Everything is
//! written to the encrypted store at once and survives a restart; a removal
//! can be undone for 5 seconds ([`undo`], [`AppState::restore_records`]).
//!
//! Lock: [`AppState::create_vault`] (first run) and [`AppState::unlock`] run Argon2id
//! on the background executor and return a task; [`AppState::lock`] is
//! immediate. While not unlocked the lists are empty.
//!
//! Core events (records written elsewhere, auto-lock from the tick) arrive
//! through [`crate::platform::events`] and trigger the same reload.
//!
//! Auto-lock (UI 13.3): when the interval has passed, the tick (and the
//! first input after a sleep, [`AppState::activity`]) asks the windows to
//! lock with [`AppStateEvent::AutoLockDue`], so their details panels save
//! pending edits before the keys go; the core's own tick is the backstop.

pub mod hosts_view;
pub mod model;
pub mod records_edit;
pub mod security;
pub mod selection;
pub mod undo;

use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt as _;
use gpui::{AppContext as _, Context, EventEmitter, SharedString, Task};
use secrecy::SecretString;
use serde_json::{Map, Value};
use shellpp_core::model::types::RecordKind;
use shellpp_core::service::dto::{
    CoreEvent, CoreEventKind, LockState, QueryScope, RecordBase, RecordDeleteResult,
    RecordMoveInput, RecordRef, RecordSaveInput, RecordType, RecordView, RecordsQuery, RecordsRef,
    RecoveryKeyResult, SortMode, VaultKind,
};
use shellpp_core::{Core, CoreError};
use uuid::Uuid;

use crate::dirs::AppDirs;
use crate::platform::PlatformExtras;
use crate::platform::events::EventReceiver;
use crate::profile::TempProfile;

pub use model::{GroupItem, HostItem};
pub use selection::{SelectMode, Selection};

/// An opened core and what belongs to it, ready for [`AppState::new`].
pub struct OpenedCore {
    pub core: Arc<Core>,
    /// Receiver of the core's broadcast events.
    pub events: EventReceiver,
    pub extras: PlatformExtras,
    /// The directories in use (`tmp/` is emptied at exit).
    pub dirs: Option<AppDirs>,
    /// A temporary profile owned by this run (`--demo`, tests): deleted at
    /// shutdown, after the core has closed its files.
    pub temp_profile: Option<TempProfile>,
}

impl std::fmt::Debug for OpenedCore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenedCore")
            .field("temporary", &self.temp_profile.is_some())
            .finish_non_exhaustive()
    }
}

/// What changed in the app state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AppStateEvent {
    /// The lock state changed (first run done, unlocked, locked, auto-locked).
    LockChanged(LockState),
    /// Hosts or groups were reloaded after a write.
    RecordsChanged,
    /// The selection changed.
    SelectionChanged,
    /// The current group (navigation) changed.
    LocationChanged(Option<Uuid>),
    /// The auto-lock interval has passed: the windows save pending edits
    /// and lock (`shell::lock_all`). Nothing is locked yet when it arrives.
    AutoLockDue,
}

/// A saved workspace as the new tab page, Ctrl+K and the menus list it
/// (UI 4.5, 6).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkspaceItem {
    pub id: Uuid,
    pub name: SharedString,
    /// The hosts it links to that do not exist here, by the names the
    /// workspace keeps; an empty name for one it has no name of (D94).
    pub missing: Rc<[SharedString]>,
}

/// The application state entity. Field order matters: the core is dropped
/// before the temporary profile that holds its files.
pub struct AppState {
    core: Option<Arc<Core>>,
    /// Why the core did not open (shown instead of the app).
    open_error: Option<String>,
    lock: LockState,
    /// The personal vault (`None` while locked).
    vault_id: Option<Uuid>,
    hosts: Rc<Vec<HostItem>>,
    groups: Rc<Vec<GroupItem>>,
    /// The workspaces' ids and names (UI 4.5), read with the records.
    workspaces: Rc<Vec<WorkspaceItem>>,
    /// The group whose contents the Hosts screen shows; `None` is the top level.
    location: Option<Uuid>,
    selection: Selection,
    /// Removals that can still be undone ([`undo::UNDO_WINDOW`]).
    removals: undo::PendingRemovals,
    extras: PlatformExtras,
    /// Counts the record changes other than command history, the
    /// connection log and Restore ([`AppState::records_generation`]).
    records_generation: u64,
    dirs: Option<AppDirs>,
    shut_down: bool,
    _event_task: Option<Task<()>>,
    tick_task: Option<Task<()>>,
    /// Retention of the connection log runs (D68): started with the tick.
    retention_on: bool,
    /// Prunes the log now and every [`RETENTION_INTERVAL`] while unlocked.
    retention_task: Option<Task<()>>,
    /// How many times retention ran (the tests read it).
    retention_runs: u64,
    temp_profile: Option<TempProfile>,
    /// Stage 7: the last sync status read from the core (UI 12.2), the
    /// 60 s cycle while unlocked and the cycle 2 s after a change.
    sync_status: Option<shellpp_core::service::SyncStatus>,
    /// The records with a sync conflict (UI 13.6): their cards show a
    /// badge. Read with the status.
    sync_conflicted: Rc<std::collections::HashSet<Uuid>>,
    sync_task: Option<Task<()>>,
    sync_kick: Option<Task<()>>,
}

/// The automatic sync cycle while unlocked (docs/sync-protocol.md, 6.11).
pub const SYNC_INTERVAL: Duration = Duration::from_secs(60);
/// The cycle after a local change.
pub const SYNC_AFTER_CHANGE: Duration = Duration::from_secs(2);

/// How often retention of the connection log runs while the app is
/// unlocked (D68; it also runs when the data becomes available).
pub const RETENTION_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);

impl std::fmt::Debug for AppState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppState")
            .field("lock", &self.lock)
            .field("hosts", &self.hosts.len())
            .field("groups", &self.groups.len())
            .finish_non_exhaustive()
    }
}

impl EventEmitter<AppStateEvent> for AppState {}

/// Whether a write of `classes` touches only command history, the
/// connection log and Restore (`history_entry`, `history_hint_state`,
/// `connection_log_entry`, `last_session`): such writes leave
/// [`AppState::records_generation`] as it is.
fn only_log_classes(classes: &[u8]) -> bool {
    let log = [
        RecordKind::HistoryEntry,
        RecordKind::HistoryHintState,
        RecordKind::ConnectionLogEntry,
        RecordKind::LastSession,
    ]
    .map(RecordKind::class);
    classes.iter().all(|class| log.contains(class))
}

impl AppState {
    /// Takes over an opened core: reads the lock state, loads the records
    /// when already unlocked and starts listening to core events.
    pub fn new(opened: OpenedCore, cx: &mut Context<Self>) -> Self {
        let OpenedCore {
            core,
            mut events,
            extras,
            dirs,
            temp_profile,
        } = opened;
        let event_task = cx.spawn(async move |this, cx| {
            while let Some(event) = events.next().await {
                if this
                    .update(cx, |state, cx| state.on_core_event(&event, cx))
                    .is_err()
                {
                    break;
                }
            }
        });
        let mut state = Self {
            lock: core.lock_status().state,
            core: Some(core),
            open_error: None,
            vault_id: None,
            hosts: Rc::default(),
            groups: Rc::default(),
            workspaces: Rc::default(),
            location: None,
            selection: Selection::default(),
            removals: undo::PendingRemovals::default(),
            extras,
            records_generation: 0,
            dirs,
            shut_down: false,
            _event_task: Some(event_task),
            tick_task: None,
            retention_on: false,
            retention_task: None,
            retention_runs: 0,
            sync_status: None,
            sync_conflicted: Rc::default(),
            sync_task: None,
            sync_kick: None,
            temp_profile,
        };
        if state.lock == LockState::Unlocked {
            state.load_unlocked();
        }
        state
    }

    /// A state without a core: the app shows `message` (for example "the
    /// data directory is in use by another Shell++ window").
    pub fn failed(message: impl Into<String>) -> Self {
        Self {
            core: None,
            open_error: Some(message.into()),
            lock: LockState::Locked,
            vault_id: None,
            hosts: Rc::default(),
            groups: Rc::default(),
            workspaces: Rc::default(),
            location: None,
            selection: Selection::default(),
            removals: undo::PendingRemovals::default(),
            extras: PlatformExtras::default(),
            records_generation: 0,
            dirs: None,
            shut_down: false,
            _event_task: None,
            tick_task: None,
            retention_on: false,
            retention_task: None,
            retention_runs: 0,
            sync_status: None,
            sync_conflicted: Rc::default(),
            sync_task: None,
            sync_kick: None,
            temp_profile: None,
        }
    }

    /// About once per second: a due auto-lock goes to the windows first
    /// ([`AppState::check_auto_lock`]), then `core.tick()` runs on the
    /// background executor (undo expiry, clipboard clearing, and the lock
    /// itself when no window took the request). The app starts it; tests
    /// usually do not.
    pub fn start_tick(&mut self, cx: &mut Context<Self>) {
        let Some(core) = self.core.clone() else {
            return;
        };
        self.start_retention(cx);
        self.tick_task = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                // Checked here, on the app's thread, before the core's tick:
                // the windows save their edits and lock while handling the
                // event, so the tick never locks ahead of them.
                if this
                    .update(cx, |state, cx| state.check_auto_lock(cx))
                    .is_err()
                {
                    break;
                }
                let core = core.clone();
                cx.background_spawn(async move { core.tick() }).await;
            }
        }));
    }

    /// Retention of the connection log (UI 12.6, D68): when the data is
    /// available (now, or at the next unlock) and every
    /// [`RETENTION_INTERVAL`] after, not only when the Logs screen reads
    /// the log. The app starts it with the tick.
    pub fn start_retention(&mut self, cx: &mut Context<Self>) {
        self.retention_on = true;
        self.restart_retention(cx);
    }

    /// How many times retention ran.
    pub fn retention_runs(&self) -> u64 {
        self.retention_runs
    }

    /// Starts retention over while unlocked; stops it otherwise.
    fn restart_retention(&mut self, cx: &mut Context<Self>) {
        self.retention_task = None;
        if !self.retention_on || self.lock != LockState::Unlocked {
            return;
        }
        let Some(core) = self.core.clone() else {
            return;
        };
        self.retention_task = Some(cx.spawn(async move |this, cx| {
            loop {
                let pruning = core.clone();
                let pruned = cx
                    .background_spawn(async move { pruning.connection_log_prune() })
                    .await;
                if let Err(error) = pruned {
                    tracing::warn!(code = error.code(), "connection log retention failed");
                }
                if this
                    .update(cx, |state, _| state.retention_runs += 1)
                    .is_err()
                {
                    break;
                }
                cx.background_executor().timer(RETENTION_INTERVAL).await;
            }
        }));
    }

    // -- Sync (stage 7) ------------------------------------------------------

    /// The cached sync status (`None` before the first read or locked).
    pub fn sync_status(&self) -> Option<&shellpp_core::service::SyncStatus> {
        self.sync_status.as_ref()
    }

    /// Sets the cached status (the screenshot harness shows a bound account
    /// without a server).
    pub fn set_sync_status(
        &mut self,
        status: shellpp_core::service::SyncStatus,
        cx: &mut Context<Self>,
    ) {
        self.sync_status = Some(status);
        cx.notify();
    }

    /// Re-reads the status from the core.
    pub fn refresh_sync_status(&mut self, cx: &mut Context<Self>) {
        if self.lock != LockState::Unlocked {
            return;
        }
        if let Some(core) = &self.core
            && let Ok(status) = core.sync_status()
        {
            self.sync_conflicted = Rc::new(if status.conflicts > 0 {
                core.sync_conflicts()
                    .unwrap_or_default()
                    .iter()
                    .map(|item| item.original_record_id)
                    .collect()
            } else {
                Default::default()
            });
            self.sync_status = Some(status);
            cx.notify();
        }
    }

    /// The records with a sync conflict (UI 13.6).
    pub fn sync_conflicted(&self) -> &std::collections::HashSet<Uuid> {
        &self.sync_conflicted
    }

    /// The automatic cycle: after unlock, then every 60 s while unlocked
    /// (a no-op without a bound account; the core keeps the retry pause).
    fn restart_sync(&mut self, cx: &mut Context<Self>) {
        self.sync_task = None;
        self.sync_kick = None;
        if self.lock != LockState::Unlocked {
            self.sync_status = None;
            self.sync_conflicted = Rc::default();
            return;
        }
        self.refresh_sync_status(cx);
        let Some(core) = self.core.clone() else {
            return;
        };
        self.sync_task = Some(cx.spawn(async move |this, cx| {
            loop {
                let syncing = core.clone();
                let result = cx
                    .background_spawn(async move { syncing.sync_auto() })
                    .await;
                if let Err(error) = result {
                    tracing::info!(code = error.code(), "sync cycle failed");
                }
                if this
                    .update(cx, |state, cx| state.refresh_sync_status(cx))
                    .is_err()
                {
                    break;
                }
                cx.background_executor().timer(SYNC_INTERVAL).await;
            }
        }));
    }

    /// One cycle 2 s after the last local change, while bound.
    pub fn kick_sync(&mut self, cx: &mut Context<Self>) {
        if self.lock != LockState::Unlocked
            || !self
                .sync_status
                .as_ref()
                .is_some_and(|status| status.bound && status.signed_in)
        {
            return;
        }
        let Some(core) = self.core.clone() else {
            return;
        };
        self.sync_kick = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SYNC_AFTER_CHANGE).await;
            let _ = cx.background_spawn(async move { core.sync_auto() }).await;
            let _ = this.update(cx, |state, cx| state.refresh_sync_status(cx));
        }));
    }

    /// "Sync now" (UI 12.2): refused items are tried again.
    pub fn sync_now(&mut self, cx: &mut Context<Self>) {
        let Some(core) = self.core.clone() else {
            return;
        };
        self.sync_kick = Some(cx.spawn(async move |this, cx| {
            let result = cx.background_spawn(async move { core.sync_now() }).await;
            if let Err(error) = result {
                tracing::info!(code = error.code(), "sync now failed");
            }
            let _ = this.update(cx, |state, cx| state.refresh_sync_status(cx));
        }));
    }

    /// Emits [`AppStateEvent::AutoLockDue`] when the auto-lock interval has
    /// passed. Returns whether it did.
    pub fn check_auto_lock(&mut self, cx: &mut Context<Self>) -> bool {
        let due = self.core.as_ref().is_some_and(|core| core.auto_lock_due());
        if due {
            cx.emit(AppStateEvent::AutoLockDue);
        }
        due
    }

    // -- Reading ------------------------------------------------------------

    /// The core, while it is open.
    pub fn core(&self) -> Option<&Arc<Core>> {
        self.core.as_ref()
    }

    pub fn open_error(&self) -> Option<&str> {
        self.open_error.as_deref()
    }

    pub fn lock_state(&self) -> LockState {
        self.lock
    }

    pub fn is_unlocked(&self) -> bool {
        self.lock == LockState::Unlocked
    }

    pub fn vault_id(&self) -> Option<Uuid> {
        self.vault_id
    }

    /// Changes with every [`AppStateEvent::RecordsChanged`] that may touch
    /// records other than command history, the connection log and Restore
    /// (hosts, groups, keys, known hosts), and with the lock. Views whose
    /// data come from those records (Known Hosts, the "Record history" flag
    /// of running sessions) skip the other events: a recorded command must
    /// not reload them.
    pub fn records_generation(&self) -> u64 {
        self.records_generation
    }

    /// The app's temporary folder (`tmp/`, emptied when the app exits;
    /// SFTP Open copies, storage-format 9); `None` without a profile.
    pub fn tmp_dir(&self) -> Option<std::path::PathBuf> {
        self.dirs.as_ref().map(|dirs| dirs.tmp.clone())
    }

    /// Every host of the personal vault, in manual order.
    pub fn hosts(&self) -> Rc<Vec<HostItem>> {
        self.hosts.clone()
    }

    /// The workspaces' ids and names, by name (UI 4.5).
    pub fn workspaces(&self) -> Rc<Vec<WorkspaceItem>> {
        self.workspaces.clone()
    }

    /// Every group of the personal vault, in manual order.
    pub fn groups(&self) -> Rc<Vec<GroupItem>> {
        self.groups.clone()
    }

    pub fn host(&self, id: Uuid) -> Option<&HostItem> {
        self.hosts.iter().find(|host| host.record_id == id)
    }

    pub fn group(&self, id: Uuid) -> Option<&GroupItem> {
        self.groups.iter().find(|group| group.record_id == id)
    }

    /// Hosts directly inside `group` (`None`: the top level).
    pub fn hosts_in(&self, group: Option<Uuid>) -> Vec<&HostItem> {
        self.hosts
            .iter()
            .filter(|host| host.group_id == group)
            .collect()
    }

    /// Groups directly inside `parent` (`None`: the top level).
    pub fn groups_in(&self, parent: Option<Uuid>) -> Vec<&GroupItem> {
        self.groups
            .iter()
            .filter(|group| group.parent_id == parent)
            .collect()
    }

    /// The core's own query (substring search, tag filter, sort modes,
    /// scopes). Fails with `CoreError::Locked` while locked.
    pub fn query(&self, query: RecordsQuery) -> Result<Vec<RecordView>, CoreError> {
        self.require_core()?.records_query(query)
    }

    // -- Navigation and selection --------------------------------------------

    /// The group the Hosts screen shows (`None`: the top level).
    pub fn location(&self) -> Option<Uuid> {
        self.location
    }

    /// The groups from the top level down to the current location (breadcrumbs).
    pub fn location_path(&self) -> Vec<&GroupItem> {
        let Some(current) = self.location.and_then(|id| self.group(id)) else {
            return Vec::new();
        };
        let mut path: Vec<&GroupItem> = current
            .path
            .iter()
            .filter_map(|id| self.group(*id))
            .collect();
        path.push(current);
        path
    }

    /// Opens a group like a folder (`None`: back to the top level). Clears
    /// the selection.
    pub fn set_location(&mut self, group: Option<Uuid>, cx: &mut Context<Self>) {
        let group = group.filter(|id| self.group(*id).is_some());
        if self.location != group {
            self.location = group;
            self.selection.clear();
            cx.emit(AppStateEvent::LocationChanged(group));
            cx.emit(AppStateEvent::SelectionChanged);
            cx.notify();
        }
    }

    pub fn selection(&self) -> &Selection {
        &self.selection
    }

    /// Applies a click on a record; `order` is the record order on screen.
    pub fn select(&mut self, id: Uuid, mode: SelectMode, order: &[Uuid], cx: &mut Context<Self>) {
        let before = self.selection.clone();
        self.selection.apply(id, mode, order);
        if self.selection != before {
            cx.emit(AppStateEvent::SelectionChanged);
            cx.notify();
        }
    }

    pub fn clear_selection(&mut self, cx: &mut Context<Self>) {
        if !self.selection.is_empty() {
            self.selection.clear();
            cx.emit(AppStateEvent::SelectionChanged);
            cx.notify();
        }
    }

    /// Keeps only the selected records that are in `visible` (the cards on
    /// screen): a search or a tag filter never leaves hidden cards selected.
    pub fn retain_selection(&mut self, visible: &[Uuid], cx: &mut Context<Self>) {
        let before = self.selection.clone();
        self.selection.retain(|id| visible.contains(id));
        if self.selection != before {
            cx.emit(AppStateEvent::SelectionChanged);
            cx.notify();
        }
    }

    // -- Writing ---------------------------------------------------------------

    /// Creates (`record_id: None`) or updates a host. `data` is the full data
    /// object without secret members: start from [`HostItem::data_map`] and
    /// change only the edited members.
    pub fn save_host(
        &mut self,
        record_id: Option<Uuid>,
        data: Map<String, Value>,
        cx: &mut Context<Self>,
    ) -> Result<HostItem, CoreError> {
        match self.save(record_id, RecordType::Host, data, None, cx)? {
            RecordView::Host(view) => Ok(HostItem::from_view(&view)),
            RecordView::Group(_) => Err(CoreError::NotFound),
        }
    }

    /// Creates (`record_id: None`) or updates a group.
    pub fn save_group(
        &mut self,
        record_id: Option<Uuid>,
        data: Map<String, Value>,
        cx: &mut Context<Self>,
    ) -> Result<GroupItem, CoreError> {
        self.save_group_from(record_id, data, None, cx)
    }

    /// [`AppState::save_group`]; with `base`, only what `data` changes
    /// against it is written (D220).
    pub fn save_group_from(
        &mut self,
        record_id: Option<Uuid>,
        data: Map<String, Value>,
        base: Option<RecordBase>,
        cx: &mut Context<Self>,
    ) -> Result<GroupItem, CoreError> {
        match self.save(record_id, RecordType::Group, data, base, cx)? {
            RecordView::Group(view) => Ok(GroupItem::from_view(&view)),
            RecordView::Host(_) => Err(CoreError::NotFound),
        }
    }

    fn save(
        &mut self,
        record_id: Option<Uuid>,
        record_type: RecordType,
        data: Map<String, Value>,
        base: Option<RecordBase>,
        cx: &mut Context<Self>,
    ) -> Result<RecordView, CoreError> {
        let vault_id = self.require_vault()?;
        let view = self.require_core()?.record_save(RecordSaveInput {
            base,
            vault_id,
            record_id,
            record_type,
            data,
            secrets: Default::default(),
        })?;
        self.records_written(cx);
        Ok(view)
    }

    /// Deletes hosts and groups (a removed group's children move one level
    /// up). [`AppState::restore_records`] undoes it for
    /// [`undo::UNDO_WINDOW`]; after that the removal is final.
    pub fn delete_records(
        &mut self,
        record_ids: Vec<Uuid>,
        cx: &mut Context<Self>,
    ) -> Result<RecordDeleteResult, CoreError> {
        let vault_id = self.require_vault()?;
        let children = undo::moved_children(&record_ids, &self.hosts, &self.groups);
        let result = self.require_core()?.record_delete(RecordsRef {
            vault_id,
            record_ids,
        })?;
        self.remember_removal(result.deleted.clone(), children, cx);
        self.records_written(cx);
        Ok(result)
    }

    /// Moves records into `target_group` (`None`: the top level), at the end.
    pub fn move_records(
        &mut self,
        record_ids: Vec<Uuid>,
        target_group_id: Option<Uuid>,
        cx: &mut Context<Self>,
    ) -> Result<(), CoreError> {
        let vault_id = self.require_vault()?;
        self.require_core()?.record_move(RecordMoveInput {
            vault_id,
            record_ids,
            target_group_id,
            position: Default::default(),
        })?;
        self.records_written(cx);
        Ok(())
    }

    /// Duplicates a host or group (data-model 7.2 "Duplicate").
    pub fn duplicate_record(
        &mut self,
        record_id: Uuid,
        cx: &mut Context<Self>,
    ) -> Result<RecordView, CoreError> {
        let vault_id = self.require_vault()?;
        let view = self.require_core()?.record_duplicate(RecordRef {
            vault_id,
            record_id,
        })?;
        self.records_written(cx);
        Ok(view)
    }

    /// Tells the screens to read the records again (a setting that changes
    /// what they list, such as `logs.retention_days`).
    pub fn reload_records(&mut self, cx: &mut Context<Self>) {
        self.records_written(cx);
    }

    fn records_written(&mut self, cx: &mut Context<Self>) {
        self.records_generation += 1;
        self.reload_and_notify(cx);
    }

    /// Reloads the lists and tells the views (also when the current group
    /// disappeared and the location went back to the top level).
    fn reload_and_notify(&mut self, cx: &mut Context<Self>) {
        let location = self.location;
        self.reload();
        cx.emit(AppStateEvent::RecordsChanged);
        if self.location != location {
            cx.emit(AppStateEvent::LocationChanged(self.location));
        }
        cx.notify();
    }

    // -- Lock ------------------------------------------------------------------

    /// First run without "Remember on this device": sets the master password
    /// and unlocks. Argon2id runs on the background executor. The first-run
    /// screen uses [`AppState::create_vault`] (UI 13.1); the returned recovery
    /// key is never shown in local mode (D29).
    pub fn first_run_setup(
        &mut self,
        password: SecretString,
        cx: &mut Context<Self>,
    ) -> Task<Result<RecoveryKeyResult, CoreError>> {
        let core = match self.require_core() {
            Ok(core) => core.clone(),
            Err(error) => return Task::ready(Err(error)),
        };
        let work = cx.background_spawn(async move { core.first_run_setup(password) });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |state, cx| state.refresh_lock(cx));
            if result.is_ok() {
                tracing::info!("first run completed");
            }
            result
        })
    }

    /// Unlock with the master password (UI 13.2). Argon2id runs on the
    /// background executor. A wrong password is `CoreError::InvalidPassword`.
    pub fn unlock(
        &mut self,
        password: SecretString,
        cx: &mut Context<Self>,
    ) -> Task<Result<(), CoreError>> {
        let core = match self.require_core() {
            Ok(core) => core.clone(),
            Err(error) => return Task::ready(Err(error)),
        };
        let work = cx.background_spawn(async move { core.unlock(password) });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |state, cx| state.refresh_lock(cx));
            result.map(|_| ())
        })
    }

    /// Locks now (UI 13.3): keys are wiped and the lists emptied.
    pub fn lock(&mut self, cx: &mut Context<Self>) -> Result<(), CoreError> {
        let result = self.require_core()?.lock();
        self.refresh_lock(cx);
        result.map(|_| ())
    }

    /// Reports keyboard or mouse input to the auto-lock timer. Returns
    /// `true` instead when the interval has already passed (the first input
    /// after a sleep): the caller then locks through the windows
    /// (`shell::lock_all`), which save pending edits first.
    #[must_use]
    pub fn activity(&self) -> bool {
        let Some(core) = &self.core else {
            return false;
        };
        if core.auto_lock_due() {
            return true;
        }
        core.activity();
        false
    }

    // -- Shutdown --------------------------------------------------------------

    /// Exit work, run once: stops the tick, lets the core clear sensitive
    /// clipboard content and close its files, empties `tmp/`, releases the
    /// clipboard and deletes a temporary profile.
    pub fn shutdown(&mut self) {
        if self.shut_down {
            return;
        }
        self.shut_down = true;
        self.tick_task = None;
        self.retention_task = None;
        self.removals.clear();
        if let Some(core) = self.core.take() {
            core.on_exit();
            drop(core);
        }
        self.hosts = Rc::default();
        self.groups = Rc::default();
        self.selection.clear();
        if let Some(dirs) = &self.dirs {
            let failed = dirs.empty_tmp();
            if failed > 0 {
                tracing::warn!(failed, "tmp entries left for the next start");
            }
        }
        self.extras.shutdown();
        if let Some(mut profile) = self.temp_profile.take() {
            // The log file of a temporary profile lives inside it.
            crate::logging::close_file();
            match profile.remove() {
                Ok(()) => tracing::info!("temporary profile removed"),
                Err(_) => tracing::warn!("temporary profile could not be removed"),
            }
        }
        tracing::info!("shutdown complete");
    }

    // -- Internals -------------------------------------------------------------

    fn require_core(&self) -> Result<&Arc<Core>, CoreError> {
        self.core.as_ref().ok_or(CoreError::Unavailable)
    }

    fn require_vault(&self) -> Result<Uuid, CoreError> {
        if self.lock != LockState::Unlocked {
            return Err(CoreError::Locked);
        }
        self.vault_id.ok_or(CoreError::Locked)
    }

    fn on_core_event(&mut self, event: &CoreEvent, cx: &mut Context<Self>) {
        match &event.kind {
            CoreEventKind::LockChanged { .. } => self.refresh_lock(cx),
            CoreEventKind::RecordsChanged { classes, .. } => {
                // A local change goes up 2 s later (section 6.11).
                self.kick_sync(cx);
                if !only_log_classes(classes) {
                    self.records_generation += 1;
                }
                if self.lock == LockState::Unlocked {
                    self.reload_and_notify(cx);
                }
            }
            CoreEventKind::VaultsChanged { .. } => {
                self.records_generation += 1;
                if self.lock == LockState::Unlocked {
                    self.reload_and_notify(cx);
                }
            }
            // The interface zoom changed elsewhere (an import, a reset): every
            // window takes it (D96).
            CoreEventKind::SettingsChanged { keys } => {
                if keys.iter().any(|key| key == crate::theme::zoom::SETTING)
                    && let Some(core) = self.core.clone()
                {
                    crate::theme::zoom::apply_stored(&core, cx);
                }
            }
            // TODO(shell): feed NotificationsChanged into the notification
            // center and toasts (UI 1.5).
            CoreEventKind::NotificationsChanged { .. } => {}
            CoreEventKind::SyncChanged {} => self.refresh_sync_status(cx),
        }
    }

    /// Re-reads the lock state; loads or drops the records on a change.
    fn refresh_lock(&mut self, cx: &mut Context<Self>) {
        let Some(core) = &self.core else {
            return;
        };
        let state = core.lock_status().state;
        if state == self.lock {
            return;
        }
        self.lock = state;
        if state == LockState::Unlocked {
            self.load_unlocked();
        } else {
            self.vault_id = None;
            self.hosts = Rc::default();
            self.groups = Rc::default();
            self.workspaces = Rc::default();
            // A lock keeps the place (open group, selection): after unlock the
            // window returns to it (UI 13.3); the next reload drops ids that
            // no longer exist. A new vault starts at the top level.
            if state == LockState::FirstRun {
                self.location = None;
                self.selection.clear();
            }
            // The core wiped its undo buffer with the session.
            self.removals.clear();
        }
        tracing::info!(state = ?state, "lock state changed");
        self.restart_retention(cx);
        self.restart_sync(cx);
        self.records_generation += 1;
        cx.emit(AppStateEvent::LockChanged(state));
        cx.emit(AppStateEvent::RecordsChanged);
        cx.notify();
    }

    fn load_unlocked(&mut self) {
        self.vault_id = self.core.as_ref().and_then(|core| {
            core.vaults().ok().and_then(|vaults| {
                vaults
                    .into_iter()
                    .find(|vault| vault.kind == VaultKind::Personal)
                    .map(|vault| vault.vault_id)
            })
        });
        self.reload();
    }

    /// Reloads every host and group of the personal vault.
    fn reload(&mut self) {
        let Some(core) = &self.core else {
            return;
        };
        let views = core.records_query(RecordsQuery {
            vault_id: self.vault_id,
            types: vec![RecordType::Group, RecordType::Host],
            scope: QueryScope::All,
            text: None,
            tags: Vec::new(),
            sort: SortMode::Manual,
        });
        match views {
            Ok(views) => {
                let (hosts, groups) = model::split_views(&views);
                self.hosts = Rc::new(hosts);
                self.groups = Rc::new(groups);
            }
            Err(error) => {
                tracing::warn!(code = error.code(), "records could not be loaded");
                self.hosts = Rc::default();
                self.groups = Rc::default();
            }
        }
        self.workspaces = Rc::new(
            self.vault_id
                .and_then(|vault_id| core.workspace_summaries(vault_id).ok())
                .unwrap_or_default()
                .into_iter()
                .map(|summary| WorkspaceItem {
                    id: summary.record_id,
                    name: SharedString::from(summary.name),
                    missing: summary
                        .missing_hosts
                        .into_iter()
                        .map(SharedString::from)
                        .collect(),
                })
                .collect(),
        );
        if self.location.is_some_and(|id| self.group(id).is_none()) {
            self.location = None;
        }
        let hosts = self.hosts.clone();
        let groups = self.groups.clone();
        self.selection.retain(|id| {
            hosts.iter().any(|host| host.record_id == *id)
                || groups.iter().any(|group| group.record_id == *id)
        });
    }
}

#[cfg(test)]
mod tests;
