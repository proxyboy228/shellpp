//! Key changes that give the user a new recovery key (docs/sync-protocol.md,
//! sections 5.1, 5.3 and 5.9; D173, item 2): the recovery key reissue and
//! the password change with "My old password may be compromised".
//!
//! A new key is shown and confirmed ("I saved it") before anything is sent
//! or written (section 5.3, step 4): `*_prepare` checks the password, makes
//! the new key and keeps the change in memory; [`Core::key_change_confirm`]
//! commits it, through the server when bound (one request, one server
//! transaction), locally otherwise. A lock or [`Core::key_change_cancel`]
//! drops it. In local mode the compromised password change commits at once:
//! its new `recovery_key` record is never shown (D29).

use std::sync::atomic::{AtomicU64, Ordering};

use secrecy::SecretString;
use shellpp_sync_api::api::{self as wire, B64, code};
use uuid::Uuid;

use super::lock_ops;
use super::lock_ops::KeyContext;
use super::sync_ops::{self, Link, key_b64, wire_kdf, wire_rotation};
use super::{Core, Inner, checkpoint};
use crate::crypto::{self, Key32};
use crate::error::CoreError;
use crate::lock::{self, PasswordMaterial};
use crate::model::time::format_timestamp;
use crate::model::types::RecordKind;
use crate::service::dto::{CoreEventKind, LockState, RecoveryKeyResult};
use crate::store::WrapKind;
use crate::sync::{HttpApi, HttpError};
use crate::vault::Rotation;

/// Names a prepared change, so a dialog confirms, cancels or copies only
/// its own: a second prepare (another window) replaces the first, whose
/// dialog then gets "not accepted" instead of committing another key.
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// A prepared change: its id and the new key to show.
pub struct PreparedKey {
    pub id: u64,
    pub recovery_key: SecretString,
}

impl std::fmt::Debug for PreparedKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreparedKey")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

/// A key change waiting for "I saved it". Memory only; wiped on lock.
pub(super) struct PendingKeyChange {
    id: u64,
    /// The key rows the change was prepared against: a change made
    /// meanwhile (another window, a sign-in) refuses the confirmation.
    context: KeyContext,
    /// Wraps the new epoch's key: the new password's `kek` when the
    /// password changes, else the current one's.
    kek: Key32,
    /// The new password's salt material, when the password changes.
    new_password: Option<PasswordMaterial>,
    /// Through the server: `current_auth_key` and, with a new password,
    /// its `auth_key`. `None` in local mode.
    server: Option<ServerKeys>,
    recovery_key: Key32,
}

struct ServerKeys {
    current_auth_key: Key32,
    auth_key: Option<Key32>,
    /// The origin the auth keys were derived for (they are bound to it,
    /// section 4.1): the confirmation goes only there (run 9 review
    /// finding; D188's token binding stops it too).
    origin: String,
}

/// Tests: the next key change on this thread that the server accepted
/// is not stored on the device, as a failing write would leave it
/// (`test-hooks`; never in a release build).
#[cfg(any(test, feature = "test-hooks"))]
pub mod fail_store {
    use std::cell::Cell;

    thread_local! {
        static NEXT: Cell<bool> = const { Cell::new(false) };
    }

    pub fn once() {
        NEXT.with(|next| next.set(true));
    }

    pub(super) fn take() -> bool {
        NEXT.with(|next| next.replace(false))
    }
}

/// Why a confirmed key change did not finish. The new key's fate differs,
/// so the interface tells the user which (section 5.3, step 7).
#[derive(Debug)]
pub enum KeyChangeFailure {
    /// Nothing changed (refused before sending, or the server answered
    /// with an error and rolled back): the key shown is not valid, the old
    /// password and recovery key still work.
    NotAccepted(CoreError),
    /// This device was signed out before the request was answered: a
    /// change committed meanwhile (on another device, or this device's own
    /// earlier attempt whose answer was lost) revoked its tokens. The user
    /// keeps the key shown until a sign-in tells which key is valid.
    SignedOut(CoreError),
    /// The server did not answer: the change may or may not have been
    /// made. The user keeps the new key until a sign-in tells.
    Unknown(CoreError),
    /// The server accepted the change but this device could not store it
    /// (locked meanwhile, a write failed): the new key and password are
    /// the account's; signing in again repairs this device (section 5.1).
    NotStoredHere(CoreError),
}

impl KeyChangeFailure {
    pub fn error(&self) -> &CoreError {
        match self {
            Self::NotAccepted(error)
            | Self::SignedOut(error)
            | Self::Unknown(error)
            | Self::NotStoredHere(error) => error,
        }
    }

    pub fn into_error(self) -> CoreError {
        match self {
            Self::NotAccepted(error)
            | Self::SignedOut(error)
            | Self::Unknown(error)
            | Self::NotStoredHere(error) => error,
        }
    }
}

impl std::fmt::Display for KeyChangeFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.error().fmt(f)
    }
}

/// The answer to the change itself: an error answer of the server (`4xx`)
/// means its transaction rolled back; anything else (no answer, a gateway's
/// `5xx`, a timeout, an answer that does not parse) may have come after
/// the commit.
pub(super) fn sent_failure(error: HttpError) -> KeyChangeFailure {
    let rolled_back = match &error {
        // Only an error body of the server itself (section 8) proves a
        // rollback; a 4xx without one (`http_408` of a proxy that gave up
        // waiting) may have come after the commit.
        HttpError::Api { status, code, .. } => {
            (400..500).contains(status) && !code.starts_with("http_")
        }
        HttpError::Refused(_) | HttpError::Tls(_) | HttpError::Origin(_) => true,
        HttpError::Network | HttpError::Timeout | HttpError::Malformed => false,
    };
    if rolled_back {
        KeyChangeFailure::NotAccepted(error.into())
    } else {
        KeyChangeFailure::Unknown(error.into())
    }
}

/// `409 epoch_conflict` with a current epoch below this device's: the
/// server lost epochs (a restore), which is "server state rolled back"
/// (SP 5.1), not a concurrent change; nothing changed either way.
fn rolled_back_epoch(failure: KeyChangeFailure, local_epoch: u32) -> KeyChangeFailure {
    let behind = match &failure {
        KeyChangeFailure::NotAccepted(CoreError::Sync(HttpError::Api {
            code, details, ..
        })) => {
            code == code::EPOCH_CONFLICT
                && details
                    .get("current_epoch")
                    .and_then(serde_json::Value::as_u64)
                    .is_some_and(|current| current < u64::from(local_epoch))
        }
        _ => false,
    };
    if behind {
        KeyChangeFailure::NotAccepted(CoreError::Sync(HttpError::Refused("rolled_back")))
    } else {
        failure
    }
}

/// A failed token refresh before the change was sent (again): this request
/// changed nothing. A refused refresh token signed the device out, which
/// only a committed change (or a revocation) does.
fn refresh_failure(error: CoreError) -> KeyChangeFailure {
    let signed_out = matches!(
        &error,
        CoreError::Sync(error) if matches!(
            error.api_code(),
            Some(code::REFRESH_TOKEN_REUSED | code::UNAUTHENTICATED)
        )
    );
    if signed_out {
        KeyChangeFailure::SignedOut(error)
    } else {
        KeyChangeFailure::NotAccepted(error)
    }
}

impl Core {
    /// UI 13.4 "Reissue", first step: checks the master password and makes
    /// a new recovery key, returned to be shown; nothing is sent or written
    /// until [`Core::key_change_confirm`] (section 5.3, step 4).
    pub fn recovery_key_reissue_prepare(
        &self,
        password: SecretString,
    ) -> Result<PreparedKey, CoreError> {
        let context = self.unlocked_key_context()?;
        let (kek, server) = if self.bound()? {
            self.require_signed_in()?;
            let (link, keys) = self.current_keys(&password)?;
            (
                keys.kek,
                Some(ServerKeys {
                    current_auth_key: keys.auth_key,
                    auth_key: None,
                    origin: link.api.origin().to_owned(),
                }),
            )
        } else {
            self.require_no_account_binding(&context)?;
            (self.verify_password(&context, &password)?, None)
        };
        drop(password);
        self.keep_pending(PendingKeyChange {
            id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            context,
            kek,
            new_password: None,
            server,
            recovery_key: Key32::generate()?,
        })
    }

    /// UI 13.4 "Change master password" with "My old password may be
    /// compromised" on (D173): the change also rotates the personal vault
    /// and reissues the recovery key. Bound: checks the passwords, makes the
    /// new key and returns it to be shown; [`Core::key_change_confirm`] then
    /// sends everything in one request (section 5.9). Local mode: done at
    /// once, with a new `recovery_key` record that is not shown (`None`).
    pub fn password_change_prepare(
        &self,
        current: SecretString,
        new: SecretString,
    ) -> Result<Option<PreparedKey>, CoreError> {
        let context = self.unlocked_key_context()?;
        lock::check_new_password(&new, "new_password")?;
        let class = self.platform.class;
        if !self.bound()? {
            self.require_no_account_binding(&context)?;
            let old_kek = self.verify_password(&context, &current)?;
            drop((current, old_kek));
            let material = PasswordMaterial::fresh(class, context.local_auth.kdf_email.clone())?;
            let kek = material.derive_kek(&new, class)?;
            drop(new);
            let pending = PendingKeyChange {
                id: 0,
                context,
                kek,
                new_password: Some(material),
                server: None,
                recovery_key: Key32::generate()?,
            };
            self.key_change_local(pending)
                .map_err(KeyChangeFailure::into_error)?;
            return Ok(None);
        }
        self.require_signed_in()?;
        let (link, current_keys) = self.current_keys(&current)?;
        drop(current);
        let origin = link.api.origin().to_owned();
        let email = context.local_auth.kdf_email.clone().unwrap_or_default();
        let material = PasswordMaterial::fresh(class, Some(email.clone()))?;
        let derived = sync_ops::derive(
            &material.kdf,
            &material.kdf_salt,
            &email,
            &new,
            &origin,
            class,
        )?;
        drop(new);
        self.keep_pending(PendingKeyChange {
            id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            context,
            kek: derived.kek,
            new_password: Some(material),
            server: Some(ServerKeys {
                current_auth_key: current_keys.auth_key,
                auth_key: Some(derived.auth_key),
                origin: origin.clone(),
            }),
            recovery_key: Key32::generate()?,
        })
        .map(Some)
    }

    /// Commits the prepared change `id` after "I saved it". The prepared
    /// change is used up either way: after a failure the key shown is not
    /// used again (section 5.3, step 7), and a new attempt starts with a new
    /// key. Another id (a newer prepare replaced it) is "not accepted".
    pub fn key_change_confirm(&self, id: u64) -> Result<(), KeyChangeFailure> {
        self.key_change_confirm_revoking(id, None)
    }

    /// "Lost or stolen" (UI 12.2; SP 4.7): the device was revoked when the
    /// user chose it; after "I saved it" it is revoked once more (a sign-in
    /// with the old password meanwhile would have cleared that) and the
    /// prepared change is sent right after, so the thief's old password and
    /// recovery key stop working.
    pub fn key_change_confirm_lost(
        &self,
        id: u64,
        device_id: Uuid,
    ) -> Result<(), KeyChangeFailure> {
        self.key_change_confirm_revoking(id, Some(device_id))
    }

    fn key_change_confirm_revoking(
        &self,
        id: u64,
        revoke: Option<Uuid>,
    ) -> Result<(), KeyChangeFailure> {
        let pending = {
            let mut guard = self.inner();
            Self::require_session(&guard).map_err(KeyChangeFailure::NotAccepted)?;
            match guard.key_change.take() {
                Some(pending) if pending.id == id => pending,
                other => {
                    guard.key_change = other;
                    return Err(KeyChangeFailure::NotAccepted(CoreError::WrongState));
                }
            }
        };
        if pending.server.is_some() {
            self.key_change_bound(pending, revoke)
        } else if revoke.is_some() {
            Err(KeyChangeFailure::NotAccepted(CoreError::WrongState))
        } else {
            self.key_change_local(pending)
        }
    }

    /// "Cancel" on the step that shows the new key: nothing was sent.
    pub fn key_change_cancel(&self, id: u64) {
        let mut guard = self.inner();
        if guard
            .key_change
            .as_ref()
            .is_some_and(|pending| pending.id == id)
        {
            guard.key_change = None;
        }
    }

    /// The account's recovery key while the recovery grant runs (5 minutes
    /// after registration, Show or Reissue, never after a lock): the
    /// wizard's step that shows it (UI 13.5, step 4).
    pub fn recovery_key_granted(&self) -> Result<RecoveryKeyResult, CoreError> {
        let now = self.now();
        let inner = self.inner();
        let session = Self::require_session(&inner)?;
        if !inner.recovery_grant.allowed(now) {
            return Err(CoreError::WrongState);
        }
        Ok(RecoveryKeyResult {
            recovery_key: crypto::encode_recovery_key(&lock_ops::recovery_key_of(session)?),
        })
    }

    /// The prepared key's text, for "Copy" on the step that shows it.
    pub(super) fn pending_recovery_key(inner: &Inner, id: u64) -> Option<SecretString> {
        inner
            .key_change
            .as_ref()
            .filter(|pending| pending.id == id)
            .map(|pending| crypto::encode_recovery_key(&pending.recovery_key))
    }

    /// After Log out with the local copy kept, the personal vault keeps its
    /// account binding (a later sign-in is a rebind): a local rotation or a
    /// new recovery key there would leave the server behind ("server state
    /// rolled back") with a recovery verifier no key opens. Refused: the
    /// change is made through the account after signing in; one without
    /// the option stays local (SP 5.1; D195).
    fn require_no_account_binding(&self, context: &KeyContext) -> Result<(), CoreError> {
        let (account, _) = self.inner().store.vault_binding(&context.vault.vault_id)?;
        if account.is_some() {
            return Err(CoreError::Sync(HttpError::Refused("account_binding")));
        }
        Ok(())
    }

    /// A bound device that is signed out (no refresh token) cannot send a
    /// key change: it is refused before a new key is shown.
    fn require_signed_in(&self) -> Result<(), CoreError> {
        let guard = self.inner();
        let signed_in = guard
            .store
            .server_account()?
            .is_some_and(|account| account.refresh_token.is_some());
        if signed_in {
            Ok(())
        } else {
            Err(CoreError::Sync(HttpError::Refused("signed_out")))
        }
    }

    fn unlocked_key_context(&self) -> Result<KeyContext, CoreError> {
        let guard = self.inner();
        Self::require_session(&guard)?;
        lock_ops::key_context(&guard.store)
    }

    fn keep_pending(&self, pending: PendingKeyChange) -> Result<PreparedKey, CoreError> {
        let shown = PreparedKey {
            id: pending.id,
            recovery_key: crypto::encode_recovery_key(&pending.recovery_key),
        };
        let mut guard = self.inner();
        Self::require_session(&guard)?;
        guard.key_change = Some(pending);
        Ok(shown)
    }

    /// Sends one request of a key change. The access token is made sure of
    /// first, so a failure of that step is known to have sent nothing; a
    /// `401 unauthenticated` of the change itself (an expired token) means
    /// the server did not run it, and it is sent once more after a refresh.
    fn send_change<T>(
        &self,
        link: &Link,
        call: impl Fn(&HttpApi, &str) -> Result<T, HttpError>,
    ) -> Result<T, KeyChangeFailure> {
        // Only a token of this client's origin (D188).
        let held = {
            let inner = self.inner();
            inner
                .sync
                .access
                .clone()
                .filter(|_| inner.sync.access_origin.as_deref() == Some(link.api.origin()))
        };
        let token = match held {
            Some(token) => token,
            None => self.refresh(link).map_err(refresh_failure)?,
        };
        let answer = match call(&link.api, &token) {
            Err(error) if error.api_code() == Some(code::UNAUTHENTICATED) => {
                self.inner().sync.access = None;
                let token = self.refresh(link).map_err(refresh_failure)?;
                call(&link.api, &token)
            }
            other => other,
        };
        answer.map_err(|error| {
            tracing::warn!(code = error.code(), "key change not committed");
            if error.api_code() == Some(code::ACCOUNT_DELETED) {
                return KeyChangeFailure::NotAccepted(self.on_call_error(error));
            }
            // No answer stays "not known": the server may still be running
            // the transaction (it goes on after the client gave up), so not
            // even a later answer of another request proves it rolled back.
            sent_failure(error)
        })
    }

    /// Through the server: the reissue of section 5.3, or with a new
    /// password the combined request of section 5.9; the local rows change
    /// only after the server committed.
    fn key_change_bound(
        &self,
        pending: PendingKeyChange,
        revoke: Option<Uuid>,
    ) -> Result<(), KeyChangeFailure> {
        use KeyChangeFailure::NotAccepted;
        let _sync = self.sync_lock();
        // The rotation rewraps the `device` row ("remember on this device").
        let _remember = self.remember_lock();
        let server = pending
            .server
            .as_ref()
            .ok_or(NotAccepted(CoreError::Internal("server keys")))?;
        let link = self.link().map_err(NotAccepted)?;
        let origin = link.api.origin().to_owned();
        if origin != server.origin {
            return Err(NotAccepted(CoreError::WrongState));
        }
        // The `device` row as it is now (the option may have changed while
        // the key was shown); the other key rows must be the prepared ones.
        let context = {
            let guard = self.inner();
            lock_ops::key_context(&guard.store).map_err(NotAccepted)?
        };
        let device_wrap_key = self.rotation_device_key(&context);
        let device_lost = context.device_row.is_some() && device_wrap_key.is_none();
        let recovery_kek = crypto::derive_recovery_kek(&pending.recovery_key);
        let recovery_auth_key = crypto::derive_recovery_auth_key(&pending.recovery_key, &origin);
        let now = self.now();
        let (rotation, write) = self
            .with_session(|inner| {
                if !lock_ops::key_rows_unchanged(&inner.store, &pending.context)? {
                    return Err(CoreError::WrongState);
                }
                let session = inner.session.as_mut().ok_or(CoreError::Locked)?;
                let record_id = session
                    .index
                    .singleton(RecordKind::RecoveryKey)
                    .map(|record| record.record_id)
                    .ok_or(CoreError::Damaged)?;
                let rotation = Rotation::prepare(
                    &session.ring,
                    &pending.context.vault,
                    &pending.kek,
                    &recovery_kek,
                    device_wrap_key.as_ref(),
                )?;
                // The record is sealed under the new epoch (section 5.3, step 3).
                session
                    .ring
                    .push_epoch(rotation.new_epoch, rotation.new_key());
                let data =
                    lock_ops::recovery_record_data(&pending.recovery_key, &format_timestamp(now));
                let write = session.prepare_write(
                    &inner.store,
                    RecordKind::RecoveryKey,
                    record_id,
                    data,
                    now,
                );
                session.ring.pop_epoch(rotation.old_epoch);
                Ok((rotation, write?))
            })
            .map_err(NotAccepted)?;
        let kek_wrap = B64(rotation
            .row(WrapKind::Kek)
            .ok_or(NotAccepted(CoreError::Internal("rotation")))?
            .wrap
            .clone());
        let wire_rotation = wire_rotation(&rotation).map_err(NotAccepted)?;
        let record = wire::RecordWrite {
            base_revision: write.row.synced_revision,
            envelope: B64(write.row.envelope.clone()),
        };
        if let Some(device_id) = revoke {
            // Nothing of the change is sent when the revocation fails.
            // A signed-out device stays "signed out"; otherwise nothing of
            // the change was sent, whatever the revocation's answer was.
            self.send_change(&link, |api, token| api.device_revoke(token, device_id))
                .map_err(|failure| match failure {
                    KeyChangeFailure::SignedOut(error) => KeyChangeFailure::SignedOut(error),
                    other => NotAccepted(other.into_error()),
                })?;
        }
        let sent = match (&pending.new_password, &server.auth_key) {
            (Some(material), Some(auth_key)) => {
                let request = wire::CredentialsChangeRequest {
                    current_auth_key: key_b64(&server.current_auth_key),
                    kdf: wire_kdf(&material.kdf),
                    kdf_salt: B64(material.kdf_salt.to_vec()),
                    auth_key: key_b64(auth_key),
                    kek_wrap,
                    recovery_auth_key: key_b64(&recovery_auth_key),
                    rotation: wire_rotation,
                    record,
                };
                self.send_change(&link, |api, token| api.credentials_change(token, &request))
            }
            (None, None) => {
                let request = wire::RecoveryKeyReissueRequest {
                    current_auth_key: key_b64(&server.current_auth_key),
                    recovery_auth_key: key_b64(&recovery_auth_key),
                    kek_wrap,
                    rotation: wire_rotation,
                    record,
                };
                self.send_change(&link, |api, token| {
                    api.recovery_key_reissue(token, &request)
                })
            }
            _ => return Err(NotAccepted(CoreError::Internal("key change"))),
        };
        let response = sent.map_err(|failure| rolled_back_epoch(failure, rotation.old_epoch))?;
        let mut write = write;
        write.row.synced_revision = write.row.revision;
        write.row.base_envelope = None;
        write.row.server_seq = Some(response.seq as i64);
        let local_auth = pending
            .new_password
            .as_ref()
            .map(|material| material.to_row(link.vault_id, format_timestamp(now)));
        self.with_session(|inner| {
            #[cfg(any(test, feature = "test-hooks"))]
            if fail_store::take() {
                return Err(CoreError::Internal("key change not stored (test hook)"));
            }
            let ids = Self::commit_new_keys(
                inner,
                &link.device_id,
                Some(&rotation),
                None,
                local_auth.as_ref(),
                Some(write),
                &response.tokens,
                link.api.origin(),
                now,
            )?;
            inner.recovery_grant.grant(now);
            if let Err(error) = inner
                .store
                .write(|tx| tx.meta_set(lock_ops::META_RECOVERY_REISSUE, "0"))
            {
                tracing::warn!(code = error.code(), "reissue offer not cleared");
            }
            if device_lost
                && let Err(error) = inner
                    .store
                    .write(|tx| tx.meta_set(lock_ops::META_REMEMBER_LOST, "1"))
            {
                tracing::warn!(code = error.code(), "remember note not stored");
            }
            tracing::info!(
                epoch = rotation.new_epoch,
                password = local_auth.is_some(),
                "recovery key reissued on the server"
            );
            self.records_rewritten(inner, ids);
            Ok(())
        })
        .map_err(KeyChangeFailure::NotStoredHere)?;
        // As in local mode: a keychain item without its wrap goes too.
        if device_lost {
            self.delete_device_key(&context.db_id);
        }
        Ok(())
    }

    /// Local mode: the rotation of section 5.1 with the recovery wrap of
    /// its step 5 made from the new key, the new `recovery_key` record and,
    /// with a new password, its `kek` row and `local_auth`, in one
    /// transaction.
    fn key_change_local(&self, pending: PendingKeyChange) -> Result<(), KeyChangeFailure> {
        use KeyChangeFailure::NotAccepted;
        // Held until the end: the rotation rewraps the `device` row with the
        // device key fetched below, and a lost device key is deleted last.
        let _remember = self.remember_lock();
        let context = &pending.context;
        let device_key = context
            .device_row
            .as_ref()
            .and_then(|_| self.fetch_device_key(&context.db_id));
        let device_lost = context.device_row.is_some() && device_key.is_none();
        let recovery_kek = crypto::derive_recovery_kek(&pending.recovery_key);
        let device_wrap_key = device_key
            .as_ref()
            .map(|key| crypto::derive_device_wrap_key(key, &context.db_id));

        let mut guard = self.inner();
        let inner = &mut *guard;
        let now = self.now();
        let Inner {
            store,
            session,
            state,
            ..
        } = &mut *inner;
        let session = match (state, session.as_mut()) {
            (LockState::Unlocked, Some(session)) => session,
            _ => return Err(NotAccepted(CoreError::Locked)),
        };
        // `kek` came from the rows in `context`: a change committed
        // meanwhile would make the new `kek` row unopenable by any password.
        let unchanged = session.ring.current_epoch() == context.vault.current_epoch
            && lock_ops::key_rows_unchanged(store, context).map_err(NotAccepted)?
            && lock_ops::device_row_unchanged(store, context).map_err(NotAccepted)?;
        if !unchanged {
            return Err(NotAccepted(CoreError::WrongState));
        }
        let recovery_record = session
            .index
            .singleton(RecordKind::RecoveryKey)
            .map(|record| record.record_id)
            .ok_or(NotAccepted(CoreError::Damaged))?;
        let rotation = Rotation::prepare(
            &session.ring,
            &context.vault,
            &pending.kek,
            &recovery_kek,
            device_wrap_key.as_ref(),
        )
        .map_err(NotAccepted)?;
        let vault_id = session.ring.vault_id();
        let local_auth = pending
            .new_password
            .as_ref()
            .map(|material| material.to_row(vault_id, format_timestamp(now)));
        session
            .ring
            .push_epoch(rotation.new_epoch, rotation.new_key());
        let data = lock_ops::recovery_record_data(&pending.recovery_key, &format_timestamp(now));
        let write =
            match session.prepare_write(store, RecordKind::RecoveryKey, recovery_record, data, now)
            {
                Ok(write) => write,
                Err(error) => {
                    session.ring.pop_epoch(rotation.old_epoch);
                    return Err(NotAccepted(error));
                }
            };
        if let Err(error) = store.write(|tx| {
            rotation.write(tx)?;
            if let Some(local_auth) = &local_auth {
                tx.local_auth_put(local_auth)?;
            }
            if device_lost {
                tx.meta_set(lock_ops::META_REMEMBER_LOST, "1")?;
            }
            tx.record_put(&write.row)
        }) {
            session.ring.pop_epoch(rotation.old_epoch);
            return Err(NotAccepted(error.into()));
        }
        // Committed: the old recovery key (and password) no longer work, so
        // from here on nothing fails the command.
        session.commit(vec![write]);
        let mut written = lock_ops::reseal_or_defer(session, store, now);
        written.push(recovery_record);
        tracing::info!(
            epoch = rotation.new_epoch,
            password = local_auth.is_some(),
            "recovery key reissued; vault rotated"
        );
        // The deleted rows open the vault key with the old password and key.
        checkpoint(store);
        // A reissue shows its key; the compromised password change of local
        // mode never does (D29), so it ends any grant to copy the record's
        // key, which is now the new one.
        if local_auth.is_none() {
            inner.recovery_grant.grant(now);
        } else {
            inner.recovery_grant.clear();
        }
        self.emit(
            inner,
            CoreEventKind::VaultsChanged {
                vault_ids: vec![vault_id],
            },
        );
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
        drop(guard);
        if device_lost {
            self.delete_device_key(&context.db_id);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn api_error(status: u16, code: &str) -> HttpError {
        HttpError::Api {
            status,
            code: code.to_owned(),
            details: serde_json::Map::new(),
            retry_after: None,
        }
    }

    /// D180: only an error answer of the server is a rollback; no answer
    /// may have come after the commit.
    #[test]
    fn failures_are_told_apart_by_whether_the_server_answered() {
        for (status, code) in [
            (400, "invalid_envelope"),
            (401, "invalid_credentials"),
            (409, "revision_conflict"),
            (409, "epoch_conflict"),
            (429, "rate_limited"),
        ] {
            assert!(matches!(
                sent_failure(api_error(status, code)),
                KeyChangeFailure::NotAccepted(_)
            ));
        }
        for error in [
            HttpError::Network,
            HttpError::Timeout,
            HttpError::Malformed,
            api_error(408, "http_408"),
            api_error(404, "http_404"),
            api_error(502, "bad_gateway"),
            api_error(500, "internal"),
        ] {
            assert!(matches!(sent_failure(error), KeyChangeFailure::Unknown(_)));
        }
        assert!(matches!(
            refresh_failure(CoreError::Sync(api_error(401, code::REFRESH_TOKEN_REUSED))),
            KeyChangeFailure::SignedOut(_)
        ));
        assert!(matches!(
            refresh_failure(CoreError::Sync(HttpError::Network)),
            KeyChangeFailure::NotAccepted(_)
        ));
        // An epoch conflict below this device's epoch is a rolled-back server.
        let behind = |current: u64| {
            let mut details = serde_json::Map::new();
            details.insert("current_epoch".into(), current.into());
            let failure = KeyChangeFailure::NotAccepted(CoreError::Sync(HttpError::Api {
                status: 409,
                code: code::EPOCH_CONFLICT.to_owned(),
                details,
                retry_after: None,
            }));
            match rolled_back_epoch(failure, 3).into_error() {
                CoreError::Sync(error) => error.code(),
                other => other.code().to_owned(),
            }
        };
        assert_eq!(behind(1), "rolled_back");
        assert_eq!(behind(3), "epoch_conflict", "a concurrent rotation");
    }
}
