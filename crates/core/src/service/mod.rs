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
//! Owner: W2 (stage 1), S4 (stage 2). The signatures are the contract
//! written by the lead; the stage 2 methods are listed at the end of the
//! `impl` block (bodies in `connect.rs`, stage 2 plan, section 4.4).

mod connect;
pub mod dto;
mod export;
pub mod host_api;
pub mod keychain;
mod known_hosts_io;
/// The last session of Restore (UI 6; stage 2b).
pub mod last_session;
mod lock_ops;
mod records;
mod session;

use std::fmt;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, SystemTime};

use secrecy::SecretString;
use uuid::Uuid;
use zeroize::Zeroize;

use crate::error::CoreError;
use crate::lock::lockout::META_UNLOCK_LOCKOUT;
use crate::lock::{AutoLock, ClipboardGuard, FirstRunOptions, Lockout, RecoveryGrant};
use crate::model::time::{IdGenerator, format_timestamp};
use crate::notify::NotificationCenter;
use crate::os_detect::DetectedOs;
use crate::platform::{Availability, Platform};
use crate::settings::Settings;
use crate::store::{OpenOutcome, Store, StoreError, WrapKind};
use dto::{
    CoreEvent, CoreEventKind, CoreSnapshot, DataExport, DataExportInput, DataImportInput,
    DataImportSummary, HistoryClearInput, HistoryClearResult, HistoryEntryView, HistoryListInput,
    KeyGenerateInput, KeyImportInput, KeyImportResult, KeyView, KnownHostAddInput, KnownHostView,
    KnownHostsImportSummary, LockState, LockStatus, NewNotification, Notification,
    NotificationList, OsSource, RecentConnection, RecordDeleteResult, RecordMoveInput, RecordRef,
    RecordSaveInput, RecordView, RecordsQuery, RecordsRef, RecoveryKeyResult, SecretCopyTarget,
    SecretFieldRef, SessionTarget, SettingsSetInput, SettingsValues, VaultView,
};
use host_api::{
    ConnectionEndReason, ConnectionLogStart, HistoryDecision, HistoryRecordInput, HostKeyCheck,
    HostKeyStatus, SessionConfig,
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
    /// Wrong master passwords in a row and the current pause (D29); kept in
    /// `meta` across restarts.
    lockout: Lockout,
    recovery_grant: RecoveryGrant,
    /// State version of the last emitted event.
    v: u64,
    /// The device key is used at launch only, never after a lock (UI 13.2).
    device_unlock_allowed: bool,
    ids: IdGenerator,
    /// Stage 2: connection log entries of this process and ends waiting for
    /// unlock (survives locks; `connect.rs`).
    connlog: crate::connlog::Tracker,
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
        let lockout = Lockout::from_meta(store.meta_get(META_UNLOCK_LOCKOUT)?.as_deref());
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
            lockout,
            recovery_grant: RecoveryGrant::default(),
            v: 0,
            device_unlock_allowed: true,
            ids: IdGenerator::new(),
            connlog: crate::connlog::Tracker::new(),
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
        // The guard stays in `inner` and is dropped there, so its hash is
        // wiped in place (see `ClipboardGuard`).
        let Some(guard) = inner.clipboard_guard.as_mut() else {
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
        if cleared {
            inner.clipboard_guard = None;
        } else if guard.record_failure(self.now()) {
            tracing::warn!("clipboard could not be cleared; retrying");
        } else {
            tracing::warn!("clipboard could not be cleared; giving up");
            inner.clipboard_guard = None;
        }
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

    /// `true` when unlocked and the auto-lock interval has passed since the
    /// last input: [`Core::tick`] and [`Core::activity`] would lock now. Only
    /// a check: a host that saves pending edits before locking asks this
    /// first, saves, then calls [`Core::lock`]; the tick stays the backstop.
    pub fn auto_lock_due(&self) -> bool {
        let now = self.now();
        let mut inner = self.inner();
        let interval = inner.settings.auto_lock_interval();
        inner.state == LockState::Unlocked && inner.auto_lock.due(now, interval)
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
            .as_ref()
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
    /// `validation_failed` (`password`, `too_short`) under 8 characters.
    pub fn first_run_setup(&self, password: SecretString) -> Result<RecoveryKeyResult, CoreError> {
        self.first_run_setup_impl(password)
    }

    /// UI 13.1 as the app runs it (D29): the first run, then "Remember on
    /// this device" when `options` ask for it and an OS keychain is there
    /// (a keychain failure only leaves the option off). The recovery key is
    /// created and kept in the vault but not returned: local mode never shows
    /// it. Errors as [`Core::first_run_setup`].
    pub fn first_run_setup_with(
        &self,
        password: SecretString,
        options: FirstRunOptions,
    ) -> Result<LockStatus, CoreError> {
        self.first_run_setup_with_impl(password, options)
    }

    /// `invalid_password` on a failed unwrap; `wrong_state` unless `locked`;
    /// `too_many_attempts` during a pause after 5 wrong passwords in a row
    /// (D29), checked before any key derivation.
    pub fn unlock(&self, password: SecretString) -> Result<LockStatus, CoreError> {
        self.unlock_impl(password)
    }

    /// The time left before the next unlock attempt is accepted; `None` when
    /// an attempt is allowed now (D29).
    pub fn unlock_retry_after(&self) -> Option<Duration> {
        let now = self.now();
        self.inner().lockout.remaining(now)
    }

    /// Wrong master passwords since the last successful unlock (D29).
    pub fn unlock_failures(&self) -> u32 {
        self.inner().lockout.failures()
    }

    /// "Forgot password?" -> "Start with a new vault" (UI 13.2, D29): only
    /// while `locked`. Closes the database, renames `shellpp.db` (and its
    /// `-wal` / `-shm`) to `shellpp.db.<UTC yyyymmddThhmmssZ>.old`, never
    /// deleting it, opens a new empty database and returns to `first_run`
    /// (settings and the attempt counter start over). Returns the archive's
    /// file name. On failure the old database stays in place and open.
    pub fn start_new_vault(&self) -> Result<String, CoreError> {
        self.start_new_vault_impl()
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

    // ----- stage 2: sessions (bodies in `connect.rs`) ------------------------
    //
    // Every method needs the unlocked state (`locked` otherwise), except
    // `connection_log_end`, which holds the end in memory while locked.

    /// Effective settings of a new session (docs/data-model.md, sections 8.3
    /// and 8.5). `not_found` for a missing host; a quick-connect target is
    /// validated like a host address and username (`validation_failed` with
    /// fields `address`, `port`, `username`).
    pub fn session_config(&self, target: &SessionTarget) -> Result<SessionConfig, CoreError> {
        self.session_config_impl(target)
    }

    /// Looks up a presented host key in known hosts (UI 10.2).
    pub fn known_host_check(&self, check: &HostKeyCheck) -> Result<HostKeyStatus, CoreError> {
        self.known_host_check_impl(check)
    }

    /// "Accept and save": a new `known_host` record with `source = tofu`
    /// (plain `host` = the address in lowercase, `port` = the port).
    pub fn known_host_accept(&self, check: &HostKeyCheck) -> Result<KnownHostView, CoreError> {
        self.known_host_accept_impl(check)
    }

    /// "Replace saved key": updates `key_type` and `public_key` of the record
    /// in place and sets `source = replaced` when the record is a plain or
    /// hashed entry for exactly this address and port. A wildcard pattern
    /// (or an entry for another host) is never rewritten: a new plain entry
    /// for the address and port gets the key (`source = replaced`), so the
    /// trust of the other hosts the pattern covers does not change.
    pub fn known_host_replace(
        &self,
        record_id: Uuid,
        check: &HostKeyCheck,
    ) -> Result<KnownHostView, CoreError> {
        self.known_host_replace_impl(record_id, check)
    }

    /// Adds an entry as from a known_hosts line (`source = import`): the e2e
    /// mock action `known_host_add` now, the known_hosts import in stage 3.
    /// An entry equal to an existing one (natural key) returns the existing one.
    pub fn known_host_add(&self, input: KnownHostAddInput) -> Result<KnownHostView, CoreError> {
        self.known_host_add_impl(input)
    }

    /// Every known host entry, sorted by display address (hashed entries last).
    pub fn known_hosts_list(&self) -> Result<Vec<KnownHostView>, CoreError> {
        self.known_hosts_list_impl()
    }

    /// Removes known host entries (UI 10.1 Remove). The removal can be undone
    /// with [`Core::known_host_restore`] for 60 s.
    pub fn known_host_remove(&self, input: RecordsRef) -> Result<(), CoreError> {
        self.known_host_remove_impl(input)
    }

    /// Undo of [`Core::known_host_remove`] (UI 10.1, 2.4): the entries come
    /// back under their record ids. `not_found` once the undo window (60 s)
    /// has passed or for an id that was not a removed known host.
    pub fn known_host_restore(&self, input: RecordsRef) -> Result<Vec<KnownHostView>, CoreError> {
        self.known_host_restore_impl(input)
    }

    /// Imports a known_hosts file in the OpenSSH format (UI 10.1, D21;
    /// `crate::known_hosts::openssh`): one entry per host pattern, hashed
    /// entries as they are, `@revoked` as revoked keys, `@cert-authority`
    /// lines and negated patterns skipped. Entries equal to existing ones
    /// are duplicates; nothing existing is changed. `validation_failed`
    /// (`file`, `too_long`) for more than 16 MiB.
    pub fn known_hosts_import(&self, input: &[u8]) -> Result<KnownHostsImportSummary, CoreError> {
        self.known_hosts_import_impl(input)
    }

    /// Every entry as a known_hosts file (docs/storage-format.md, section
    /// 13.6): hashed entries as stored, revoked ones with `@revoked`.
    pub fn known_hosts_export(&self) -> Result<String, CoreError> {
        self.known_hosts_export_impl()
    }

    /// Every key of the personal vault, by label.
    pub fn keys_list(&self) -> Result<Vec<KeyView>, CoreError> {
        self.keys_list_impl()
    }

    /// "+ -> Key" and the Keychain's New key: parses and stores a private
    /// key (rules and errors: stage 2 plan, sections 4.4 and 5.1). OpenSSH,
    /// PEM (PKCS#1, PKCS#8, SEC1) and PuTTY `.ppk` keys are accepted; the
    /// stored form is OpenSSH (`crate::keys::prepare`).
    pub fn key_import(&self, input: KeyImportInput) -> Result<KeyImportResult, CoreError> {
        self.key_import_impl(input)
    }

    /// Export data (SPEC principle 4; docs/storage-format.md, sections 13.2
    /// and 13.3; D34): every record of the personal vault but the recovery
    /// key, and the app settings, as plain JSON (secrets in clear text; the
    /// UI warns first) or encrypted with `password`. Slow when encrypted
    /// (Argon2id): call it off the UI thread.
    pub fn data_export(&self, input: DataExportInput) -> Result<DataExport, CoreError> {
        self.data_export_impl(input)
    }

    /// Import data from a plain or encrypted export (D34): merged by record
    /// id; a record whose id exists with other content is kept as a copy
    /// with an " (imported)" suffix; existing records are never
    /// overwritten. Errors: `text` (`invalid`, `too_long`, `too_many`,
    /// `not_allowed` for refused KDF parameters), `password` (`required`,
    /// `invalid`). Slow when encrypted: call it off the UI thread.
    pub fn data_import(&self, input: DataImportInput) -> Result<DataImportSummary, CoreError> {
        self.data_import_impl(input)
    }

    /// Generate key (UI 7.1): a new key pair stored like an import. The
    /// label is required (`label` `required`, `too_long`). Slow for RSA:
    /// call it off the UI thread.
    pub fn key_generate(&self, input: KeyGenerateInput) -> Result<KeyImportResult, CoreError> {
        let label =
            match crate::model::text::string(&input.label, crate::model::key::MAX_LABEL_CHARS) {
                Ok(Some(label)) => label,
                Ok(None) => {
                    return Err(CoreError::Validation(vec![crate::error::FieldError::new(
                        "label",
                        crate::error::ValidationReason::Required,
                    )]));
                }
                Err(reason) => {
                    return Err(CoreError::Validation(vec![crate::error::FieldError::new(
                        "label", reason,
                    )]));
                }
            };
        {
            let inner = self.inner();
            let session = Self::require_session(&inner)?;
            records::check_vault(session, Some(input.vault_id))?;
        }
        let text = crate::keys::generate::generate(input.kind, &label, input.passphrase.as_ref())
            .map_err(CoreError::Crypto)?;
        self.key_import_impl(KeyImportInput {
            vault_id: input.vault_id,
            label: Some(label),
            private_key: SecretString::from(text.as_str().to_owned()),
            passphrase: input.passphrase,
            save_passphrase: input.save_passphrase,
        })
    }

    /// A password typed at a prompt with "Save" checked: `host.ssh.password`
    /// of a personal-vault host (docs/data-model.md, section 7.3).
    pub fn save_prompted_password(
        &self,
        host: RecordRef,
        password: SecretString,
    ) -> Result<(), CoreError> {
        self.save_prompted_password_impl(host, password)
    }

    /// A passphrase typed at a prompt with "Save" checked: `key.passphrase`.
    pub fn save_key_passphrase(
        &self,
        key: RecordRef,
        passphrase: SecretString,
    ) -> Result<(), CoreError> {
        self.save_key_passphrase_impl(key, passphrase)
    }

    /// Writes `host.os` when it is absent or the family or distro differs
    /// (docs/data-model.md, section 7.6). Returns `true` when it wrote.
    pub fn host_os_detected(
        &self,
        host: RecordRef,
        os: DetectedOs,
        source: OsSource,
    ) -> Result<bool, CoreError> {
        self.host_os_detected_impl(host, os, source)
    }

    /// Applies rule 4 of docs/security.md, section 10.1 and the secret
    /// detector; writes a `history_entry` when everything passes.
    pub fn history_record(&self, input: HistoryRecordInput) -> Result<HistoryDecision, CoreError> {
        self.history_record_impl(input)
    }

    /// Whether a session linked to `host` (`None`: quick connect or local
    /// terminal) records history now: the host's `record_history` override,
    /// else `terminal.record_history`; `false` for a host of the personal
    /// vault that no longer exists. Running sessions refresh their flag
    /// with it when hosts or settings change.
    pub fn record_history_now(&self, host: Option<RecordRef>) -> Result<bool, CoreError> {
        self.record_history_now_impl(host)
    }

    /// History of one host (or of no host), newest first.
    pub fn history_list(
        &self,
        input: HistoryListInput,
    ) -> Result<Vec<HistoryEntryView>, CoreError> {
        self.history_list_impl(input)
    }

    /// Deletes history entries (tombstones, docs/storage-format.md, section 7).
    pub fn history_delete(&self, input: RecordsRef) -> Result<(), CoreError> {
        self.history_delete_impl(input)
    }

    /// Clears the history of one host (or of no host) in one transaction.
    pub fn history_clear(&self, input: HistoryClearInput) -> Result<HistoryClearResult, CoreError> {
        self.history_clear_impl(input)
    }

    /// Writes the personal `connection_log_entry` of an established SSH
    /// session; returns its record id.
    pub fn connection_log_start(&self, entry: ConnectionLogStart) -> Result<Uuid, CoreError> {
        self.connection_log_start_impl(entry)
    }

    /// Sets `ended_at`, `end_reason` and `exit_status`. Never fails visibly:
    /// while locked the end is kept in memory and written after unlock.
    pub fn connection_log_end(
        &self,
        log_id: Uuid,
        reason: ConnectionEndReason,
        exit_status: Option<u32>,
    ) {
        self.connection_log_end_impl(log_id, reason, exit_status)
    }

    /// Recent connections (UI 6): newest first, one row per host link (or
    /// per address and port), at most 10.
    pub fn recent_connections(&self) -> Result<Vec<RecentConnection>, CoreError> {
        self.recent_connections_impl()
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
