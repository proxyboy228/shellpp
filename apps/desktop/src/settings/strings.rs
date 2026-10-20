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

// Terminal (UI 12.4; stage 2a lead decision 12).
pub const TERMINAL_BLOCK: &str = "Terminal";
pub const AUTORECONNECT: &str = "Autoreconnect";
pub const AUTORECONNECT_HINT: &str = "After a connection drop, try to reconnect after 1, 2, 4, 8, 16 and 32 seconds, then every minute.";
pub const RECORD_HISTORY: &str = "Record command history";
pub const RECORD_HISTORY_HINT: &str = "Keep the commands you run on saved hosts in the History tab, encrypted. Password prompts, full-screen programs and commands that look like secrets are never recorded. A host can override this.";
pub const COPY_ON_SELECT: &str = "Copy on select";
pub const COPY_ON_SELECT_HINT: &str =
    "Copy text as soon as you select it with the mouse. Right-click always pastes.";
pub const BELL_SOUND: &str = "Bell sound";
pub const BELL_SOUND_HINT: &str = "Play the system sound when a program rings the terminal bell.";
pub const BOLD_IS_BRIGHT: &str = "Bright colors for bold text";
pub const BOLD_IS_BRIGHT_HINT: &str = "Show bold text in the bright variant of its color.";
pub const MIN_CONTRAST: &str = "Adjust indistinguishable colors";
pub const MIN_CONTRAST_HINT: &str = "When text is hard to tell from its background, make it lighter or darker until it is readable.";
pub const MIN_CONTRAST_OFF: &str = "Off";
pub const MIN_CONTRAST_PALETTE: &str = "Palette only";
pub const MIN_CONTRAST_PALETTE_HINT: &str =
    "Theme colors only; exact colors set by programs stay as they are";
pub const MIN_CONTRAST_ALWAYS: &str = "Always";
pub const MIN_CONTRAST_ALWAYS_HINT: &str = "Also exact colors set by programs";
pub const TERM: &str = "Terminal emulation type";
pub const TERM_HINT: &str = "The TERM value sent to the server when a session starts.";
pub const DEFAULT_CHOICE: &str = "Default";
pub const FONT_BLOCK: &str = "Font";
pub const FONT: &str = "Font";
pub const FONT_BUNDLED: &str = "Bundled with Shell++";
pub const FONT_SEARCH_PLACEHOLDER: &str = "Search fonts";
pub const FONTS_LOADING: &str = "Looking for installed monospace fonts...";
pub const FONT_SIZE: &str = "Text size";
pub const LINE_HEIGHT: &str = "Line spacing";
pub const THEME_BLOCK: &str = "Terminal theme";
pub const THEME_HINT: &str = "The theme of hosts that do not set their own.";
/// The mark of themes added in this release (UI 4.2).
pub const THEME_NEW: &str = "New";
/// The sample text of a theme preview.
pub const THEME_PREVIEW_SAMPLE: &str = "~ $ ls";
pub const KEEPALIVE_BLOCK: &str = "Keepalive";
pub const KEEPALIVE_HINT: &str = "How often to send an SSH keepalive, so that a dropped connection is noticed in time. 0 turns keepalive off. A host can set its own interval.";
pub const KEEPALIVE_INTERVAL: &str = "Interval";
pub const SECONDS: &str = "seconds";

/// "Enter a whole number from 6 to 72."
pub fn font_size_range(min: u32, max: u32) -> String {
    format!("Enter a whole number from {min} to {max}.")
}

/// "Enter a number from 1.0 to 2.0."
pub fn line_height_range(min: f32, max: f32) -> String {
    format!("Enter a number from {min:.1} to {max:.1}.")
}

/// "Enter a whole number from 0 to 3600 (0 = off)."
pub fn keepalive_range(max: u32) -> String {
    format!("Enter a whole number from 0 to {max} (0 = off).")
}

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
