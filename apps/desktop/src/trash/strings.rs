//! The texts of the Recently deleted screen (UI 14).

pub const HINT: &str = "Deleted objects stay here for 90 days, on this device only.";
pub const SEARCH_PLACEHOLDER: &str = "Search by name";

// The action bar.
pub const RESTORE: &str = "Restore";
pub const EXPORT: &str = "Export…";
pub const DELETE_FOREVER: &str = "Delete forever";
pub const EMPTY_TRASH: &str = "Empty trash";
pub const TYPE_FILTER: &str = "Filter by type";
pub const ALL_TYPES: &str = "All types";

// The table.
pub const NAME: &str = "Name";
pub const TYPE: &str = "Type";
pub const VAULT: &str = "Vault";
pub const DELETED: &str = "Deleted";
pub const WHERE: &str = "Where";
pub const THIS_DEVICE: &str = "This device";
pub const OTHER_DEVICE: &str = "Another device";
pub const OTHER_VAULT: &str = "Other vault";

// Empty states and failures.
pub const EMPTY: &str = "Nothing deleted in the last 90 days";
pub const NO_MATCH: &str = "No deleted objects match";
pub const LOAD_FAILED: &str = "Recently deleted could not be read.";
pub const ACTION_FAILED: &str = "The action failed. Nothing was changed.";

// The details panel.
pub const PANEL_TITLE: &str = "Deleted object";
pub const PANEL_READ_ONLY: &str = "Read only. Restore it to change it.";
pub const FIELDS: &str = "Fields";

// Confirmations (UI 14: they name the count).
pub const CANCEL: &str = "Cancel";
pub const DELETE_TITLE: &str = "Delete forever?";
pub const EMPTY_TITLE: &str = "Empty trash?";

pub fn delete_body(count: usize) -> String {
    format!(
        "{} will be removed from this device. Other devices keep their own copies. This cannot be undone.",
        objects(count)
    )
}

pub fn empty_body(count: usize) -> String {
    format!(
        "All {} here will be removed from this device. Other devices keep their own copies. This cannot be undone.",
        objects(count)
    )
}

pub fn restored(count: usize) -> String {
    format!("{} restored", capitalized(&objects(count)))
}

pub fn deleted_forever(count: usize) -> String {
    format!("{} deleted forever", capitalized(&objects(count)))
}

/// "1 object", "3 objects".
pub fn objects(count: usize) -> String {
    if count == 1 {
        "1 object".to_owned()
    } else {
        format!("{count} objects")
    }
}

fn capitalized(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// The type column (UI 14): the words of the spec for the core's type
/// names; an unknown one shows as it is, underscores as spaces.
pub fn type_label(type_name: &str) -> String {
    match type_name {
        "host" => "Host",
        "group" => "Group",
        "key" => "Key",
        "identity" => "Identity",
        "snippet" => "Snippet",
        "snippet_package" => "Snippet package",
        "forwarding_rule" => "Forwarding rule",
        "known_host" => "Known host",
        "workspace" => "Workspace",
        "terminal_theme" => "Terminal theme",
        "highlight_rule" => "Highlighting rule",
        "credential_binding" => "Credential binding",
        "connection_log_entry" => "Log entry",
        other => return capitalized(&other.replace('_', " ")),
    }
    .to_owned()
}
