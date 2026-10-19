//! English interface strings (UI 0: the interface is in English; the meaning
//! comes from docs/ui-behavior-spec.md). Views use these constants instead of
//! literals so a later catalog can replace this module in one place.

pub const APP_NAME: &str = "Shell++";

/// "In development" placeholder (sections and tabs not built yet).
pub mod placeholder {
    pub const TITLE: &str = "In development";
    pub const BODY: &str = "This part of Shell++ is not built yet.";
}

/// Top bar (UI 1.1) and app menu (UI 1.4).
pub mod top_bar {
    pub const APP_MENU: &str = "Menu";
    pub const VAULTS: &str = "Vaults";
    pub const SFTP: &str = "SFTP";
    pub const NEW_TAB: &str = "New tab";
    pub const NOTIFICATIONS: &str = "Notifications";
    pub const MENU_SETTINGS: &str = "Settings";
    pub const MENU_LOCK: &str = "Lock";
    pub const MENU_EXIT: &str = "Exit";
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

/// Hosts screen (UI 2).
pub mod hosts {
    pub const SEARCH_PLACEHOLDER: &str = "Search hosts or type user@host to connect";
    pub const CONNECT: &str = "Connect";
    pub const NEW_HOST: &str = "New host";
    pub const NEW_GROUP: &str = "New group";
    pub const IMPORT: &str = "Import";
    pub const TERMINAL: &str = "Terminal";
    pub const SERIAL: &str = "Serial";
    pub const GROUPS: &str = "Groups";
    pub const HOSTS: &str = "Hosts";
    pub const EMPTY_TITLE: &str = "No hosts yet";
    pub const EMPTY_BODY: &str = "Create a host to connect to it.";
    pub const NO_MATCHES: &str = "No hosts match the search";
    pub const HOST_COUNT_ONE: &str = "1 host";
    /// `"{n} hosts"`.
    pub fn host_count(count: usize) -> String {
        if count == 1 {
            HOST_COUNT_ONE.to_owned()
        } else {
            format!("{count} hosts")
        }
    }
}

/// Host and group panels (UI 3).
pub mod panels {
    pub const HOST_DETAILS: &str = "Host details";
    pub const NEW_HOST: &str = "New host";
    pub const GROUP_DETAILS: &str = "Group details";
    pub const NEW_GROUP: &str = "New group";
    pub const CLOSE: &str = "Close panel";
    pub const ADDRESS: &str = "Address";
    pub const ADDRESS_PLACEHOLDER: &str = "IP or hostname";
    pub const LABEL: &str = "Label";
    pub const NAME: &str = "Name";
    pub const CONNECT: &str = "Connect";
    pub const INVALID_VALUE: &str = "This value is not valid; the last valid value is kept.";
}

/// Lock screens (UI 13.1, 13.2).
pub mod lock {
    pub const FIRST_RUN_TITLE: &str = "Create a master password";
    pub const FIRST_RUN_WARNING: &str = "Without the master password and the recovery key, nobody can recover your data, not even a server administrator.";
    pub const PASSWORD: &str = "Password";
    pub const CONFIRM_PASSWORD: &str = "Confirm password";
    pub const CONTINUE: &str = "Continue";
    pub const PASSWORDS_DIFFER: &str = "The passwords do not match.";
    pub const RECOVERY_TITLE: &str = "Save your recovery key";
    pub const RECOVERY_BODY: &str = "Keep this key somewhere safe. It is the only way back in if you forget the master password.";
    pub const COPY: &str = "Copy";
    pub const SAVED_CHECKBOX: &str = "I saved the recovery key";
    pub const FINISH: &str = "Finish";
    pub const MASTER_PASSWORD: &str = "Master password";
    pub const UNLOCK: &str = "Unlock";
    pub const WRONG_PASSWORD: &str = "Wrong password.";
    pub const USE_RECOVERY_KEY: &str = "Use recovery key";
    pub const WORKING: &str = "Please wait…";
    pub const FAILED: &str = "Something went wrong. Try again.";
}

/// Startup failures shown instead of the app.
pub mod startup {
    pub const DATA_DIR_IN_USE: &str =
        "Another Shell++ window is using this data directory. Close it and start Shell++ again.";
    pub const NEWER_SCHEMA: &str =
        "This data was written by a newer version of Shell++. Update Shell++ to open it.";
    pub const OPEN_FAILED: &str = "Shell++ could not open its data directory.";
}
