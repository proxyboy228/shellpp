//! English strings of the Hosts screen (UI 0: the interface is in English;
//! the meaning comes from docs/ui-behavior-spec.md, section 2). The i18n
//! catalog (`crate::i18n`) re-exports this module; views of this module use
//! these constants instead of literals.

/// "In development" placeholder (decision 5 of the audit).
pub const IN_DEVELOPMENT: &str = "In development";

// 2.1 Search and quick connect.
pub const SEARCH_PLACEHOLDER: &str = "Search hosts or type ssh user@hostname";
/// The same hint for a narrow search bar (the minimum window with the
/// details panel open), so it is never cut off.
pub const SEARCH_PLACEHOLDER_SHORT: &str = "Search or ssh user@host";
/// The hint of a search bar too narrow even for the short one.
pub const SEARCH_PLACEHOLDER_SHORTEST: &str = "Search";
pub const CONNECT: &str = "Connect";
pub const SAVE_HOST_OFFER: &str = "Save host?";
pub const SAVE_HOST_ACTION: &str = "Save";
pub const HOST_SAVED: &str = "Host saved";
/// The button of an empty search result whose text is an address.
pub const SAVE_AS_HOST: &str = "Save as host";

/// An empty search result whose text is an address (owner's step 0.2):
/// `"Press Enter to connect to root@db.lan:2222"`.
pub fn press_enter_to_connect(address: &str) -> String {
    format!("Press Enter to connect to {address}")
}

// 2.2 Action bar.
pub const NEW_HOST: &str = "New host";
pub const NEW_HOST_MENU: &str = "More ways to add hosts";
pub const NEW_GROUP: &str = "New Group";
pub const IMPORT: &str = "Import";
pub const TERMINAL: &str = "Terminal";
pub const SERIAL: &str = "Serial";
pub const VIEW_TILES: &str = "Tiles";
pub const VIEW_LIST: &str = "List";
pub const TAG_FILTER: &str = "Filter by tag";
pub const TAG_SEARCH_PLACEHOLDER: &str = "Search tags";
pub const TAG_CLEAR: &str = "Clear";
pub const TAG_NONE: &str = "No tags yet";
pub const TAG_NO_MATCH: &str = "No tags match";
pub const SORT: &str = "Sort";
pub const SORT_MANUAL: &str = "Manual";
pub const SORT_NAME: &str = "Name";
pub const SORT_CREATED: &str = "Date created";
pub const SORT_MODIFIED: &str = "Date modified";

/// The chip of the tags that did not fit: `"+3"`.
pub fn more_tags(hidden: usize) -> String {
    format!("+{hidden}")
}

/// Tooltip of a chip of a tag that is on.
pub fn remove_tag_filter(tag: &str) -> String {
    format!("Stop filtering by “{tag}”")
}

// 2.3 Content.
pub const GROUPS: &str = "Groups";
pub const HOSTS: &str = "Hosts";
pub const BREADCRUMB_SEPARATOR: &str = "›";
pub const PROTOCOL_SSH: &str = "SSH";
pub const PROTOCOL_TELNET: &str = "Telnet";
pub const EMPTY_TITLE: &str = "No hosts yet";
pub const EMPTY_BODY: &str =
    "Add a host to connect to it, or import your hosts from an SSH config file.";
pub const CREATE_HOST: &str = "Create host";
pub const IMPORT_SSH_CONFIG: &str = "Import OpenSSH config";
/// The file Import OpenSSH config reads, in its tooltip.
pub const SSH_CONFIG_PATH: &str = "~/.ssh/config";
pub const EMPTY_GROUP: &str = "This group is empty";
pub const NO_MATCHES: &str = "No hosts match the search";
// The column headers of the list layout (owner's step 0.4).
pub const COLUMN_NAME: &str = "Name";
pub const COLUMN_ADDRESS: &str = "Address";
pub const COLUMN_USER: &str = "User";
pub const COLUMN_TAGS: &str = "Tags";

/// `"1 host"` / `"{n} hosts"` (group card subtitle, UI 2.3).
pub fn host_count(count: usize) -> String {
    if count == 1 {
        "1 host".to_owned()
    } else {
        format!("{count} hosts")
    }
}

// 2.4 Card context menu.
pub const MENU_QUICK_CONNECT: &str = "Quick Connect";
pub const MENU_CONNECT: &str = "Connect";
pub const MENU_VIA_SSH: &str = "via SSH";
pub const MENU_VIA_SFTP: &str = "via SFTP";
pub const MENU_NEW_WINDOW_HINT: &str = "Hold Alt to open in a new window";
pub const MENU_ADD_TO_WORKSPACE: &str = "Add to Workspace";
pub const MENU_EDIT_HOST: &str = "Edit Host Details";
pub const MENU_COLLABORATE: &str = "Collaborate";
pub const MENU_MOVE_TO: &str = "Move to";
pub const MENU_COPY_TO: &str = "Copy to";
pub const MENU_DUPLICATE: &str = "Duplicate";
pub const MENU_COPY_LINK: &str = "Copy Link";
pub const MENU_COPY_LINK_INFO: &str =
    "Opens this host for anyone who has access to its vault in Shell++.";
pub const MENU_REMOVE: &str = "Remove";
pub const MENU_TOP_LEVEL: &str = "Top level";
pub const MENU_VAULT: &str = "Another vault…";
pub const KEY_ENTER: &str = "Enter";
pub const KEY_E: &str = "E";

// Group menu (UI 3.4).
pub const MENU_OPEN: &str = "Open";
pub const MENU_EDIT: &str = "Edit";

// Toasts and dialogs.
pub const LINK_COPIED: &str = "Link copied";
pub const UNDO: &str = "Undo";
pub const MOVE_OR_COPY_TITLE: &str = "Move or copy to a vault";
pub const IMPORT_TITLE: &str = "Import";
pub const REMOVE_TITLE_ONE: &str = "Remove host?";
pub const REMOVE_TITLE_GROUP: &str = "Remove group?";
pub const CANCEL: &str = "Cancel";
pub const CLOSE: &str = "Close";
pub const FAILED: &str = "Something went wrong. Try again.";

/// Title of the removal dialog for `count` records.
pub fn remove_title(count: usize, groups: usize) -> String {
    match (count, groups) {
        (1, 0) => REMOVE_TITLE_ONE.to_owned(),
        (1, 1) => REMOVE_TITLE_GROUP.to_owned(),
        (count, _) => format!("Remove {count} items?"),
    }
}

/// Body of the removal dialog: what happens, and that it can be undone.
pub fn remove_body(name: Option<&str>, count: usize, non_empty_groups: usize) -> String {
    let what = match name {
        Some(name) if count == 1 => format!("“{name}” will be removed."),
        _ => format!("{count} items will be removed."),
    };
    if non_empty_groups > 0 {
        format!(
            "{what} Subgroups and hosts inside will move one level up. You can undo this for a few seconds."
        )
    } else {
        format!("{what} You can undo this for a few seconds.")
    }
}

/// Toast after a removal: `"Removed “web”"` / `"Removed 3 items"`.
pub fn removed(name: Option<&str>, count: usize) -> String {
    match name {
        Some(name) if count == 1 => format!("Removed “{name}”"),
        _ => format!("Removed {count} items"),
    }
}

/// Toast after Copy to: `"Copied to Production"` / `"Copied to the top level"`.
pub fn copied_to(group: Option<&str>) -> String {
    match group {
        Some(group) => format!("Copied to {group}"),
        None => "Copied to the top level".to_owned(),
    }
}

/// Toast after a move: `"Moved to Production"` / `"Moved to the top level"`.
pub fn moved_to(group: Option<&str>) -> String {
    match group {
        Some(group) => format!("Moved to {group}"),
        None => "Moved to the top level".to_owned(),
    }
}

/// Title of a quick-connect session tab: `user@host` or `host`.
pub fn session_title(username: Option<&str>, host: &str) -> String {
    match username {
        Some(user) => format!("{user}@{host}"),
        None => host.to_owned(),
    }
}
