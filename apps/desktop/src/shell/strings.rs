//! Strings of the window shell added in stage 2b: the session tab menu
//! (UI 4.4), Rename, the close-window confirmation (UI 1.4, 4.6) and
//! Restore (UI 6). The frame's older strings are in `crate::i18n`.

/// The session tab menu (UI 4.4), in its order.
pub const MENU_DUPLICATE: &str = "Duplicate";
pub const MENU_DUPLICATE_NEW_WINDOW: &str = "Duplicate in a New Window";
pub const MENU_RENAME: &str = "Rename";
pub const MENU_SPLIT_HORIZONTALLY: &str = "Split Horizontally";
pub const MENU_SPLIT_VERTICALLY: &str = "Split Vertically";
pub const MENU_BROADCAST_INPUT: &str = "Broadcast Input";
pub const MENU_BROADCAST_STOP: &str = "Stop Broadcast Input";
pub const MENU_CLOSE: &str = "Close";

/// Accessible name of the inline Rename field.
pub const RENAME_FIELD: &str = "Tab name";

/// Closing a window while its sessions are active (UI 1.4, 4.6).
pub const CLOSE_WINDOW_TITLE: &str = "Close this window?";
pub const CLOSE_WINDOW_CANCEL: &str = "Cancel";
pub const CLOSE_WINDOW_OK: &str = "Close Window";

/// "1 session in this window is open. It will be closed."
pub fn close_window_body(sessions: usize) -> String {
    if sessions == 1 {
        "1 session in this window is open. It will be closed.".to_owned()
    } else {
        format!("{sessions} sessions in this window are open. They will be closed.")
    }
}

/// The toast when Restore found nothing it could open (the hosts are gone).
pub const RESTORE_NOTHING: &str = "Nothing from the previous launch could be restored.";

/// A split run whose hosts were removed down to fewer than two (D81).
pub const SPLIT_RUN_TOO_FEW: &str = "A split tab needs two to four saved hosts";
