//! The English string catalog `en` (UI 0: the interface is in English; the
//! meaning comes from docs/ui-behavior-spec.md). Views use these constants
//! instead of literals so a later catalog can replace this module in one
//! place, and UI tests find elements by id, never by these texts.
//!
//! This module is the aggregator: the window frame's strings live here; a
//! screen module keeps its strings in its own `strings.rs`, re-exported here
//! under the screen's name.

pub const APP_NAME: &str = "Shell++";

/// "In development" placeholder (UI 0; audit decision 5): screens, tabs and
/// actions that are not built yet.
pub mod placeholder {
    pub const TITLE: &str = "In development";
    pub const BODY: &str = "This part of Shell++ is not built yet.";

    /// Toast text for an action that is not built yet: `"<feature>: In development"`.
    pub fn feature(feature: &str) -> String {
        format!("{feature}: {TITLE}")
    }
}

/// Top bar (UI 1.1).
pub mod top_bar {
    pub const APP_MENU: &str = "Menu";
    pub const VAULTS: &str = "Vaults";
    pub const SFTP: &str = "SFTP";
    pub const SWITCH_VAULT: &str = "Switch vault";
    pub const PERSONAL_VAULT: &str = "Personal";
    pub const NEW_TAB: &str = "New tab";
    pub const CLOSE_TAB: &str = "Close tab";
    pub const NOTIFICATIONS: &str = "Notifications";
    pub const TERMINAL_SIDEBAR: &str = "Terminal sidebar";
    // Kept for older callers; the app menu strings are in `menu`.
    pub const MENU_SETTINGS: &str = super::menu::SETTINGS;
    pub const MENU_LOCK: &str = super::menu::LOCK;
    pub const MENU_EXIT: &str = super::menu::EXIT;
}

/// App menu (UI 1.4). Shortcut hints come from the key bindings.
pub mod menu {
    pub const VAULTS: &str = "Vaults";
    pub const SFTP: &str = "SFTP";
    pub const SETTINGS: &str = "Settings";
    pub const NEW_LOCAL_TERMINAL: &str = "New Local Terminal";
    pub const NEW_SERIAL_CONNECTION: &str = "New Serial Connection";
    pub const IMPORT_DATA: &str = "Import Data…";
    pub const EXPORT_DATA: &str = "Export Data…";
    pub const EDIT: &str = "Edit";
    pub const UNDO: &str = "Undo";
    pub const REDO: &str = "Redo";
    pub const CUT: &str = "Cut";
    pub const COPY: &str = "Copy";
    pub const PASTE: &str = "Paste";
    pub const SELECT_ALL: &str = "Select All";
    pub const VIEW: &str = "View";
    pub const ZOOM_IN: &str = "Zoom In";
    pub const ZOOM_OUT: &str = "Zoom Out";
    pub const ACTUAL_SIZE: &str = "Actual Size";
    pub const FULL_SCREEN: &str = "Toggle Full Screen";
    pub const TOGGLE_SIDEBAR: &str = "Show or Hide Sidebar";
    pub const INTERFACE_ZOOM: &str = "Interface zoom";
    pub const WINDOW: &str = "Window";
    pub const NEW_WINDOW: &str = "New Window";
    pub const NEXT_TAB: &str = "Next Tab";
    pub const PREVIOUS_TAB: &str = "Previous Tab";
    pub const CLOSE_TAB: &str = "Close Tab";
    pub const HELP: &str = "Help";
    pub const DOCUMENTATION: &str = "Documentation";
    pub const REPORT_ISSUE: &str = "Report an Issue";
    pub const CHECK_FOR_UPDATES: &str = "Check for Updates";
    pub const ABOUT: &str = "About Shell++";
    pub const LOCK: &str = "Lock";
    pub const EXIT: &str = "Exit";

    /// "Shell++ 0.1.0".
    pub fn about_text() -> String {
        format!("{} {}", super::APP_NAME, env!("CARGO_PKG_VERSION"))
    }
}

/// Exit confirmation (UI 1.4, 4.6).
pub mod exit {
    pub const TITLE: &str = "Exit Shell++?";
    pub const CANCEL: &str = "Cancel";
    pub const EXIT: &str = "Exit";

    /// "2 sessions are open. They will be closed."
    pub fn body(sessions: usize) -> String {
        if sessions == 1 {
            "1 session is open. It will be closed.".to_owned()
        } else {
            format!("{sessions} sessions are open. They will be closed.")
        }
    }
}

/// Session tabs (UI 1.1, 6).
pub mod sessions {
    pub const SSH: &str = "SSH";
    pub const SFTP: &str = "SFTP";
    pub const LOCAL_TERMINAL: &str = "Local terminal";
    pub const SERIAL: &str = "Serial";
    pub const NEW_TAB: &str = "New Tab";
}

/// Notification center (UI 1.5) and toasts.
pub mod notifications {
    pub const TITLE: &str = "Notifications";
    pub const EMPTY: &str = "No notifications";
    pub const DISMISS: &str = "Dismiss";
    pub const UNDO: &str = "Undo";

    /// Accessible label of the bell: "Notifications, 3 unread".
    pub fn bell_label(unread: usize) -> String {
        if unread == 0 {
            TITLE.to_owned()
        } else {
            format!("{TITLE}, {unread} unread")
        }
    }

    /// The counter on the bell ("9+" above nine).
    pub fn counter(unread: usize) -> String {
        if unread > 9 {
            "9+".to_owned()
        } else {
            unread.to_string()
        }
    }
}

/// Details panel frame (UI 1.3).
pub mod details {
    pub const MORE_ACTIONS: &str = "More actions";
    pub const COLLAPSE: &str = "Collapse panel";
}

/// Sidebar sections (UI 1.2).
pub mod nav {
    pub const HOSTS: &str = "Hosts";
    pub const KEYCHAIN: &str = "Keychain";
    pub const PORT_FORWARDING: &str = "Port Forwarding";
    pub const SNIPPETS: &str = "Snippets";
    pub const KNOWN_HOSTS: &str = "Known Hosts";
    pub const LOGS: &str = "Logs";
}

/// Hosts screen (UI 2): `crate::hosts::strings`.
pub use crate::hosts::strings as hosts;

/// Host and group panels (UI 1.3, 3): `crate::panels::strings`.
pub use crate::panels::strings as panels;

/// Lock screens (UI 13.1-13.3): `crate::lock::strings`.
pub use crate::lock::strings as lock;

/// Settings window (UI 12.1, 13.4): `crate::settings::strings`.
pub use crate::settings::strings as settings;

/// Startup failures shown instead of the app.
pub mod startup {
    pub const DATA_DIR_IN_USE: &str =
        "Another Shell++ window is using this data directory. Close it and start Shell++ again.";
    pub const NEWER_SCHEMA: &str =
        "This data was written by a newer version of Shell++. Update Shell++ to open it.";
    pub const OPEN_FAILED: &str = "Shell++ could not open its data directory.";
}
