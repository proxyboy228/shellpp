//! English strings of the new tab page, the Ctrl+K overlay and the pane
//! picker (UI 0: the interface is in English; the meaning comes from
//! docs/ui-behavior-spec.md, sections 4.7 and 6).

/// The search field of the page and the overlay.
pub const SEARCH_PLACEHOLDER: &str = "Search hosts and tabs, or type user@hostname";
/// The search field of a new pane (it lists hosts, not tabs).
pub const PANE_SEARCH_PLACEHOLDER: &str = "Search hosts or type user@hostname";
pub const RECENT_CONNECTIONS: &str = "Recent connections";
pub const OPEN_TABS: &str = "Open tabs";
pub const HOSTS: &str = "Hosts";
pub const RESTORE: &str = "Restore";
/// Tooltip of Restore while there is nothing to restore.
pub const RESTORE_NOTHING: &str = "Nothing to restore from the previous launch";
/// Tooltip of Restore while it is enabled.
pub const RESTORE_HINT: &str = "Reopen the windows, tabs and splits of the previous launch";
pub const CREATE_WORKSPACE: &str = "Create a workspace";
/// Tooltip of the disabled "Create a workspace" (section 0: later stages).
pub const WORKSPACES_LATER: &str = "Workspaces arrive in stage 9.";
pub const NO_RECENT: &str = "Hosts you connect to appear here.";
pub const NO_MATCHES: &str = "No hosts or tabs match the search";
pub const PANE_NO_MATCHES: &str = "No hosts match the search";
/// The detail of a local terminal row.
pub const LOCAL_TERMINAL: &str = "Local terminal";
/// Between the parts of "vault / group".
pub const PATH_SEPARATOR: &str = " / ";

/// The row of an address that is not a saved host: "Connect to root@db.lan".
pub fn connect_to(address: &str) -> String {
    format!("Connect to {address}")
}
