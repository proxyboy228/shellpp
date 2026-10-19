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
//! [`AppState::duplicate_record`]. Each runs synchronously (one record:
//! encryption plus one SQLite write), reloads the lists and emits
//! [`AppStateEvent::RecordsChanged`]. Validation errors come back as
//! `CoreError::Validation` with the failing fields.
//!
//! Lock: [`AppState::first_run_setup`] and [`AppState::unlock`] run Argon2id
//! on the background executor and return a task; [`AppState::lock`] is
//! immediate. While not unlocked the lists are empty.
//!
//! Core events (records written elsewhere, auto-lock from the tick) arrive
//! through [`crate::platform::events`] and trigger the same reload.

pub mod model;
pub mod selection;

use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt as _;
use gpui::{AppContext as _, Context, EventEmitter, Task};
use secrecy::SecretString;
use serde_json::{Map, Value};
use shellpp_core::service::dto::{
    CoreEvent, CoreEventKind, LockState, QueryScope, RecordDeleteResult, RecordMoveInput,
    RecordRef, RecordSaveInput, RecordType, RecordView, RecordsQuery, RecordsRef,
    RecoveryKeyResult, SecretCopyTarget, SortMode, VaultKind,
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
    /// The group whose contents the Hosts screen shows; `None` is the top level.
    location: Option<Uuid>,
    selection: Selection,
    extras: PlatformExtras,
    dirs: Option<AppDirs>,
    shut_down: bool,
    _event_task: Option<Task<()>>,
    tick_task: Option<Task<()>>,
    temp_profile: Option<TempProfile>,
}

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
            location: None,
            selection: Selection::default(),
            extras,
            dirs,
            shut_down: false,
            _event_task: Some(event_task),
            tick_task: None,
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
            location: None,
            selection: Selection::default(),
            extras: PlatformExtras::default(),
            dirs: None,
            shut_down: false,
            _event_task: None,
            tick_task: None,
            temp_profile: None,
        }
    }

    /// Calls `core.tick()` about once per second (auto-lock, clipboard
    /// clearing). The app starts it; tests usually do not.
    pub fn start_tick(&mut self, cx: &mut Context<Self>) {
        let Some(core) = self.core.clone() else {
            return;
        };
        let executor = cx.background_executor().clone();
        self.tick_task = Some(cx.background_spawn(async move {
            loop {
                executor.timer(Duration::from_secs(1)).await;
                core.tick();
            }
        }));
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

    /// Every host of the personal vault, in manual order.
    pub fn hosts(&self) -> Rc<Vec<HostItem>> {
        self.hosts.clone()
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
        match self.save(record_id, RecordType::Host, data, cx)? {
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
        match self.save(record_id, RecordType::Group, data, cx)? {
            RecordView::Group(view) => Ok(GroupItem::from_view(&view)),
            RecordView::Host(_) => Err(CoreError::NotFound),
        }
    }

    fn save(
        &mut self,
        record_id: Option<Uuid>,
        record_type: RecordType,
        data: Map<String, Value>,
        cx: &mut Context<Self>,
    ) -> Result<RecordView, CoreError> {
        let vault_id = self.require_vault()?;
        let view = self.require_core()?.record_save(RecordSaveInput {
            vault_id,
            record_id,
            record_type,
            data,
            secrets: Default::default(),
        })?;
        self.records_written(cx);
        Ok(view)
    }

    /// Deletes hosts and groups (a removed group's children move one level up).
    pub fn delete_records(
        &mut self,
        record_ids: Vec<Uuid>,
        cx: &mut Context<Self>,
    ) -> Result<RecordDeleteResult, CoreError> {
        let vault_id = self.require_vault()?;
        let result = self.require_core()?.record_delete(RecordsRef {
            vault_id,
            record_ids,
        })?;
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

    fn records_written(&mut self, cx: &mut Context<Self>) {
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

    /// First run (UI 13.1): sets the master password, unlocks and returns the
    /// recovery key. Argon2id runs on the background executor.
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

    /// Copies the recovery key shown at first run (the core clears the
    /// clipboard after 30 seconds; allowed for 5 minutes after the first run).
    pub fn copy_recovery_key(&self) -> Result<(), CoreError> {
        self.require_core()?
            .secret_copy(SecretCopyTarget::RecoveryKey)
    }

    /// Reports user activity to the auto-lock timer.
    pub fn activity(&self) {
        if let Some(core) = &self.core {
            core.activity();
        }
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
            CoreEventKind::RecordsChanged { .. } | CoreEventKind::VaultsChanged { .. } => {
                if self.lock == LockState::Unlocked {
                    self.reload_and_notify(cx);
                }
            }
            // TODO(shell): feed NotificationsChanged into the notification
            // center and toasts (UI 1.5); settings arrive with the Settings window.
            CoreEventKind::SettingsChanged { .. } | CoreEventKind::NotificationsChanged { .. } => {}
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
            self.location = None;
            self.selection.clear();
        }
        tracing::info!(state = ?state, "lock state changed");
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
