//! Non-sensitive clipboard text for the webview: Copy Link (UI 2.4) and
//! Edit ▸ Paste (UI 1.4). Secrets never pass through here; they are copied by
//! Rust with `secret_copy`. Both commands need the unlocked state (host-only
//! guard, plan 4.7) so a locked window cannot read the clipboard.

use serde::{Deserialize, Serialize};
use shellpp_core::error::ValidationReason;
use shellpp_core::platform::PlatformError;
use tauri::State;

use super::with_guard;
use crate::error::CmdError;
use crate::state::AppState;

/// Longest text `clipboard_write_text` accepts (bytes of UTF-8).
pub const MAX_CLIPBOARD_BYTES: usize = 1024 * 1024;

/// Input of `clipboard_write_text`.
#[derive(Clone, Deserialize)]
pub struct ClipboardWriteInput {
    pub text: String,
}

impl std::fmt::Debug for ClipboardWriteInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClipboardWriteInput")
            .field("len", &self.text.len())
            .finish()
    }
}

/// Result of `clipboard_read_text`: `null` when the clipboard holds no text.
#[derive(Clone, PartialEq, Eq, Serialize)]
pub struct ClipboardText {
    pub text: Option<String>,
}

impl std::fmt::Debug for ClipboardText {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ClipboardText { .. }")
    }
}

/// At most 1 MiB.
pub fn validate_clipboard_text(text: &str) -> Result<(), CmdError> {
    if text.len() > MAX_CLIPBOARD_BYTES {
        return Err(CmdError::validation("text", ValidationReason::TooLong));
    }
    Ok(())
}

fn platform_error(_error: PlatformError) -> CmdError {
    CmdError::unavailable()
}

#[tauri::command(rename_all = "snake_case")]
pub async fn clipboard_write_text(
    state: State<'_, AppState>,
    input: ClipboardWriteInput,
) -> Result<(), CmdError> {
    let clipboard = state.platform()?.clipboard.clone();
    // Lock guard first, then the argument check.
    with_guard(&state, "clipboard_write_text", move |_core| {
        validate_clipboard_text(&input.text)?;
        clipboard.write_text(&input.text).map_err(platform_error)
    })
    .await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn clipboard_read_text(state: State<'_, AppState>) -> Result<ClipboardText, CmdError> {
    let clipboard = state.platform()?.clipboard.clone();
    with_guard(&state, "clipboard_read_text", move |_core| {
        let text = clipboard.read_text().map_err(platform_error)?;
        Ok(ClipboardText { text })
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn length_limit() {
        assert!(validate_clipboard_text("").is_ok());
        assert!(validate_clipboard_text(&"a".repeat(MAX_CLIPBOARD_BYTES)).is_ok());
        assert_eq!(
            validate_clipboard_text(&"a".repeat(MAX_CLIPBOARD_BYTES + 1)),
            Err(CmdError::validation("text", ValidationReason::TooLong))
        );
        // The limit counts bytes, not characters.
        let wide = "\u{00e9}".repeat(MAX_CLIPBOARD_BYTES / 2 + 1);
        assert!(validate_clipboard_text(&wide).is_err());
    }

    #[test]
    fn shapes() {
        let input: ClipboardWriteInput =
            serde_json::from_value(serde_json::json!({"text": "shellpp://host/x"})).unwrap();
        assert_eq!(input.text, "shellpp://host/x");
        assert_eq!(format!("{input:?}"), "ClipboardWriteInput { len: 16 }");
        assert_eq!(
            serde_json::to_value(ClipboardText { text: None }).unwrap(),
            serde_json::json!({"text": null})
        );
        assert_eq!(
            serde_json::to_value(ClipboardText {
                text: Some("a".into())
            })
            .unwrap(),
            serde_json::json!({"text": "a"})
        );
    }
}
