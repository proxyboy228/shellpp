//! The texts of the Logs screen (UI 11).

pub const DATE: &str = "Date";
pub const USER: &str = "User";
pub const HOST: &str = "Host";
pub const SAVED: &str = "Saved";
pub const SORT_NEWEST: &str = "Newest first: click for oldest first";
pub const SORT_OLDEST: &str = "Oldest first: click for newest first";
pub const SAVED_ONLY: &str = "Show only saved entries";
pub const SAVED_ALL: &str = "Show every entry";
pub const SAVE: &str = "Save: auto-cleanup keeps it";
pub const UNSAVE: &str = "Saved: click to let auto-cleanup remove it";
pub const NOW: &str = "now";
pub const UNKNOWN_END: &str = "unknown";
pub const THIS_DEVICE: &str = "This device";
pub const OTHER_DEVICE: &str = "Another device";
pub const EMPTY_TITLE: &str = "No connections yet";
pub const EMPTY_BODY: &str = "Each SSH session to a host is listed here with its start and end.";
pub const NO_SAVED: &str = "No saved entries";
pub const LOAD_FAILED: &str = "The log could not be read.";
pub const SAVE_FAILED: &str = "The entry could not be saved";

// Details panel (11.2).
pub const PANEL_TITLE: &str = "Connection";
pub const FIELD_HOST: &str = "Host";
pub const FIELD_ADDRESS: &str = "Address";
pub const FIELD_USER: &str = "Remote user";
pub const FIELD_PROTOCOL: &str = "Protocol";
pub const FIELD_STARTED: &str = "Started";
pub const FIELD_ENDED: &str = "Ended";
pub const FIELD_END: &str = "How it ended";
pub const FIELD_DEVICE: &str = "Device";
pub const NOT_RECORDED: &str = "The output of this session was not recorded.";
pub const ACTIVE: &str = "Still open";
pub const END_CLOSED: &str = "Closed";
pub const END_DROPPED: &str = "The connection dropped";
pub const END_UNKNOWN: &str = "Unknown: the app stopped during the session";

/// "Exit code 2".
pub fn end_exit(code: Option<u32>) -> String {
    match code {
        Some(code) => format!("Exit code {code}"),
        None => "Ended with an error".to_owned(),
    }
}
