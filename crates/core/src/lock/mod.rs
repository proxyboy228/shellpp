//! Lock state machine: first run, unlock (password or device key), lock,
//! recovery unlock, password change, recovery key show and reissue, "remember
//! on this device", the auto-lock timer, the unlock attempt limit and "start
//! with a new vault" (docs/security.md, sections 5.9 and 8; docs/architecture.md,
//! sections 3.3 and 4.1; UI 13.1-13.4; decision D29).
//!
//! This module holds the pieces that do not need the facade's mutex: password
//! rules, the key derivation from a password (Argon2id, slow: never called
//! while the core mutex is held), the auto-lock timer, the attempt limit
//! ([`lockout`]), the archive of the data file ([`new_vault`]), the recovery
//! key grant and the clipboard guard. The state transitions themselves live
//! in [`crate::service::Core`], which owns the store and the key ring.
//!
//! State machine (architecture 3.3):
//!
//! ```text
//! first_run --first_run_setup--> unlocked
//! locked    --unlock / recovery_unlock / device key at launch--> unlocked
//! unlocked  --lock / auto-lock--> locked
//! locked    --start_new_vault (data file archived)--> first_run
//! ```
//!
//! Local mode (D29): the recovery key is still created at first run and kept
//! in the vault (the sync design and the key rotations need its wrap), but
//! the app does not show it; a forgotten master password cannot be
//! recovered locally.
//!
//! Owner: W2.

pub mod lockout;
pub mod new_vault;

pub use lockout::Lockout;

use std::fmt;
use std::time::{Duration, SystemTime};

use secrecy::SecretString;
use zeroize::{Zeroize, ZeroizeOnDrop};

use crate::crypto::{self, KdfParams, Key32, PlatformClass};
use crate::error::{CoreError, ValidationReason};
use crate::store::LocalAuthRow;

/// The recovery key may be copied for 5 minutes after it was shown (plan decision 15).
pub const RECOVERY_GRANT: Duration = Duration::from_secs(5 * 60);
/// Sensitive clipboard content is cleared after 30 seconds (docs/security.md, section 9.4).
pub const CLIPBOARD_CLEAR_AFTER: Duration = Duration::from_secs(30);
/// After a failed attempt to read or clear the clipboard (another program
/// holds it open), clearing is retried at every tick for this long.
pub const CLIPBOARD_RETRY_FOR: Duration = Duration::from_secs(5 * 60);
/// Removed records can be restored for 60 seconds (plan decision 14).
pub const UNDO_WINDOW: Duration = Duration::from_secs(60);

/// A new master password must have at least [`crypto::MIN_PASSWORD_CHARS`]
/// (8) Unicode scalar values after NFC; there are no composition rules (D29).
pub fn check_new_password(password: &SecretString, field: &str) -> Result<(), CoreError> {
    if crypto::password_length_ok(password) {
        Ok(())
    } else {
        Err(CoreError::field(field, ValidationReason::TooShort))
    }
}

/// Options of the first run (UI 13.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FirstRunOptions {
    /// "Remember on this device": store a device key in the OS keychain so
    /// the app unlocks at launch without the password. On by default (D29);
    /// ignored without an OS keychain.
    pub remember_device: bool,
}

impl Default for FirstRunOptions {
    fn default() -> Self {
        Self {
            remember_device: true,
        }
    }
}

/// The salt material of the stored master password (`local_auth`).
#[derive(Clone, PartialEq, Eq)]
pub struct PasswordMaterial {
    pub kdf: KdfParams,
    pub kdf_salt: [u8; 16],
    pub kdf_email: Option<String>,
}

impl PasswordMaterial {
    /// New material for a password set on this platform: the current preset
    /// and a fresh random salt; the email binding is kept.
    pub fn fresh(class: PlatformClass, kdf_email: Option<String>) -> Result<Self, CoreError> {
        Ok(Self {
            kdf: KdfParams::preset(class),
            kdf_salt: crypto::random_array::<16>()?,
            kdf_email,
        })
    }

    /// The material stored in `local_auth` (KDF parameters are checked
    /// against the floor and the platform ceiling before use).
    pub fn from_row(row: &LocalAuthRow) -> Result<Self, CoreError> {
        Ok(Self {
            kdf: KdfParams::from_json(&row.kdf_json).map_err(|_| CoreError::Damaged)?,
            kdf_salt: row.kdf_salt,
            kdf_email: row.kdf_email.clone(),
        })
    }

    /// Argon2id, then HKDF to `kek`; `master_key` is wiped when it goes out of
    /// scope. Slow (about a second): never call it while holding the core mutex.
    pub fn derive_kek(
        &self,
        password: &SecretString,
        class: PlatformClass,
    ) -> Result<Key32, CoreError> {
        // Stored parameters outside the floor or the platform ceiling are
        // refused before any derivation (docs/security.md, section 5.4).
        self.kdf.check(class).map_err(|_| CoreError::Damaged)?;
        let mut salt = crypto::argon2_salt(self.kdf_email.as_deref(), &self.kdf_salt);
        let master_key = crypto::derive_master_key(password, &self.kdf, &salt, class);
        salt.zeroize();
        let master_key = master_key?;
        Ok(crypto::derive_kek(&master_key))
    }

    /// The `local_auth` row for this material.
    pub fn to_row(&self, personal_vault_id: uuid::Uuid, updated_at: String) -> LocalAuthRow {
        LocalAuthRow {
            personal_vault_id,
            kdf_json: self.kdf.to_json(),
            kdf_salt: self.kdf_salt,
            kdf_email: self.kdf_email.clone(),
            updated_at,
        }
    }
}

impl fmt::Debug for PasswordMaterial {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PasswordMaterial")
            .field("kdf", &self.kdf)
            .finish_non_exhaustive()
    }
}

/// The auto-lock timer (docs/security.md, section 8): wall-clock time since the
/// last keyboard or mouse input, so that sleep counts as inactivity.
#[derive(Clone, Copy, Debug)]
pub struct AutoLock {
    last_activity: SystemTime,
}

impl AutoLock {
    pub fn new(now: SystemTime) -> Self {
        Self { last_activity: now }
    }

    /// Input in an app window.
    pub fn activity(&mut self, now: SystemTime) {
        self.last_activity = now;
    }

    pub fn last_activity(&self) -> SystemTime {
        self.last_activity
    }

    /// `true` when the app must lock now. A clock that went backwards resets
    /// the last activity to `now`. `None` = never.
    pub fn due(&mut self, now: SystemTime, interval: Option<Duration>) -> bool {
        let Some(interval) = interval else {
            return false;
        };
        match now.duration_since(self.last_activity) {
            Ok(idle) => idle >= interval,
            Err(_) => {
                self.last_activity = now;
                false
            }
        }
    }
}

/// Permission to copy the recovery key with `secret_copy` (plan decision 15).
#[derive(Clone, Copy, Debug, Default)]
pub struct RecoveryGrant {
    until: Option<SystemTime>,
}

impl RecoveryGrant {
    /// Starts a 5-minute grant.
    pub fn grant(&mut self, now: SystemTime) {
        self.until = now.checked_add(RECOVERY_GRANT);
    }

    pub fn allowed(&self, now: SystemTime) -> bool {
        self.until.is_some_and(|until| now < until)
    }

    pub fn clear(&mut self) {
        self.until = None;
    }
}

/// Remembers a sensitive value copied to the clipboard by its SHA-256, so it
/// is cleared after 30 seconds, on lock and on exit only if the clipboard still
/// holds it (docs/security.md, section 9.4).
///
/// The unsalted hash of a weak password is a fast offline guess, so it is
/// wiped on drop and the guard is neither `Copy` nor `Clone`: keep it in one
/// place and drop it there (assigning `None` to an `Option` holding it drops
/// it in place; `Option::take` would leave the hash behind in the old slot).
pub struct ClipboardGuard {
    hash: [u8; 32],
    deadline: SystemTime,
    /// The first failed attempt to read or clear the clipboard.
    failed_since: Option<SystemTime>,
}

impl ClipboardGuard {
    pub fn new(text: &str, now: SystemTime) -> Self {
        Self {
            hash: crypto::sha256(text.as_bytes()),
            deadline: now.checked_add(CLIPBOARD_CLEAR_AFTER).unwrap_or(now),
            failed_since: None,
        }
    }

    /// `true` when the clipboard must be cleared now: 30 seconds have passed,
    /// or an earlier attempt failed and is retried.
    pub fn expired(&self, now: SystemTime) -> bool {
        now >= self.deadline || self.failed_since.is_some()
    }

    /// Records a failed attempt. `true` while the guard is to be retried at
    /// every tick, that is until [`CLIPBOARD_RETRY_FOR`] has passed since the
    /// first failure; `false` when it is given up and should be dropped.
    pub fn record_failure(&mut self, now: SystemTime) -> bool {
        let since = *self.failed_since.get_or_insert(now);
        !matches!(now.duration_since(since), Ok(elapsed) if elapsed >= CLIPBOARD_RETRY_FOR)
    }

    /// `true` when `current` is the copied value.
    pub fn matches(&self, current: &str) -> bool {
        let mut current_hash = crypto::sha256(current.as_bytes());
        let same = crypto::ct_eq(&current_hash, &self.hash);
        current_hash.zeroize();
        same
    }
}

impl Drop for ClipboardGuard {
    fn drop(&mut self) {
        self.hash.zeroize();
    }
}

impl ZeroizeOnDrop for ClipboardGuard {}

impl fmt::Debug for ClipboardGuard {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ClipboardGuard")
            .field("deadline", &self.deadline)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::UNIX_EPOCH;

    fn at(secs: u64) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(secs)
    }

    #[test]
    fn password_length_rule() {
        assert!(check_new_password(&SecretString::from("1234567"), "password").is_err());
        assert!(check_new_password(&SecretString::from("12345678"), "password").is_ok());
        // No composition rules: eight spaces or one repeated letter are fine.
        assert!(check_new_password(&SecretString::from("        "), "password").is_ok());
        assert!(check_new_password(&SecretString::from("aaaaaaaa"), "password").is_ok());
        match check_new_password(&SecretString::from("short"), "new_password") {
            Err(CoreError::Validation(errors)) => {
                assert_eq!(errors[0].field, "new_password");
                assert_eq!(errors[0].reason, ValidationReason::TooShort);
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn auto_lock_uses_wall_clock() {
        let mut timer = AutoLock::new(at(1000));
        let minute = Some(Duration::from_secs(60));
        assert!(!timer.due(at(1059), minute));
        assert!(timer.due(at(1060), minute));
        assert!(!timer.due(at(5000), None));
        timer.activity(at(1050));
        assert!(!timer.due(at(1100), minute));
        assert!(timer.due(at(1110), minute));
        // The clock went backwards: the timer restarts from now.
        assert!(!timer.due(at(900), minute));
        assert_eq!(timer.last_activity(), at(900));
        assert!(timer.due(at(960), minute));
    }

    #[test]
    fn recovery_grant_expires_after_five_minutes() {
        let mut grant = RecoveryGrant::default();
        assert!(!grant.allowed(at(0)));
        grant.grant(at(100));
        assert!(grant.allowed(at(399)));
        assert!(!grant.allowed(at(400)));
        grant.grant(at(100));
        grant.clear();
        assert!(!grant.allowed(at(101)));
    }

    #[test]
    fn clipboard_guard_matches_only_the_copied_value() {
        let guard = ClipboardGuard::new("hunter2-canary", at(100));
        assert!(!guard.expired(at(129)));
        assert!(guard.expired(at(130)));
        assert!(guard.matches("hunter2-canary"));
        assert!(!guard.matches("something else"));
        assert!(!format!("{guard:?}").contains("hunter2"));
    }

    #[test]
    fn clipboard_guard_retries_for_five_minutes_after_a_failure() {
        let mut guard = ClipboardGuard::new("hunter2-canary", at(100));
        // A failed attempt on lock, before the 30 seconds: due at once.
        assert!(guard.record_failure(at(110)), "retried");
        assert!(guard.expired(at(110)));
        assert!(guard.matches("hunter2-canary"));
        assert!(guard.record_failure(at(111)), "retried");
        // Counted from the first failure, not the latest one.
        assert!(guard.record_failure(at(409)), "still retried");
        assert!(!guard.record_failure(at(410)), "given up");
    }

    #[test]
    fn clipboard_guard_wipes_its_hash_on_drop() {
        fn wiped_on_drop<T: ZeroizeOnDrop>() {}
        wiped_on_drop::<ClipboardGuard>();
    }
}
