//! Lock state transitions of the facade (docs/security.md, sections 5.9 and 8;
//! docs/storage-format.md, sections 6.2-6.6; UI 13.1-13.4).

use std::time::SystemTime;

use data_encoding::BASE64URL_NOPAD;
use secrecy::{ExposeSecret, SecretString};
use serde_json::{Map, Value, json};
use uuid::Uuid;
use zeroize::Zeroizing;

use super::session::{PendingWrite, Session};
use super::{Core, Inner, checkpoint, device_row_exists, open_error};
use crate::crypto::{self, Key32};
use crate::error::{CoreError, FieldError, ValidationReason};
use crate::ids;
use crate::lock::lockout::{self, META_UNLOCK_LOCKOUT};
use crate::lock::{self, AutoLock, FirstRunOptions, Lockout, PasswordMaterial};
use crate::model::hlc::HlcClock;
use crate::model::time::{format_timestamp, unix_ms};
use crate::model::types::RecordKind;
use crate::platform::{Availability, OsKind};
use crate::service::dto::{CoreEventKind, LockState, LockStatus, RecoveryKeyResult};
use crate::settings::Settings;
use crate::store::{
    KeyWrapRow, LocalAuthRow, META_CREATED_AT, META_DB_ID, META_DEVICE_ID, Store, VaultRow,
    WrapKind,
};
use crate::vault::{self, KeyRing, Rotation};

/// Rows needed to open the personal vault, read under the mutex.
/// `meta` flag: "Remember on this device" went off at a rotation because
/// the device key was missing from the OS key store (D173); Settings says
/// so until the option is set again. Plaintext: it is not a secret.
pub(super) const META_REMEMBER_LOST: &str = "remember_device_lost";

/// `meta` flag: a re-post gave the server this device's newer `recovery_key`
/// record while its recovery verifier is still the older key's (SP 5.1,
/// re-post step 4; D195): Account & Sync offers a reissue until one passes.
pub(super) const META_RECOVERY_REISSUE: &str = "recovery_reissue_needed";

pub(super) struct KeyContext {
    pub local_auth: LocalAuthRow,
    pub vault: VaultRow,
    pub db_id: Uuid,
    pub device_id: Uuid,
    pub kek_row: KeyWrapRow,
    pub recovery_row: Option<KeyWrapRow>,
    pub device_row: Option<KeyWrapRow>,
}

fn meta_uuid(store: &Store, key: &str) -> Result<Uuid, CoreError> {
    store
        .meta_get(key)?
        .and_then(|text| Uuid::parse_str(&text).ok())
        .ok_or(CoreError::Damaged)
}

pub(super) fn key_context(store: &Store) -> Result<KeyContext, CoreError> {
    let local_auth = store.local_auth()?.ok_or(CoreError::WrongState)?;
    let vault_id = local_auth.personal_vault_id;
    let vault = store.vault(&vault_id)?.ok_or(CoreError::Damaged)?;
    let epoch = vault.current_epoch;
    let kek_row = store
        .key_wrap(&vault_id, epoch, WrapKind::Kek)?
        .ok_or(CoreError::Damaged)?;
    Ok(KeyContext {
        db_id: meta_uuid(store, META_DB_ID)?,
        device_id: meta_uuid(store, META_DEVICE_ID)?,
        recovery_row: store.key_wrap(&vault_id, epoch, WrapKind::Recovery)?,
        device_row: store.key_wrap(&vault_id, epoch, WrapKind::Device)?,
        local_auth,
        vault,
        kek_row,
    })
}

/// `true` while the key material read into `context` is still the stored one:
/// the vault epoch, the `local_auth` row and the current `kek` row. Every
/// two-phase operation calls it after re-acquiring the mutex, because another
/// window may have changed the password or rotated the vault while Argon2id
/// ran; acting on the stale rows could leave a `kek` row that no password opens.
pub(super) fn key_rows_unchanged(store: &Store, context: &KeyContext) -> Result<bool, CoreError> {
    let vault_id = context.vault.vault_id;
    let epoch = context.vault.current_epoch;
    Ok(
        store.vault(&vault_id)?.map(|row| row.current_epoch) == Some(epoch)
            && store.local_auth()?.as_ref() == Some(&context.local_auth)
            && store.key_wrap(&vault_id, epoch, WrapKind::Kek)?.as_ref() == Some(&context.kek_row),
    )
}

/// `true` while the `device` row read into `context` is still the stored one.
/// Rotations rewrap it with the device key fetched for that row.
pub(super) fn device_row_unchanged(store: &Store, context: &KeyContext) -> Result<bool, CoreError> {
    let current = store.key_wrap(
        &context.vault.vault_id,
        context.vault.current_epoch,
        WrapKind::Device,
    )?;
    Ok(current == context.device_row)
}

/// [`Session::reseal_older`] as upkeep after a committed change: on failure
/// the records stay on their older epochs, readable through the chain, and
/// the next unlock re-seals them.
pub(super) fn reseal_or_defer(
    session: &mut Session,
    store: &mut Store,
    now: SystemTime,
) -> Vec<Uuid> {
    session.reseal_older(store, now).unwrap_or_else(|error| {
        tracing::warn!(code = error.code(), "re-seal deferred");
        Vec::new()
    })
}

pub(super) fn os_name(os: OsKind) -> &'static str {
    match os {
        OsKind::Windows => "windows",
        OsKind::Macos => "macos",
        OsKind::Linux => "linux",
        OsKind::Ios => "ios",
        OsKind::Android => "android",
    }
}

/// `device_info.name`: `string(64)` from the OS computer name, else `<Platform> device`.
pub(super) fn device_name(name: &str, os: OsKind) -> String {
    let cleaned: String = name.chars().filter(|c| !c.is_control()).collect();
    let cleaned: String = cleaned.trim().chars().take(64).collect();
    let cleaned = cleaned.trim().to_owned();
    if cleaned.is_empty() {
        let platform = match os {
            OsKind::Windows => "Windows",
            OsKind::Macos => "macOS",
            OsKind::Linux => "Linux",
            OsKind::Ios => "iOS",
            OsKind::Android => "Android",
        };
        format!("{platform} device")
    } else {
        cleaned
    }
}

pub(super) fn short_version(version: &str) -> String {
    let version: String = version
        .chars()
        .filter(|c| !c.is_control())
        .take(32)
        .collect();
    if version.trim().is_empty() {
        "0.0.0".to_owned()
    } else {
        version.trim().to_owned()
    }
}

fn object(value: Value) -> Map<String, Value> {
    match value {
        Value::Object(map) => map,
        _ => Map::new(),
    }
}

/// The recovery key held in the `recovery_key` record. The decoded bytes
/// go straight from a zeroizing buffer into the key (no stack copy).
pub(super) fn recovery_key_of(session: &Session) -> Result<Key32, CoreError> {
    let record = session
        .index
        .singleton(RecordKind::RecoveryKey)
        .ok_or(CoreError::Damaged)?;
    let text = record
        .data()
        .get("recovery_key")
        .and_then(Value::as_str)
        .ok_or(CoreError::Damaged)?;
    let bytes = Zeroizing::new(
        BASE64URL_NOPAD
            .decode(text.as_bytes())
            .map_err(|_| CoreError::Damaged)?,
    );
    Key32::from_slice(&bytes).ok_or(CoreError::Damaged)
}

pub(super) fn recovery_record_data(key: &Key32, issued_at: &str) -> Map<String, Value> {
    let encoded = Zeroizing::new(BASE64URL_NOPAD.encode(key.expose()));
    object(json!({ "recovery_key": encoded.as_str(), "issued_at": issued_at }))
}

impl Core {
    fn lock_changed(&self, inner: &mut Inner) {
        let state = inner.state;
        self.emit(
            inner,
            CoreEventKind::LockChanged {
                locked: state != LockState::Unlocked,
                state,
            },
        );
    }

    /// Wipes the session (keys, index, undo buffer), the recovery grant and our
    /// clipboard content; emits `lock:changed`.
    pub(super) fn lock_now(&self, inner: &mut Inner) {
        inner.session = None;
        inner.sync.wipe();
        inner.recovery_grant.clear();
        inner.key_change = None;
        self.clear_clipboard_if_ours(inner);
        inner.device_unlock_allowed = false;
        inner.state = LockState::Locked;
        tracing::info!("locked");
        self.lock_changed(inner);
    }

    /// Reads the device key from the OS keychain (call without the mutex).
    pub(super) fn fetch_device_key(&self, db_id: &Uuid) -> Option<Key32> {
        if self.platform.secret_store.availability() != Availability::Available {
            return None;
        }
        match self
            .platform
            .secret_store
            .get(&ids::keychain_account(db_id))
        {
            Ok(Some(secret)) => Some(Key32::from_secret_box(&secret)),
            Ok(None) => None,
            Err(_) => {
                tracing::warn!("device key could not be read");
                None
            }
        }
    }

    pub(super) fn delete_device_key(&self, db_id: &Uuid) {
        if self
            .platform
            .secret_store
            .delete(&ids::keychain_account(db_id))
            .is_err()
        {
            tracing::warn!("device key could not be deleted");
        }
    }

    /// Rewrites `device_info.app_version` of this device on the first unlock
    /// after an update. Returns the record id when it wrote one.
    fn refresh_app_version(
        &self,
        store: &mut Store,
        session: &mut Session,
        now: SystemTime,
    ) -> Result<Option<Uuid>, CoreError> {
        let version = short_version(&self.config.app_version);
        let device_id = session.device_id.to_string();
        let stale = session
            .index
            .of_kind(RecordKind::DeviceInfo)
            .find(|record| {
                record.data().get("device_id").and_then(Value::as_str) == Some(device_id.as_str())
            })
            .filter(|record| {
                record.data().get("app_version").and_then(Value::as_str) != Some(version.as_str())
            })
            .map(|record| (record.record_id, record.data().clone()));
        let Some((record_id, mut data)) = stale else {
            return Ok(None);
        };
        data.insert("app_version".into(), Value::String(version));
        let write = session.prepare_write(store, RecordKind::DeviceInfo, record_id, data, now)?;
        store.write(|tx| tx.record_put(&write.row))?;
        session.commit(vec![write]);
        Ok(Some(record_id))
    }

    /// Opens the session with the current vault key and unlocks (password,
    /// device key or recovery). Emits `lock:changed` and, when records were
    /// written, `records:changed`.
    ///
    /// Re-sealing older records and the `app_version` rewrite are upkeep:
    /// every record stays readable through the chain, so their failures are
    /// logged and retried at the next unlock instead of failing the unlock.
    pub(super) fn start_session(
        &self,
        inner: &mut Inner,
        context: &KeyContext,
        vault_key: Key32,
        now: SystemTime,
    ) -> Result<(), CoreError> {
        let vault_id = context.vault.vault_id;
        let mut ring = KeyRing::new(vault_id, context.vault.current_epoch, vault_key);
        ring.open_chain(&inner.store)?;
        let mut session = Session::load(&inner.store, ring, context.db_id, context.device_id, now)?;
        let mut written = reseal_or_defer(&mut session, &mut inner.store, now);
        match self.refresh_app_version(&mut inner.store, &mut session, now) {
            Ok(Some(record_id)) => written.push(record_id),
            Ok(None) => {}
            Err(error) => tracing::warn!(code = error.code(), "device_info update deferred"),
        }

        inner.session = Some(session);
        inner.personal_vault_id = Some(vault_id);
        inner.state = LockState::Unlocked;
        inner.auto_lock.activity(now);
        if inner.lockout.reset() {
            Self::save_lockout(inner);
        }
        tracing::info!("unlocked");
        // Stage 2: connection log ends held while locked (stage 2 plan,
        // section 4.4); emits its own `records:changed`.
        self.connlog_after_unlock(inner, now);
        if !written.is_empty() {
            self.emit(
                inner,
                CoreEventKind::RecordsChanged {
                    vault_id,
                    record_ids: written,
                    classes: vec![
                        shellpp_sync_api::classes::VAULT,
                        shellpp_sync_api::classes::HOSTS,
                    ],
                },
            );
        }
        self.lock_changed(inner);
        Ok(())
    }

    pub(super) fn first_run_setup_impl(
        &self,
        password: SecretString,
    ) -> Result<RecoveryKeyResult, CoreError> {
        if self.inner().state != LockState::FirstRun {
            return Err(CoreError::WrongState);
        }
        lock::check_new_password(&password, "password")?;
        let class = self.platform.class;
        let material = PasswordMaterial::fresh(class, None)?;
        let kek = material.derive_kek(&password, class)?;
        drop(password);
        let recovery_key = Key32::generate()?;
        let recovery_kek = crypto::derive_recovery_kek(&recovery_key);

        let mut guard = self.inner();
        let inner = &mut *guard;
        if inner.state != LockState::FirstRun {
            return Err(CoreError::WrongState);
        }
        let now = self.now();
        let now_text = format_timestamp(now);
        let db_id = inner.ids.next(now);
        let device_id = inner.ids.next(now);
        let vault_id = inner.ids.next(now);
        let new_vault = vault::create_personal_vault(vault_id, now_text.clone())?;
        let kek_row = vault::wrap_row(WrapKind::Kek, vault_id, 1, &kek, &new_vault.key)?;
        let recovery_row = vault::wrap_row(
            WrapKind::Recovery,
            vault_id,
            1,
            &recovery_kek,
            &new_vault.key,
        )?;
        drop(kek);
        drop(recovery_kek);
        let vault_row = new_vault.row;
        let ring = KeyRing::new(vault_id, 1, new_vault.key);
        let mut session = Session::new(ring, db_id, device_id, HlcClock::start(None, unix_ms(now)));

        let records = [
            (
                RecordKind::VaultMeta,
                object(json!({ "name": "Personal", "color": "graphite" })),
            ),
            (
                RecordKind::RecoveryKey,
                recovery_record_data(&recovery_key, &now_text),
            ),
            (
                RecordKind::DeviceInfo,
                object(json!({
                    "device_id": device_id,
                    "name": device_name(&self.platform.device_name, self.platform.os),
                    "platform": os_name(self.platform.os),
                    "app_version": short_version(&self.config.app_version),
                })),
            ),
        ];
        let mut writes: Vec<PendingWrite> = Vec::with_capacity(records.len());
        for (kind, data) in records {
            let record_id = inner.ids.next(now);
            writes.push(session.prepare_write(&inner.store, kind, record_id, data, now)?);
        }
        let local_auth = material.to_row(vault_id, now_text.clone());
        let created = inner.store.write(|tx| {
            // Another process on the same data directory may have completed
            // a first run meanwhile: never replace its `local_auth` row.
            if tx.local_auth()?.is_some() {
                return Ok(false);
            }
            tx.meta_set(META_DB_ID, &db_id.to_string())?;
            tx.meta_set(META_DEVICE_ID, &device_id.to_string())?;
            tx.meta_set(META_CREATED_AT, &now_text)?;
            tx.local_auth_put(&local_auth)?;
            tx.vault_put(&vault_row)?;
            tx.key_wrap_put(&kek_row)?;
            tx.key_wrap_put(&recovery_row)?;
            for write in &writes {
                tx.record_put(&write.row)?;
            }
            Ok(true)
        })?;
        if !created {
            return Err(CoreError::WrongState);
        }
        session.commit(writes);
        inner.session = Some(session);
        inner.personal_vault_id = Some(vault_id);
        inner.state = LockState::Unlocked;
        inner.auto_lock.activity(now);
        inner.recovery_grant.grant(now);
        // A sign-in waiting for its TOTP code was for the empty device: its
        // tokens must not bind this new vault (review of run 13).
        inner.sync.drop_pending();
        tracing::info!(vault_id = %vault_id, "first run completed");
        self.lock_changed(inner);
        self.emit(
            inner,
            CoreEventKind::VaultsChanged {
                vault_ids: vec![vault_id],
            },
        );
        Ok(RecoveryKeyResult {
            recovery_key: crypto::encode_recovery_key(&recovery_key),
        })
    }

    pub(super) fn unlock_impl(&self, password: SecretString) -> Result<LockStatus, CoreError> {
        let available = self.remember_available();
        let context = {
            let mut guard = self.inner();
            let inner = &mut *guard;
            if inner.state != LockState::Locked {
                return Err(CoreError::WrongState);
            }
            // The attempt limit (D29) is checked before Argon2id runs.
            let now = self.now();
            if let Some(left) = inner.lockout.remaining(now) {
                return Err(CoreError::TooManyAttempts {
                    retry_after_secs: lockout::whole_seconds(left),
                });
            }
            let context = key_context(&inner.store)?;
            // The attempt counts as a wrong password until Argon2id proves
            // otherwise, under the same lock as the check: concurrent unlocks
            // (another window, another caller) see the pause as soon as the
            // attempt that earns it starts, so they cannot guess in parallel.
            inner.lockout.record_failure(now);
            Self::save_lockout(inner);
            context
        };
        let class = self.platform.class;
        let material = PasswordMaterial::from_row(&context.local_auth)?;
        let kek = material.derive_kek(&password, class)?;
        drop(password);
        let vault_key = match vault::open_wrap(&context.kek_row, &kek) {
            Ok(key) => key,
            Err(_) => {
                drop(kek);
                self.unlock_failed();
                return Err(CoreError::InvalidPassword);
            }
        };
        drop(kek);

        let mut guard = self.inner();
        let inner = &mut *guard;
        // The password is right: the attempt reserved above was no failure.
        if inner.lockout.reset() {
            Self::save_lockout(inner);
        }
        if inner.state != LockState::Locked || !key_rows_unchanged(&inner.store, &context)? {
            return Err(CoreError::WrongState);
        }
        let now = self.now();
        self.start_session(inner, &context, vault_key, now)?;
        Ok(self.status(inner, available))
    }

    /// Logs a wrong master password (D29). [`Core::unlock_impl`] counted it
    /// and stored the counter before Argon2id ran.
    fn unlock_failed(&self) {
        let now = self.now();
        let mut guard = self.inner();
        let inner = &mut *guard;
        let failures = inner.lockout.failures();
        match inner.lockout.remaining(now) {
            Some(pause) => tracing::warn!(
                failures,
                pause_s = lockout::whole_seconds(pause),
                "wrong master password; unlock paused"
            ),
            None => tracing::info!(failures, "wrong master password"),
        }
    }

    /// Writes the attempt counter to `meta` (best effort: on failure it
    /// stays in memory for this run).
    fn save_lockout(inner: &mut Inner) {
        let text = inner.lockout.to_meta();
        if let Err(error) = inner
            .store
            .write(|tx| tx.meta_set(META_UNLOCK_LOCKOUT, &text))
        {
            tracing::warn!(code = error.code(), "unlock attempt counter not saved");
        }
    }

    pub(super) fn first_run_setup_with_impl(
        &self,
        password: SecretString,
        options: FirstRunOptions,
    ) -> Result<LockStatus, CoreError> {
        let recovery = self.first_run_setup_impl(password)?;
        // Local mode never shows the recovery key (D29); it stays in the vault.
        drop(recovery);
        self.inner().recovery_grant.clear();
        if options.remember_device
            && self.remember_available()
            && let Err(error) = self.remember_device_set_impl(true)
        {
            tracing::warn!(code = error.code(), "remember on this device not turned on");
        }
        Ok(self.lock_status())
    }

    pub(super) fn start_new_vault_impl(&self) -> Result<String, CoreError> {
        self.set_aside_store()
    }

    /// Sets the database aside and starts at first run: "Start with a new
    /// vault" from the unlock screen, in one step under the mutex, so no
    /// unlock comes between; a failure leaves the vault as it was.
    pub(super) fn set_aside_store(&self) -> Result<String, CoreError> {
        // No keychain or rotation work may run while the database changes.
        let _remember = self.remember_lock();
        let mut guard = self.inner();
        let inner = &mut *guard;
        if inner.state != LockState::Locked {
            return Err(CoreError::WrongState);
        }
        let data_dir = inner
            .store
            .data_dir()
            .map(std::path::Path::to_path_buf)
            .ok_or(CoreError::Unavailable)?;
        checkpoint(&inner.store);
        // The directory stays locked until the new store holds it: another
        // process must not open the directory while no database is in place.
        // `Store::open` below shares this lock (same process).
        let _dir_lock = inner.store.dir_lock();
        // Closes the database file (Windows cannot rename an open file).
        drop(std::mem::replace(
            &mut inner.store,
            Store::open_in_memory()?,
        ));
        let now = self.now();
        let archived = match lock::new_vault::archive_data_file(&data_dir, now) {
            Ok(name) => name,
            Err(error) => {
                self.reopen_store(inner, &data_dir);
                return Err(CoreError::Store(error.into()));
            }
        };
        let store = match Store::open(&data_dir, &self.config.app_version) {
            Ok((store, _)) => store,
            Err(error) => {
                // Sets the half-made database aside (never deletes it) and
                // puts the archive back.
                if lock::new_vault::restore_archived(&data_dir, &archived, now).is_err() {
                    tracing::error!("archived database not put back");
                }
                self.reopen_store(inner, &data_dir);
                return Err(open_error(error));
            }
        };
        // The old device key (if any) stays in the OS keychain under the old
        // database id: it opens only the archived file, which is kept too.
        tracing::info!("data file archived; first run");
        self.enter_first_run(inner, store, now);
        Ok(archived)
    }

    /// Log out with "delete local copy" (D197): the local copy is deleted
    /// for real ([`lock::erase`]) and the app starts at first run, in one
    /// step under the mutex. The device keys of the set-aside copies leave
    /// the OS keychain with them (the caller removed the current one). When
    /// the database itself cannot be deleted nothing has changed and the
    /// vault stays open; the names of other files that stayed are returned.
    pub(super) fn erase_store(&self) -> Result<Vec<String>, CoreError> {
        let _remember = self.remember_lock();
        let (result, set_aside) = {
            let mut guard = self.inner();
            let inner = &mut *guard;
            if inner.state != LockState::Unlocked {
                return Err(CoreError::WrongState);
            }
            let data_dir = inner
                .store
                .data_dir()
                .map(std::path::Path::to_path_buf)
                .ok_or(CoreError::Unavailable)?;
            // This database's own device key goes too, whatever its
            // `device` row says (a key the keychain once refused to delete
            // would still open a copy of this file; review of run 10).
            let own = inner
                .store
                .meta_get(crate::store::META_DB_ID)
                .ok()
                .flatten()
                .and_then(|id| Uuid::parse_str(&id).ok());
            checkpoint(&inner.store);
            let _dir_lock = inner.store.dir_lock();
            drop(std::mem::replace(
                &mut inner.store,
                Store::open_in_memory()?,
            ));
            let mut set_aside = lock::erase::set_aside_db_ids(&data_dir);
            let left = match lock::erase::erase_data_files(&data_dir) {
                Ok(left) => left,
                Err(error) => {
                    self.reopen_store(inner, &data_dir);
                    return Err(CoreError::Store(error.into()));
                }
            };
            set_aside.extend(own);
            // The database is gone: the session never goes on over an empty
            // new one (review of run 10). The new store is tried twice;
            // without one the session closes, and the next start begins at
            // first run.
            let opened = Store::open(&data_dir, &self.config.app_version)
                .or_else(|_| Store::open(&data_dir, &self.config.app_version));
            let result = match opened {
                Ok((store, _)) => {
                    tracing::info!(left = left.len(), "local copy deleted; first run");
                    self.enter_first_run(inner, store, self.now());
                    Ok(left)
                }
                Err(error) => {
                    tracing::error!(code = error.code(), "no new database after the deletion");
                    if inner.session.take().is_some() {
                        inner.sync.wipe();
                        self.clear_clipboard_if_ours(inner);
                    }
                    inner.personal_vault_id = None;
                    inner.state = LockState::Locked;
                    self.lock_changed(inner);
                    Err(open_error(error))
                }
            };
            (result, set_aside)
        };
        // The deleted copies' keys go either way.
        for db_id in set_aside {
            self.delete_device_key(&db_id);
        }
        result
    }

    /// Opens the database of `data_dir` again after a failed set-aside or
    /// erase. When it does not open, no database is there to write to: an
    /// open session would keep edits in memory only and lose them, so it is
    /// closed; the app opens the database again at its next start (cross
    /// review of run 9).
    fn reopen_store(&self, inner: &mut Inner, data_dir: &std::path::Path) {
        match Store::open(data_dir, &self.config.app_version) {
            Ok((store, _)) => inner.store = store,
            Err(error) => {
                tracing::error!(code = error.code(), "database not reopened");
                if inner.session.take().is_some() {
                    inner.sync.wipe();
                    self.clear_clipboard_if_ours(inner);
                }
                inner.state = LockState::Locked;
                self.lock_changed(inner);
            }
        }
    }

    /// The new, empty `store` holds the directory: the session is wiped
    /// (what a lock wipes: keys, sync tokens, clipboard) and the app is at
    /// first run.
    fn enter_first_run(&self, inner: &mut Inner, store: Store, now: std::time::SystemTime) {
        let settings = match store.settings() {
            Ok(rows) => Settings::new(self.platform.os, &rows),
            Err(_) => Settings::new(self.platform.os, &[]),
        };
        inner.store = store;
        inner.settings = settings;
        if inner.session.is_some() {
            inner.sync.wipe();
            self.clear_clipboard_if_ours(inner);
        }
        inner.session = None;
        inner.personal_vault_id = None;
        inner.lockout = Lockout::default();
        inner.recovery_grant.clear();
        inner.key_change = None;
        inner.auto_lock = AutoLock::new(now);
        inner.device_unlock_allowed = true;
        inner.state = LockState::FirstRun;
        self.lock_changed(inner);
        let keys: Vec<String> = inner.settings.keys().map(str::to_owned).collect();
        self.emit(inner, CoreEventKind::SettingsChanged { keys });
    }

    pub(super) fn try_device_unlock_impl(&self) -> Result<bool, CoreError> {
        let context = {
            let inner = self.inner();
            if inner.state != LockState::Locked || !inner.device_unlock_allowed {
                return Ok(false);
            }
            match key_context(&inner.store) {
                Ok(context) => context,
                Err(_) => return Ok(false),
            }
        };
        let Some(device_row) = context.device_row.as_ref() else {
            return Ok(false);
        };
        let Some(device_key) = self.fetch_device_key(&context.db_id) else {
            return Ok(false);
        };
        let wrapping_key = crypto::derive_device_wrap_key(&device_key, &context.db_id);
        drop(device_key);
        let Ok(vault_key) = vault::open_wrap(device_row, &wrapping_key) else {
            tracing::warn!("device key does not open the vault");
            return Ok(false);
        };
        drop(wrapping_key);

        let mut inner = self.inner();
        if inner.state != LockState::Locked
            || !inner.device_unlock_allowed
            || !matches!(key_rows_unchanged(&inner.store, &context), Ok(true))
        {
            return Ok(false);
        }
        let now = self.now();
        match self.start_session(&mut inner, &context, vault_key, now) {
            Ok(()) => Ok(true),
            Err(error) => {
                tracing::warn!(code = error.code(), "device unlock failed");
                Ok(false)
            }
        }
    }

    pub(super) fn lock_impl(&self) -> Result<LockStatus, CoreError> {
        let available = self.remember_available();
        let mut inner = self.inner();
        match inner.state {
            LockState::FirstRun => Err(CoreError::WrongState),
            LockState::Locked => Ok(self.status(&inner, available)),
            LockState::Unlocked => {
                self.lock_now(&mut inner);
                Ok(self.status(&inner, available))
            }
        }
    }

    pub(super) fn recovery_unlock_impl(
        &self,
        recovery_key: SecretString,
        new_password: SecretString,
    ) -> Result<LockStatus, CoreError> {
        let available = self.remember_available();
        // Held until the end: the rotation rewraps the `device` row with the
        // device key fetched below, and a lost device key is deleted last.
        let _remember = self.remember_lock();
        if self.inner().state != LockState::Locked {
            return Err(CoreError::WrongState);
        }
        let mut errors: Vec<FieldError> = Vec::new();
        let decoded = crypto::decode_recovery_key(recovery_key.expose_secret());
        drop(recovery_key);
        if decoded.is_err() {
            errors.push(FieldError::new("recovery_key", ValidationReason::Invalid));
        }
        if !crypto::password_length_ok(&new_password) {
            errors.push(FieldError::new("new_password", ValidationReason::TooShort));
        }
        let recovery_key = match decoded {
            Ok(key) if errors.is_empty() => key,
            _ => return Err(CoreError::Validation(errors)),
        };

        let context = {
            let inner = self.inner();
            if inner.state != LockState::Locked {
                return Err(CoreError::WrongState);
            }
            key_context(&inner.store)?
        };
        let recovery_row = context
            .recovery_row
            .as_ref()
            .ok_or(CoreError::InvalidRecoveryKey)?;
        let recovery_kek = crypto::derive_recovery_kek(&recovery_key);
        drop(recovery_key);
        let vault_key = vault::open_wrap(recovery_row, &recovery_kek)
            .map_err(|_| CoreError::InvalidRecoveryKey)?;
        let device_key = context
            .device_row
            .as_ref()
            .and_then(|_| self.fetch_device_key(&context.db_id));
        let device_lost = context.device_row.is_some() && device_key.is_none();
        let class = self.platform.class;
        let material = PasswordMaterial::fresh(class, context.local_auth.kdf_email.clone())?;
        let kek = material.derive_kek(&new_password, class)?;
        drop(new_password);

        let mut guard = self.inner();
        let inner = &mut *guard;
        if inner.state != LockState::Locked
            || !key_rows_unchanged(&inner.store, &context)?
            || !device_row_unchanged(&inner.store, &context)?
        {
            return Err(CoreError::WrongState);
        }
        let vault_id = context.vault.vault_id;
        // Bound to an account: the offline recovery of storage-format 6.6,
        // a local rewrap without a rotation (the server would not know the
        // new epoch); the device signs in again afterwards.
        if let Some(mut account) = inner.store.server_account()? {
            let epoch = context.vault.current_epoch;
            let kek_row = vault::wrap_row(WrapKind::Kek, vault_id, epoch, &kek, &vault_key)?;
            account.refresh_token = None;
            let now = self.now();
            let local_auth = material.to_row(vault_id, format_timestamp(now));
            inner.store.write(|tx| {
                tx.key_wrap_put(&kek_row)?;
                tx.local_auth_put(&local_auth)?;
                tx.server_account_put(&account)
            })?;
            drop(kek);
            drop(recovery_kek);
            tracing::info!("recovered with the recovery key; bound, so no rotation");
            let started = match key_context(&inner.store) {
                Ok(new_context) => self.start_session(inner, &new_context, vault_key, now),
                Err(error) => Err(error),
            };
            if let Err(error) = started {
                tracing::warn!(code = error.code(), "unlock after recovery failed");
            }
            checkpoint(&inner.store);
            let status = self.status(inner, available);
            return Ok(status);
        }
        let ring = KeyRing::new(vault_id, context.vault.current_epoch, vault_key);
        let device_wrap_key = device_key
            .as_ref()
            .map(|key| crypto::derive_device_wrap_key(key, &context.db_id));
        let rotation = Rotation::prepare(
            &ring,
            &context.vault,
            &kek,
            &recovery_kek,
            device_wrap_key.as_ref(),
        )?;
        let now = self.now();
        let local_auth = material.to_row(vault_id, format_timestamp(now));
        inner.store.write(|tx| {
            rotation.write(tx)?;
            tx.local_auth_put(&local_auth)
        })?;
        drop(kek);
        drop(recovery_kek);
        tracing::info!(
            epoch = rotation.new_epoch,
            "recovered with the recovery key; vault rotated"
        );

        // The new password is in place: from here on nothing fails the
        // command. Unlock with the new epoch's key; the session reaches the
        // older epochs through the chain row written above and re-seals the
        // older records. If the session cannot start, the app stays locked
        // and the user unlocks with the new password.
        drop(ring);
        let started = match key_context(&inner.store) {
            Ok(new_context) => self.start_session(inner, &new_context, rotation.new_key(), now),
            Err(error) => Err(error),
        };
        if let Err(error) = started {
            tracing::warn!(code = error.code(), "unlock after recovery failed");
        }
        checkpoint(&inner.store);
        self.emit(
            inner,
            CoreEventKind::VaultsChanged {
                vault_ids: vec![vault_id],
            },
        );
        let status = self.status(inner, available);
        drop(guard);
        if device_lost {
            self.delete_device_key(&context.db_id);
        }
        Ok(status)
    }

    /// Verifies the master password against the current `kek` row and returns `kek`.
    pub(super) fn verify_password(
        &self,
        context: &KeyContext,
        password: &SecretString,
    ) -> Result<Key32, CoreError> {
        let class = self.platform.class;
        let material = PasswordMaterial::from_row(&context.local_auth)?;
        let kek = material.derive_kek(password, class)?;
        vault::open_wrap(&context.kek_row, &kek).map_err(|_| CoreError::InvalidPassword)?;
        Ok(kek)
    }

    fn unlocked_context(&self) -> Result<KeyContext, CoreError> {
        let inner = self.inner();
        Self::require_session(&inner)?;
        key_context(&inner.store)
    }

    pub(super) fn password_change_impl(
        &self,
        current: SecretString,
        new: SecretString,
    ) -> Result<(), CoreError> {
        let context = self.unlocked_context()?;
        lock::check_new_password(&new, "new_password")?;
        // A bound vault changes the password on the server first (stage 7).
        if self.bound()? {
            return self.password_change_bound(&context, current, new);
        }
        self.verify_password(&context, &current)?;
        drop(current);
        let class = self.platform.class;
        let material = PasswordMaterial::fresh(class, context.local_auth.kdf_email.clone())?;
        let new_kek = material.derive_kek(&new, class)?;
        drop(new);

        let mut guard = self.inner();
        let inner = &mut *guard;
        let session = match (&inner.state, &inner.session) {
            (LockState::Unlocked, Some(session)) => session,
            _ => return Err(CoreError::Locked),
        };
        let epoch = session.ring.current_epoch();
        if epoch != context.vault.current_epoch || !key_rows_unchanged(&inner.store, &context)? {
            return Err(CoreError::WrongState);
        }
        let vault_id = session.ring.vault_id();
        let kek_row = vault::wrap_row(
            WrapKind::Kek,
            vault_id,
            epoch,
            &new_kek,
            session.ring.current_key(),
        )?;
        let local_auth = material.to_row(vault_id, format_timestamp(self.now()));
        inner.store.write(|tx| {
            tx.key_wrap_put(&kek_row)?;
            tx.local_auth_put(&local_auth)
        })?;
        tracing::info!("master password changed");
        // The replaced `kek` row opens the vault key with the old password.
        checkpoint(&inner.store);
        Ok(())
    }

    pub(super) fn recovery_key_show_impl(
        &self,
        password: SecretString,
    ) -> Result<RecoveryKeyResult, CoreError> {
        let context = self.unlocked_context()?;
        self.require_recovery_key_mode()?;
        self.verify_password(&context, &password)?;
        drop(password);
        let now = self.now();
        let mut inner = self.inner();
        let key = recovery_key_of(Self::require_session(&inner)?)?;
        inner.recovery_grant.grant(now);
        Ok(RecoveryKeyResult {
            recovery_key: crypto::encode_recovery_key(&key),
        })
    }

    pub(super) fn remember_device_set_impl(&self, enabled: bool) -> Result<LockStatus, CoreError> {
        let available = self.remember_available();
        // One toggle at a time, and never during a rotation: the keychain
        // item and the `device` row must stay a pair.
        let _remember = self.remember_lock();
        let db_id = {
            let inner = self.inner();
            let db_id = Self::require_session(&inner)?.db_id;
            if enabled && device_row_exists(&inner) {
                // Already on (a window with a stale toggle): keep the key.
                return Ok(self.status(&inner, available));
            }
            db_id
        };
        if enabled {
            if !available {
                return Err(CoreError::Unavailable);
            }
            let device_key = Key32::generate()?;
            self.platform
                .secret_store
                .set(&ids::keychain_account(&db_id), &device_key.to_secret_box())
                .map_err(|_| CoreError::Unavailable)?;
            let wrapping_key = crypto::derive_device_wrap_key(&device_key, &db_id);
            drop(device_key);

            let result = (|| {
                let mut inner = self.inner();
                let session = Self::require_session(&inner)?;
                let vault_id = session.ring.vault_id();
                let epoch = session.ring.current_epoch();
                let row = vault::wrap_row(
                    WrapKind::Device,
                    vault_id,
                    epoch,
                    &wrapping_key,
                    session.ring.current_key(),
                )?;
                // The user chose again: the note of a lost device key goes.
                inner.store.write(|tx| {
                    tx.key_wrap_put(&row)?;
                    tx.meta_set(META_REMEMBER_LOST, "0")
                })?;
                tracing::info!("remember on this device turned on");
                self.remember_changed(&mut inner, vault_id);
                Ok(self.status(&inner, available))
            })();
            if result.is_err() {
                // Locked meanwhile or the write failed: no keychain item without its wrap.
                self.delete_device_key(&db_id);
            }
            result
        } else {
            // The keychain item goes first. When the OS refuses to delete it,
            // nothing changes: the option stays on, and Settings says so and
            // can try again. A `device` row left without its item (locked
            // meanwhile, or the write below failed) opens nothing; the next
            // launch asks for the password, and turning the option off again
            // removes the row.
            if self
                .platform
                .secret_store
                .delete(&ids::keychain_account(&db_id))
                .is_err()
            {
                tracing::warn!("device key could not be deleted; remember stays on");
                return Err(CoreError::Unavailable);
            }
            let mut inner = self.inner();
            let session = Self::require_session(&inner)?;
            let vault_id = session.ring.vault_id();
            let epoch = session.ring.current_epoch();
            inner.store.write(|tx| {
                tx.key_wrap_delete(&vault_id, epoch, WrapKind::Device)?;
                tx.meta_set(META_REMEMBER_LOST, "0")
            })?;
            // The deleted `device` row opens the vault key with the device key.
            checkpoint(&inner.store);
            tracing::info!("remember on this device turned off");
            self.remember_changed(&mut inner, vault_id);
            Ok(self.status(&inner, available))
        }
    }

    /// The `device` row changed: every window takes a new snapshot, so its
    /// "Remember on this device" toggle shows the stored state (UI 1.4).
    fn remember_changed(&self, inner: &mut Inner, vault_id: Uuid) {
        self.emit(
            inner,
            CoreEventKind::VaultsChanged {
                vault_ids: vec![vault_id],
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::memory::MemoryPlatform;
    use crate::service::CoreConfig;

    fn generated_password() -> SecretString {
        let bytes = crypto::random_array::<12>().expect("random");
        let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
        SecretString::from(format!("Pw-{hex}-x"))
    }

    #[test]
    fn key_rows_unchanged_sees_password_changes_and_rotations() {
        // The local recovery paths (the app offers neither in local mode, D217).
        crate::service::local_recovery_key::allow(true);
        let dir = tempfile::tempdir().expect("temp dir");
        let mem = MemoryPlatform::new();
        let config = CoreConfig {
            data_dir: dir.path().join("data"),
            app_version: "0.1.0".to_owned(),
        };
        let core = Core::open(config, mem.platform(OsKind::Windows, "TEST-PC")).expect("open");
        let first = generated_password();
        core.first_run_setup(first.clone()).expect("first run");
        let initial = key_context(&core.inner().store).expect("context");
        assert!(key_rows_unchanged(&core.inner().store, &initial).unwrap());

        // A password change keeps the epoch but replaces `kek` and `local_auth`.
        let second = generated_password();
        core.password_change(first, second.clone())
            .expect("password change");
        assert!(!key_rows_unchanged(&core.inner().store, &initial).unwrap());

        // A reissue rotates the vault.
        let changed = key_context(&core.inner().store).expect("context");
        assert!(key_rows_unchanged(&core.inner().store, &changed).unwrap());
        core.recovery_key_reissue(second).expect("reissue");
        assert!(!key_rows_unchanged(&core.inner().store, &changed).unwrap());
    }

    #[test]
    fn device_names() {
        assert_eq!(
            device_name("  WORKSTATION-7 ", OsKind::Windows),
            "WORKSTATION-7"
        );
        assert_eq!(device_name("", OsKind::Macos), "macOS device");
        assert_eq!(device_name("\u{7}", OsKind::Linux), "Linux device");
        assert_eq!(
            device_name(&"n".repeat(100), OsKind::Linux).chars().count(),
            64
        );
        assert_eq!(short_version("0.1.0"), "0.1.0");
        assert_eq!(short_version(&"9".repeat(40)).len(), 32);
    }
}
