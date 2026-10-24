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
pub const AUTO_UPDATE_CHECK: &str = "Check for updates at launch";
pub const AUTO_UPDATE_CHECK_HINT: &str = "A check is a plain request for the update manifest, without any identifiers, at most once a day. Nothing is downloaded or installed: an update found is a notice with its release page.";
pub const UPDATE_SOURCE: &str = "Update source";
pub const UPDATE_SOURCE_HINT: &str = "The update manifest (latest.json) of the project's releases.";
pub const UPDATE_SOURCE_INVALID: &str = "Use an https:// address";
pub const UPDATE_SOURCE_RESET: &str = "Reset to Default";
pub const UPDATE_CHECK_NOW: &str = "Check Now";

// Account & Sync (UI 12.2; D67).
pub const SYNC_SERVER: &str = "Sync server";
pub const SYNC_SERVER_HINT: &str =
    "Shell++ works without an account. A sync server arrives in a later version.";
pub const CONNECT_SYNC: &str = "Connect sync server";
pub const DATA: &str = "Your data";
pub const DATA_HINT: &str = "Export everything to a file, or bring a file back.";
pub const EXPORT_DATA: &str = "Export Data...";
pub const IMPORT_DATA: &str = "Import Data...";

// SFTP (UI 12.6).
pub const SHOW_HIDDEN: &str = "Show hidden files";
pub const SHOW_HIDDEN_HINT: &str = "Files and folders whose names start with a dot.";
pub const CONFIRM_DELETE: &str = "Confirm deletion";
pub const CONFIRM_DELETE_HINT: &str = "Ask before files and folders are deleted.";
pub const NAME_CONFLICT: &str = "When a file with the same name exists";
pub const CONFLICT_ASK: &str = "Ask";
pub const CONFLICT_REPLACE: &str = "Replace";
pub const CONFLICT_SKIP: &str = "Skip";
pub const CONFLICT_RENAME: &str = "Rename";
pub const PARALLEL: &str = "Transfers at the same time";
pub const OPEN_PROGRAM: &str = "Program for Open";
pub const OPEN_PROGRAM_HINT: &str =
    "Empty: the program the system assigns to the file type. Open with always asks.";
pub const OPEN_PROGRAM_PLACEHOLDER: &str = "The system's program";

// Logs (UI 12.6).
pub const RETENTION: &str = "Keep the connection log";
pub const RETENTION_HINT: &str = "Older entries are removed. Saved entries are kept.";
pub const RECORD_OUTPUT: &str = "Record session output";
pub const RECORD_OUTPUT_HINT: &str = "Keeps what servers print in each session, encrypted on this device and never synced. A host can decide for itself (its Record output). Output can contain secrets.";
pub const RECORD_TITLE: &str = "Record session output?";
pub const RECORD_BODY: &str = "Everything the servers print is kept, also passwords, keys and tokens a program prints or a command echoes. Recordings are encrypted on this device, never synced, and go with their log entry.";
pub const RECORD_CONFIRM: &str = "Record output";
pub const CLEAR_LOG: &str = "Clear the log";
pub const PRESET: &str = "Set of keys";
pub const PRESET_PC: &str = "PC";
pub const PRESET_MAC: &str = "Mac";
pub const PRESET_HINT: &str =
    "PC uses Ctrl (Ctrl+Shift in the terminal), Mac uses Cmd. Keys you set yourself stay.";
pub const GROUP_APP: &str = "App";
pub const GROUP_TERMINAL: &str = "Terminal";
pub const CLEAR_LOG_HINT: &str = "Removes every entry of this device, saved ones too.";
pub const CLEAR_LOG_TITLE: &str = "Clear the connection log?";
pub const CLEAR_LOG_BODY: &str = "Every entry of this device is removed, saved ones too.";
pub const CANCEL: &str = "Cancel";

/// "90 days".
pub fn days(days: u64) -> String {
    match days {
        365 => "1 year".to_owned(),
        days => format!("{days} days"),
    }
}

/// "3 entries removed".
pub fn cleared(count: usize) -> String {
    match count {
        1 => "1 entry removed".to_owned(),
        count => format!("{count} entries removed"),
    }
}

// Shortcuts (UI 12.5; D67).
pub const SHORTCUTS_HINT: &str = "Click the keys of an action, then press the new ones with Ctrl, Alt or an F-key; \
     Esc cancels. While a terminal has the focus, keys without Shift (such as Ctrl+L or Ctrl+K) go to the shell.";
pub const PRESS_KEYS: &str = "Press keys...";
pub const UNASSIGNED: &str = "Unassigned";
pub const UNASSIGN: &str = "Leave unassigned";
pub const RESET_SHORTCUT: &str = "Back to the default";
pub const RESET_ALL: &str = "Reset all";
pub const CONFLICT: &str = "Another action, or the terminal, has the same keys";

// Security (UI 13.4).
pub const AUTO_LOCK: &str = "Auto-lock";
pub const AUTO_LOCK_HINT: &str =
    "Lock Shell++ after this time without keyboard or mouse input in its windows.";
pub const NEVER: &str = "Never";
pub const SYSTEM_AGENT_WHILE_LOCKED: &str = "Forward the system agent while locked";
pub const SYSTEM_AGENT_WHILE_LOCKED_HINT: &str = "While Shell++ is locked, hosts with Agent \
     Forwarding get no signatures. Turned on, hosts with \"Also offer the system agent's \
     keys\" can still ask your system agent to sign while Shell++ is locked.";
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
pub const HIGHLIGHTING: &str = "Keyword highlighting";
pub const HIGHLIGHTING_HINT: &str = "Color errors, warnings, OK, info and debug words, IP and MAC \
     addresses in plain output. Programs' own colors and full-screen programs are left as they are.";
pub const MIN_CONTRAST: &str = "Adjust indistinguishable colors";
pub const MIN_CONTRAST_HINT: &str = "When text is hard to tell from its background, make it lighter or darker until it is readable.";
pub const MIN_CONTRAST_OFF: &str = "Off";
pub const MIN_CONTRAST_PALETTE: &str = "Palette only";
pub const MIN_CONTRAST_PALETTE_HINT: &str =
    "Theme colors only; exact colors set by programs stay as they are";
pub const MIN_CONTRAST_ALWAYS: &str = "Always";
pub const MIN_CONTRAST_ALWAYS_HINT: &str = "Also exact colors set by programs";
pub const GENERATION: &str = "Script generation (beta)";
pub const GENERATION_HINT: &str = "Writes a snippet's script from a description, with an \
     OpenAI-compatible service you choose. A description is sent only when you press Generate \
     in the snippet form; the script is never run by itself.";
pub const GENERATION_URL: &str = "Address";
pub const GENERATION_URL_INVALID: &str = "Use https://, or http:// on this computer.";
pub const GENERATION_MODEL: &str = "Model";
pub const GENERATION_MODEL_PLACEHOLDER: &str = "gpt-4o-mini, llama3.1 …";
pub const GENERATION_KEY: &str = "API key";
pub const GENERATION_KEY_SAVE: &str = "Save key";
pub const GENERATION_KEY_HINT: &str = "Kept in the system's credential store for this address's \
     host, and sent only there. Leave it empty for a local model.";
pub const GENERATION_KEY_ADDRESS: &str = "Fix the address first: the key is kept for its host.";
pub const GENERATION_KEY_SAVED: &str = "The key is saved in the system's credential store.";
pub const GENERATION_KEY_REMOVED: &str = "The saved key is removed.";
pub const GENERATION_KEY_FAILED: &str = "The system's credential store did not take the key.";
pub const AUTOCOMPLETE: &str = "Autocomplete (beta)";
pub const AUTOCOMPLETE_HINT: &str = "Suggests the rest of a command from this host's history while you type; Tab or → accepts it. Never at a password prompt or in full-screen programs.";
pub const AUTOCOMPLETE_OFF: &str = "Off";
pub const AUTOCOMPLETE_HISTORY: &str = "This host's history";
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

// Keyword highlighting rules (UI 12.4, D77).
pub const HL_CATEGORIES: &str = "Categories";
pub const HL_CATEGORIES_HINT: &str = "The color is shown on dark terminal backgrounds; a lighter or darker shade of it on light ones.";
pub const HL_CUSTOM: &str = "Your rules";
pub const HL_CUSTOM_HINT: &str = "A regular expression and its color. Your rules come before the categories; where two match the same text, the first wins.";
pub const HL_PATTERN_PLACEHOLDER: &str = "timeout|refused";
pub const HL_ANY_CASE: &str = "Any case";
pub const HL_ADD: &str = "Add rule";
pub const HL_INVALID: &str =
    "Enter a regular expression that compiles and does not match empty text.";
pub const HL_TOO_LONG: &str = "The expression is longer than 512 characters.";
pub const HL_TOO_MANY: &str = "There are 64 rules already. Remove one first.";
pub const HL_SAVE_FAILED: &str = "The rule could not be saved.";
pub const HL_ERROR: &str = "Error";
pub const HL_WARNING: &str = "Warning";
pub const HL_OK: &str = "OK";
pub const HL_INFO: &str = "Info";
pub const HL_DEBUG: &str = "Debug";
pub const HL_ADDRESS: &str = "IP and MAC addresses";
pub const REMEMBER_DEVICE_LOST: &str = "Remember on this device was turned off: your vault got a new key, and this device's key was no longer in the system's credential store. Turn it on again to unlock at launch without the password.";
