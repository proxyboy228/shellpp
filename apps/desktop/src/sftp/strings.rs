//! Strings of the SFTP tab (UI 5), catalog `en`.

// Empty panel (UI 5.2).
pub const EMPTY_TITLE: &str = "Connect to a host";
pub const EMPTY_TEXT: &str = "Its files open here, next to the other panel.";
pub const CHOOSE_HOST: &str = "Choose host…";
pub const PICKER_SEARCH: &str = "Search hosts";
pub const PICKER_EMPTY: &str = "No saved host matches.";
pub const PICKER_NO_HOSTS: &str = "There are no saved hosts yet.";

// Header (UI 5.3).
pub const LOCAL: &str = "Local";
pub const FILTER: &str = "Filter";
pub const ACTIONS: &str = "Actions";
pub const BACK: &str = "Back";
pub const FORWARD: &str = "Forward";
pub const REFRESH: &str = "Refresh";
pub const DRIVES: &str = "Drives";
pub const SOURCE_LOCAL: &str = "Local files";
pub const SOURCE_HOST: &str = "Host…";

// Columns (UI 5.3).
pub const COLUMN_NAME: &str = "Name";
pub const COLUMN_MODIFIED: &str = "Date Modified";
pub const COLUMN_SIZE: &str = "Size";
pub const COLUMN_KIND: &str = "Kind";
pub const PARENT_ROW: &str = "..";

// States of a panel.
pub const LOADING: &str = "Loading…";
pub const FOLDER_EMPTY: &str = "This folder is empty.";
pub const NOTHING_MATCHES: &str = "No file matches the filter.";
pub const CONNECTING: &str = "Connecting";
pub const RECONNECT: &str = "Reconnect";
pub const RETRY: &str = "Retry";
pub const CLOSE: &str = "Close";
pub const CANCEL: &str = "Cancel";

// Context menu and Actions (UI 5.4, 5.5).
pub const MENU_OPEN: &str = "Open";
pub const MENU_OPEN_WITH: &str = "Open with…";
pub const MENU_COPY: &str = "Copy to target directory";
pub const MENU_RENAME: &str = "Rename…";
pub const MENU_DELETE: &str = "Delete…";
pub const MENU_REFRESH: &str = "Refresh";
pub const MENU_NEW_FOLDER: &str = "New Folder…";
pub const MENU_PERMISSIONS: &str = "Edit Permissions…";
pub const MENU_SHOW_HIDDEN: &str = "Show Hidden Files";
pub const MENU_SELECT_ALL: &str = "Select All";
pub const MENU_CLOSE: &str = "Close";

// Dialogs.
pub const NEW_FOLDER_TITLE: &str = "New folder";
pub const NEW_FOLDER_PLACEHOLDER: &str = "Folder name";
pub const RENAME_TITLE: &str = "Rename";
pub const CREATE: &str = "Create";
pub const RENAME: &str = "Rename";
pub const DELETE: &str = "Delete";
pub const NAME_INVALID: &str = "Use a name without / and other than . and ..";
pub const PERMISSIONS_TITLE: &str = "Edit permissions";
pub const PERMISSIONS_OWNER: &str = "Owner";
pub const PERMISSIONS_GROUP: &str = "Group";
pub const PERMISSIONS_OTHERS: &str = "Others";
pub const PERMISSIONS_READ: &str = "Read";
pub const PERMISSIONS_WRITE: &str = "Write";
pub const PERMISSIONS_EXECUTE: &str = "Execute";
pub const PERMISSIONS_OCTAL: &str = "Octal";
pub const PERMISSIONS_RECURSIVE: &str = "Apply to everything inside";
pub const APPLY: &str = "Apply";

/// `Delete "report.txt"?` / `Delete 3 items?`.
pub fn delete_title(names: &[String]) -> String {
    match names {
        [one] => format!("Delete \u{201c}{one}\u{201d}?"),
        _ => format!("Delete {} items?", names.len()),
    }
}

/// What a delete removes for good.
pub fn delete_text(folders: usize) -> String {
    if folders == 0 {
        "This cannot be undone.".to_owned()
    } else {
        "Folders are deleted with everything inside. This cannot be undone.".to_owned()
    }
}

/// A failed file operation, with the reason.
pub fn failed(what: &str, reason: &str) -> String {
    format!("{what} failed: {reason}")
}

pub const LIST_FAILED: &str = "This folder could not be read";
pub const CREATE_FAILED: &str = "The folder could not be created";
pub const RENAME_FAILED: &str = "The item could not be renamed";
pub const DELETE_FAILED: &str = "Deleting failed";
pub const PERMISSIONS_FAILED: &str = "The permissions could not be changed";

// Open (UI 5.4).
pub const OPEN_FAILED: &str = "The file could not be opened";
pub const UPLOAD_BACK_FAILED: &str = "Uploading the changes failed";
pub const OPEN_NOT_ENCRYPTED: &str =
    "A copy is open in another program. It is not encrypted; saved changes are uploaded back.";
pub const CHANGED_ON_SERVER_TITLE: &str = "The file changed on the server";
pub const KEEP_SERVER_FILE: &str = "Keep the server's file";
pub const OVERWRITE: &str = "Overwrite";

/// After "Keep the server's file".
pub fn kept_server_file(name: &str) -> String {
    format!(
        "Saves of your copy of {name} are no longer uploaded. Open the file again to edit the server's version."
    )
}

/// "Uploaded report.txt".
pub fn uploaded_back(name: &str) -> String {
    format!("Uploaded {name}")
}

/// The question when the server's file changed since the copy was made.
pub fn changed_on_server(name: &str) -> String {
    format!(
        "{name} changed on the server after it was opened. Overwrite it with your copy, or keep the server's file?"
    )
}

// Transfer queue (UI 5.6).
pub const QUEUE_TITLE: &str = "Transfers";
pub const QUEUE_CLEAR: &str = "Clear finished";
pub const PAUSE: &str = "Pause";
pub const RESUME: &str = "Resume";
pub const UPLOAD: &str = "Upload";
pub const DOWNLOAD: &str = "Download";
pub const BETWEEN_HOSTS: &str = "Between hosts";
pub const BETWEEN_HOSTS_FOLDERS: &str = "Folders cannot be copied between two hosts yet";
pub const NOT_CONNECTED: &str = "The host is not connected";
pub const SAME_FILE: &str = "The source and the target are the same file";
pub const INTO_ITSELF: &str = "A folder cannot be copied into itself";
pub const NAME_TAKEN: &str = "An item with this name exists";

/// Why a folder transfer left an entry out (D56).
pub fn skip_reason(reason: shellpp_ssh::transfer::SkipReason) -> &'static str {
    use shellpp_ssh::transfer::SkipReason;
    match reason {
        SkipReason::LinkToFolder => "a link to a folder",
        SkipReason::BrokenLink => "a broken link",
        SkipReason::Special => "a special file",
        SkipReason::NameNotUtf8 => "a name that is not UTF-8",
    }
}

// Open of a file the system would run (security.md 11.4).
pub const RUN_TITLE: &str = "Open a program file?";
pub const RUN_OPEN: &str = "Open";

/// "report.exe is a program file (.exe): opening it runs it."
pub fn run_text(name: &str, extension: &str) -> String {
    format!("{name} is a program file (.{extension}): opening it runs it on this computer.")
}
pub const COPY_LOCAL: &str = "Local copy";
pub const QUEUED: &str = "Queued";
pub const DONE: &str = "Done";
pub const CANCELLED: &str = "Cancelled";
pub const PAUSED: &str = "Paused";

/// "2 active".
pub fn active_count(count: usize) -> String {
    format!("{count} active")
}

/// "Skipped 2: a/link (a link to a folder), …" at the end of a folder
/// transfer (D56).
pub fn skipped(count: usize) -> String {
    format!("{count} left out")
}

// Name conflicts (UI 5.7).
pub const CONFLICT_TITLE: &str = "A file with this name exists";
pub const CONFLICT_REPLACE: &str = "Replace";
pub const CONFLICT_SKIP: &str = "Skip";
pub const CONFLICT_RENAME: &str = "Keep both";
pub const CONFLICT_ALL: &str = "Apply to all";
pub const CONFLICT_HERE: &str = "In the target folder";
pub const CONFLICT_COPIED: &str = "Copied";
