//! Strings of the Settings window (UI 12.1, 13.4), catalog `en`.
//! Re-exported as `crate::i18n::settings`.

pub const TITLE: &str = "Settings";

// Navigation (UI 12.1).
pub const ACCOUNT_SYNC: &str = "Account & Sync";
pub const SECURITY: &str = "Security";
pub const SSH_ID: &str = "SSH ID";
pub const TEAM: &str = "Team";
pub const TERMINAL: &str = "Terminal";
pub const SFTP: &str = "SFTP";
pub const LOGS: &str = "Logs";
pub const SHORTCUTS: &str = "Shortcuts";
pub const UPDATES: &str = "Updates";

// Security (UI 13.4).
pub const AUTO_LOCK: &str = "Auto-lock";
pub const AUTO_LOCK_HINT: &str =
    "Lock Shell++ after this time without keyboard or mouse input in its windows.";
pub const NEVER: &str = "Never";
pub const REMEMBER_DEVICE: &str = "Remember on this device";
pub const REMEMBER_DEVICE_HINT: &str = crate::lock::strings::REMEMBER_DEVICE_HINT;
pub const FAILED: &str = "The setting was not saved. Try again.";

/// "5 minutes", "1 hour" or "Never".
pub fn auto_lock_label(minutes: Option<u64>) -> String {
    match minutes {
        None => NEVER.to_owned(),
        Some(60) => "1 hour".to_owned(),
        Some(1) => "1 minute".to_owned(),
        Some(minutes) => format!("{minutes} minutes"),
    }
}

/// "Shell++ 0.1.0" at the bottom of the navigation.
pub fn version() -> String {
    format!("{} {}", crate::i18n::APP_NAME, env!("CARGO_PKG_VERSION"))
}
