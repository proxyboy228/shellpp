//! The core facade used by the desktop host (and later by mobile hosts).
//!
//! [`Core`] owns the store, the lock state, the key ring, the index, settings
//! and notifications. It is `Send + Sync`; every method takes `&self` and
//! serializes access internally. Methods that run Argon2id or decrypt many
//! records (`first_run_setup`, `unlock`, `recovery_unlock`, `password_change`,
//! `recovery_key_show`, `recovery_key_reissue`, `try_device_unlock`) block for
//! up to a few seconds: the host calls them from `spawn_blocking`.
//!
//! Lock guard (docs/architecture.md, section 3.3): methods that need keys
//! return [`CoreError::Locked`] unless the state is `unlocked`. Every write
//! emits its broadcast event through [`crate::platform::EventSink`] after the
//! change is committed.
//!
//! Concurrency: one mutex guards the state. Argon2id and `SecretStore` calls
//! run without holding it; the state is checked again after re-acquiring it.
//! Events are emitted while the mutex is held, right after the commit, with
//! `v` incremented under the same mutex, so their order equals their `v`
//! order and [`Core::snapshot`] never misses or double-counts one. An
//! [`crate::platform::EventSink`] must therefore not call back into the core.
//!
//! Owner: W2. The signatures are the W2/W3 contract written by the lead.

pub mod dto;
mod lock_ops;
mod records;
mod session;

use std::fmt;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};
use std::time::SystemTime;

use secrecy::SecretString;
use uuid::Uuid;
use zeroize::Zeroize;

use crate::error::CoreError;
use crate::lock::{AutoLock, ClipboardGuard, RecoveryGrant};
use crate::model::time::{IdGenerator, format_timestamp};
use crate::notify::NotificationCenter;
use crate::platform::{Availability, Platform};
use crate::settings::Settings;
use crate::store::{OpenOutcome, Store, StoreError, WrapKind};
use dto::{
    CoreEvent, CoreEventKind, CoreSnapshot, LockState, LockStatus, NewNotification, Notification,
    NotificationList, RecordDeleteResult, RecordMoveInput, RecordRef, RecordSaveInput, RecordView,
    RecordsQuery, RecordsRef, RecoveryKeyResult, SecretCopyTarget, SecretFieldRef,
    SettingsSetInput, SettingsValues, VaultView,
};
use session::Session;

/// Startup configuration resolved by the host (the core never looks up OS directories).
#[derive(Clone, Debug)]
pub struct CoreConfig {
    /// The data directory (docs/architecture.md, section 5.3); created if missing.
    pub data_dir: PathBuf,
    /// Semver of the running app, written to `meta.last_app_version` and `device_info`.
    pub app_version: String,
}

/// Everything behind the core mutex.
struct Inner {
    store: Store,
    state: LockState,
    /// `Some` exactly while unlocked.
    session: Option<Session>,
    /// From `local_auth`; `None` at first run.
    personal_vault_id: Option<Uuid>,
    settings: Settings,
    notifications: NotificationCenter,
    clipboard_guard: Option<ClipboardGuard>,
    auto_lock: AutoLock,
    recovery_grant: RecoveryGrant,
    /// State version of the last emitted event.
    v: u64,
    /// The device key is used at launch only, never after a lock (UI 13.2).
    device_unlock_allowed: bool,
    ids: IdGenerator,
}

/// The core facade.
pub struct Core {
    platform: Platform,
    config: CoreConfig,
    inner: Mutex<Inner>,
    /// Serializes the operations that write the device key: "remember on this
    /// device" (keychain item and `device` row) and the rotations that rewrap
    /// the `device` row (recovery unlock, recovery key reissue). Held across
    /// the keychain calls, which run without `inner`; always taken before
    /// `inner`, never while holding it.
    remember: Mutex<()>,
}

impl fmt::Debug for Core {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Core").finish_non_exhaustive()
    }
}

fn open_error(error: StoreError) -> CoreError {
    match error {
        StoreError::NewerSchema { found } => CoreError::NewerSchema { found },
        other => CoreError::Store(other),
    }
}

impl Core {
    /// Opens (or creates) `shellpp.db` in the data directory and runs pending
    /// migrations (backup first). The state is `first_run` without a
    /// `local_auth` row, otherwise `locked`. Does not unlock.
    pub fn open(config: CoreConfig, platform: Platform) -> Result<Core, CoreError> {
        let (store, outcome) =
            Store::open(&config.data_dir, &config.app_version).map_err(open_error)?;
        match &outcome {
            OpenOutcome::Created => tracing::info!("database created"),
            OpenOutcome::Opened => tracing::info!("database opened"),
            OpenOutcome::Migrated { from, .. } => tracing::info!(from, "database migrated"),
        }
        let local_auth = store.local_auth()?;
        let state = if local_auth.is_some() {
            LockState::Locked
        } else {
            LockState::FirstRun
        };
        let settings = Settings::new(platform.os, &store.settings()?);
        let now = platform.clock.now();
        let inner = Inner {
            store,
            state,
            session: None,
            personal_vault_id: local_auth.map(|row| row.personal_vault_id),
            settings,
            notifications: NotificationCenter::new(),
            clipboard_guard: None,
            auto_lock: AutoLock::new(now),
            recovery_grant: RecoveryGrant::default(),
            v: 0,
            device_unlock_allowed: true,
            ids: IdGenerator::new(),
        };
        Ok(Core {
            platform,
            config,
            inner: Mutex::new(inner),
            remember: Mutex::new(()),
        })
    }

    fn inner(&self) -> MutexGuard<'_, Inner> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// See [`Core::remember`]. Take it before [`Core::inner`].
    fn remember_lock(&self) -> MutexGuard<'_, ()> {
        self.remember
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn now(&self) -> SystemTime {
        self.platform.clock.now()
    }

    /// Emits a broadcast event with the next state version. Called with the
    /// mutex held, after the change is committed.
    fn emit(&self, inner: &mut Inner, kind: CoreEventKind) {
        self.emit_with(&mut inner.v, kind);
    }

    /// [`Core::emit`] for callers that hold the fields of `Inner` separately.
    fn emit_with(&self, v: &mut u64, kind: CoreEventKind) {
        *v += 1;
        let event = CoreEvent { v: *v, kind };
        tracing::debug!(event = event.name(), v = event.v, "event");
        self.platform.events.emit(&event);
    }

    /// `true` when an OS keychain is usable. Called without the mutex.
    fn remember_available(&self) -> bool {
        self.platform.secret_store.availability() == Availability::Available
    }

    fn status(&self, inner: &Inner, available: bool) -> LockStatus {
        LockStatus {
            state: inner.state,
            remember_device_available: available,
            remember_device_enabled: device_row_exists(inner),
        }
    }

    fn require_session(inner: &Inner) -> Result<&Session, CoreError> {
        match (&inner.state, &inner.session) {
            (LockState::Unlocked, Some(session)) => Ok(session),
            _ => Err(CoreError::Locked),
        }
    }

    /// Clears the clipboard if it still holds the value `secret_copy` put there.
    ///
    /// The guard is dropped only after a decision: when the clipboard cannot
    /// be read or cleared (on Windows another program may hold it open), it
    /// is kept and every [`Core::tick`] tries again, for up to
    /// [`crate::lock::CLIPBOARD_RETRY_FOR`] (docs/security.md, section 9.4).
    fn clear_clipboard_if_ours(&self, inner: &mut Inner) {
        let Some(guard) = inner.clipboard_guard else {
            return;
        };
        let cleared = match self.platform.clipboard.read_text() {
            Ok(Some(mut text)) => {
                let ours = guard.matches(&text);
                text.zeroize();
                !ours || self.platform.clipboard.clear().is_ok()
            }
            Ok(None) => true,
            Err(_) => false,
        };
        inner.clipboard_guard = if cleared {
            None
        } else {
            let retry = guard.after_failure(self.now());
            if retry.is_some() {
                tracing::warn!("clipboard could not be cleared; retrying");
            } else {
                tracing::warn!("clipboard could not be cleared; giving up");
            }
            retry
        };
    }

    // ----- lock -------------------------------------------------------------

    pub fn lock_status(&self) -> LockStatus {
        let available = self.remember_available();
        let inner = self.inner();
        self.status(&inner, available)
    }

    pub fn is_unlocked(&self) -> bool {
        self.inner().state == LockState::Unlocked
    }

    /// State for a new or reloaded window. `v` is read under the same lock as the data.
    pub fn snapshot(&self) -> CoreSnapshot {
        let available = self.remember_available();
        let inner = self.inner();
        let vaults = Self::require_session(&inner)
            .map(|session| vec![records::vault_view(session)])
            .unwrap_or_default();
        CoreSnapshot {
            v: inner.v,
            lock: self.status(&inner, available),
            settings: inner.settings.values(),
            notifications: inner.notifications.list(),
            vaults,
        }
    }

    /// Locks when unlocked and the auto-lock interval has passed since the
    /// last input. Returns `true` when it locked.
    fn auto_lock_if_due(&self, inner: &mut Inner, now: SystemTime) -> bool {
        if inner.state != LockState::Unlocked {
            return false;
        }
        let interval = inner.settings.auto_lock_interval();
        if !inner.auto_lock.due(now, interval) {
            return false;
        }
        tracing::info!("auto-lock");
        self.lock_now(inner);
        true
    }

    /// Keyboard or mouse input in an app window (reported at most every 30 s).
    ///
    /// A lock that is already due wins over the input: after a sleep longer
    /// than the interval, the first input that reaches the core before the
    /// next [`Core::tick`] locks instead of restarting the timer
    /// (docs/security.md, section 8).
    pub fn activity(&self) {
        let now = self.now();
        let mut guard = self.inner();
        let inner = &mut *guard;
        if self.auto_lock_if_due(inner, now) {
            return;
        }
        inner.auto_lock.activity(now);
    }

    /// Called by the host about once per second: auto-lock check and the
    /// 30-second clearing of sensitive clipboard content.
    pub fn tick(&self) {
        let now = self.now();
        let mut guard = self.inner();
        let inner = &mut *guard;
        if let Some(session) = inner.session.as_mut() {
            session.undo.purge(now);
        }
        self.auto_lock_if_due(inner, now);
        if inner
            .clipboard_guard
            .is_some_and(|guard| guard.expired(now))
        {
            self.clear_clipboard_if_ours(inner);
        }
    }

    /// Called once when the app exits: clears sensitive clipboard content and
    /// moves the WAL into the database file (the host ends the process
    /// without dropping the core, so SQLite never checkpoints on close).
    pub fn on_exit(&self) {
        let mut inner = self.inner();
        self.clear_clipboard_if_ours(&mut inner);
        checkpoint(&inner.store);
    }

    // ----- settings and notifications ----------------------------------------

    /// Readable in every lock state.
    pub fn settings_get(&self) -> Result<SettingsValues, CoreError> {
        Ok(self.inner().settings.values())
    }

    pub fn settings_set(&self, input: SettingsSetInput) -> Result<SettingsValues, CoreError> {
        let mut guard = self.inner();
        Self::require_session(&guard)?;
        let update = guard.settings.prepare(&input)?;
        let rows = update.put_rows();
        guard.store.write(|tx| {
            for (key, json) in &rows {
                tx.setting_put(key, json)?;
            }
            for key in &update.deletes {
                tx.setting_delete(key)?;
            }
            Ok(())
        })?;
        let changed = guard.settings.apply(&update);
        if !changed.is_empty() {
            self.emit(&mut guard, CoreEventKind::SettingsChanged { keys: changed });
        }
        Ok(guard.settings.values())
    }

    pub fn notifications_list(&self) -> NotificationList {
        self.inner().notifications.list()
    }

    pub fn notifications_mark_read(&self) -> NotificationList {
        let mut guard = self.inner();
        if guard.notifications.mark_all_read() {
            let unread = guard.notifications.unread();
            self.emit(
                &mut guard,
                CoreEventKind::NotificationsChanged {
                    unread,
                    latest: None,
                },
            );
        }
        guard.notifications.list()
    }

    /// Adds a notification and emits `notifications:changed`.
    pub fn notify(&self, notification: NewNotification) -> Notification {
        let created_at = format_timestamp(self.now());
        let mut guard = self.inner();
        let added = guard.notifications.add(notification, created_at);
        let unread = guard.notifications.unread();
        self.emit(
            &mut guard,
            CoreEventKind::NotificationsChanged {
                unread,
                latest: Some(added.clone()),
            },
        );
        added
    }

    // ----- records (bodies in `records.rs`) ----------------------------------

    pub fn vaults(&self) -> Result<Vec<VaultView>, CoreError> {
        self.vaults_impl()
    }

    pub fn records_query(&self, query: RecordsQuery) -> Result<Vec<RecordView>, CoreError> {
        self.records_query_impl(query)
    }

    pub fn record_get(&self, record: RecordRef) -> Result<RecordView, CoreError> {
        self.record_get_impl(record)
    }

    pub fn record_save(&self, input: RecordSaveInput) -> Result<RecordView, CoreError> {
        self.record_save_impl(input)
    }

    pub fn record_delete(&self, input: RecordsRef) -> Result<RecordDeleteResult, CoreError> {
        self.record_delete_impl(input)
    }

    /// Undo of a removal (UI 2.4): writes the last live data back under the
    /// same record ids. `not_found` when the undo window (60 s) has passed.
    pub fn record_restore(&self, input: RecordsRef) -> Result<Vec<RecordView>, CoreError> {
        self.record_restore_impl(input)
    }

    pub fn record_move(&self, input: RecordMoveInput) -> Result<Vec<RecordView>, CoreError> {
        self.record_move_impl(input)
    }

    /// Duplicate (UI 2.4, docs/data-model.md, section 7.2). Hosts only in stage 1.
    pub fn record_duplicate(&self, record: RecordRef) -> Result<RecordView, CoreError> {
        self.record_duplicate_impl(record)
    }

    pub fn secret_reveal(&self, field: SecretFieldRef) -> Result<SecretString, CoreError> {
        self.secret_reveal_impl(field)
    }

    pub fn secret_copy(&self, target: SecretCopyTarget) -> Result<(), CoreError> {
        self.secret_copy_impl(target)
    }

    // ----- lock (bodies in `lock_ops.rs`) -------------------------------------

    /// At launch only: unlock with the device key when "remember on this
    /// device" is on. `Ok(false)` (and the state stays `locked`) when the
    /// option is off or anything fails (missing keychain item, failed unwrap).
    pub fn try_device_unlock(&self) -> Result<bool, CoreError> {
        self.try_device_unlock_impl()
    }

    /// First run (UI 13.1): creates ids, the personal vault, wraps and the
    /// records `vault_meta`, `recovery_key`, `device_info`; leaves the app
    /// unlocked; returns the recovery key text. `wrong_state` unless `first_run`;
    /// `validation_failed` (`password`, `too_short`) under 10 characters.
    pub fn first_run_setup(&self, password: SecretString) -> Result<RecoveryKeyResult, CoreError> {
        self.first_run_setup_impl(password)
    }

    /// `invalid_password` on a failed unwrap; `wrong_state` unless `locked`.
    pub fn unlock(&self, password: SecretString) -> Result<LockStatus, CoreError> {
        self.unlock_impl(password)
    }

    /// Wipes keys, index and sensitive clipboard content. Idempotent while locked.
    pub fn lock(&self) -> Result<LockStatus, CoreError> {
        self.lock_impl()
    }

    /// UI 13.2 "Use recovery key", local mode: opens the `recovery` wrap, sets
    /// the new password, rotates the personal vault, re-seals records, unlocks.
    pub fn recovery_unlock(
        &self,
        recovery_key: SecretString,
        new_password: SecretString,
    ) -> Result<LockStatus, CoreError> {
        self.recovery_unlock_impl(recovery_key, new_password)
    }

    /// UI 13.4: new salt and current preset, rewrap of the current personal vault key.
    pub fn password_change(
        &self,
        current: SecretString,
        new: SecretString,
    ) -> Result<(), CoreError> {
        self.password_change_impl(current, new)
    }

    /// UI 13.4 "Show": needs the master password even while unlocked.
    pub fn recovery_key_show(
        &self,
        password: SecretString,
    ) -> Result<RecoveryKeyResult, CoreError> {
        self.recovery_key_show_impl(password)
    }

    /// UI 13.4 "Reissue": new recovery key, personal vault rotation, new
    /// revision of the `recovery_key` record. The old key stops working.
    pub fn recovery_key_reissue(
        &self,
        password: SecretString,
    ) -> Result<RecoveryKeyResult, CoreError> {
        self.recovery_key_reissue_impl(password)
    }

    /// UI 13.4 "Remember on this device". `unavailable` without an OS keychain.
    pub fn remember_device_set(&self, enabled: bool) -> Result<LockStatus, CoreError> {
        self.remember_device_set_impl(enabled)
    }
}

/// Copies the WAL into the database file and truncates it, so key wraps that
/// were replaced or deleted do not stay in `shellpp.db-wal`
/// (docs/storage-format.md, section 4.1). Best effort: called after a commit,
/// a failure is logged and the next checkpoint catches up.
fn checkpoint(store: &Store) {
    match store.checkpoint_truncate() {
        Ok(true) => {}
        Ok(false) => tracing::warn!("wal checkpoint did not complete"),
        Err(error) => tracing::warn!(code = error.code(), "wal checkpoint failed"),
    }
}

/// A `device` row exists for the current personal epoch.
fn device_row_exists(inner: &Inner) -> bool {
    let Some(vault_id) = inner.personal_vault_id else {
        return false;
    };
    let epoch = match &inner.session {
        Some(session) => session.ring.current_epoch(),
        None => match inner.store.vault(&vault_id) {
            Ok(Some(row)) => row.current_epoch,
            _ => return false,
        },
    };
    matches!(
        inner.store.key_wrap(&vault_id, epoch, WrapKind::Device),
        Ok(Some(_))
    )
}

#[cfg(test)]
mod tests {
    use super::Core;

    #[test]
    fn core_is_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<Core>();
    }
}
