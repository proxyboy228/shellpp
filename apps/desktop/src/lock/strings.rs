//! Strings of the lock screens (UI 13.1-13.3, decision D29), catalog `en`.
//! Re-exported as `crate::i18n::lock`.

pub const APP_NAME: &str = crate::i18n::APP_NAME;

// -- First run (UI 13.1) --------------------------------------------------------

pub const FIRST_RUN_TAGLINE: &str = "Your data is stored on this device, encrypted.";
pub const MASTER_PASSWORD: &str = "Master password";
pub const REPEAT_PASSWORD: &str = "Repeat password";
pub const FIRST_RUN_WARNING: &str =
    "If you forget the master password, your data cannot be recovered.";
pub const REMEMBER_DEVICE: &str = "Remember on this device";
pub const REMEMBER_DEVICE_HINT: &str = "The key is kept in the system keychain, so Shell++ opens without the password. Any program running as your user can then open your data.";
pub const CREATE_VAULT: &str = "Create vault";
/// Under a too short master password (also the hint before it is checked).
pub const TOO_SHORT: &str = "At least 8 characters";
pub const PASSWORDS_DIFFER: &str = "Passwords do not match";
/// After "Start with a new vault": where the old data went.
pub fn archived_note(file_name: &str) -> String {
    format!("Your previous data file was kept as {file_name}.")
}

// -- Strength indicator ----------------------------------------------------------

pub const STRENGTH_WEAK: &str = "Weak";
pub const STRENGTH_FAIR: &str = "Fair";
pub const STRENGTH_GOOD: &str = "Good";
pub const STRENGTH_STRONG: &str = "Strong";

/// "Strength: Good".
pub fn strength_label(level: &str) -> String {
    format!("Strength: {level}")
}

// -- Unlock (UI 13.2) ------------------------------------------------------------

pub const UNLOCK_SUBTITLE: &str = "Enter the master password to unlock.";
pub const UNLOCK: &str = "Unlock";
pub const WRONG_PASSWORD: &str = "Wrong password";
pub const FORGOT_PASSWORD: &str = "Forgot password?";
pub const WORKING: &str = "Please wait…";
pub const FAILED: &str = "Something went wrong. Try again.";

/// "Too many attempts. Try again in 0:05."
pub fn retry_in(seconds: u64) -> String {
    format!(
        "Too many attempts. Try again in {}:{:02}.",
        seconds / 60,
        seconds % 60
    )
}

// -- Forgot password (UI 13.2) ---------------------------------------------------

pub const FORGOT_TITLE: &str = "Forgot your master password?";
pub const FORGOT_BODY: &str = "The master password cannot be recovered: it is not stored anywhere, and without it nobody can decrypt your data.";
pub const FORGOT_REMEMBERED: &str =
    "This device remembers your vault: restart Shell++ to open it without the password.";
pub const RESTORE_FROM_SYNC: &str = "Restore from sync";
pub const RESTORE_FROM_SYNC_HINT: &str = "Sync is not set up on this device.";
pub const START_NEW_VAULT: &str = "Start with a new vault";
pub const START_NEW_VAULT_HINT: &str =
    "Creates an empty vault. The current data file is renamed and kept, not deleted.";
pub const CANCEL: &str = "Cancel";
pub const CONFIRM_TITLE: &str = "Start with a new vault?";
pub const CONFIRM_BODY: &str = "Your current data is not deleted: the data file is renamed and kept next to the new one, but only the forgotten password opens it. Shell++ starts with an empty vault.";
pub const CONFIRM_START: &str = START_NEW_VAULT;
