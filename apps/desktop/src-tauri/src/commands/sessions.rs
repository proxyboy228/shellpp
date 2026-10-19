//! Session commands (stage 2 plan, section 5.1; docs/architecture.md,
//! sections 3.4 and 4.2): `session_open`, `session_attach`, `session_write`,
//! `session_inject`, `session_resize`, `session_prompt_reply`,
//! `session_cancel`, `session_close`, `session_reconnect`, `session_ack`.
//!
//! The work is in [`crate::sessions::SessionManager`]; every command passes
//! the lock guard first and acts only on sessions of the calling window
//! (`not_found` otherwise). `session_attach`, `session_resize` and
//! `session_ack` are accepted while locked: terminals keep running and
//! showing output under the lock screen (D9).
//!
//! `session_write`, `session_inject` and `session_resize` are synchronous
//! commands: Tauri runs them on the main thread in the order the webview
//! sent them, so typed input keeps its order (async commands run on
//! separate tasks without an ordering guarantee). They only validate, encode
//! and queue the input; the core is never called on the main thread (a
//! history candidate goes to a blocking thread).

use serde::Deserialize;
use shellpp_core::service::dto::{
    SessionInfo, SessionInjectInput, SessionOpenInput, SessionPromptReplyInput, SessionRef,
    SessionResizeInput, SessionWriteInput,
};
use tauri::ipc::{Channel, InvokeResponseBody, JavaScriptChannelId};
use tauri::{State, WebviewWindow};
use uuid::Uuid;

use crate::error::CmdError;
use crate::sessions::OutputSink;
use crate::state::AppState;

/// Input of `session_attach`: the output channel travels inside `input`
/// (plan decision 3), as `__CHANNEL__:<id>`.
#[derive(Deserialize)]
pub struct SessionAttachInput {
    pub session_id: Uuid,
    pub on_output: JavaScriptChannelId,
}

impl std::fmt::Debug for SessionAttachInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionAttachInput")
            .field("session_id", &self.session_id)
            .finish_non_exhaustive()
    }
}

/// Input of `session_ack`: the pane's terminal processed `bytes` more bytes
/// of the output it received (flow control, docs/architecture.md, 3.5).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionAckInput {
    pub session_id: Uuid,
    pub bytes: u64,
}

/// Output frames to the pane's Tauri channel, as raw bytes (`ArrayBuffer`).
struct ChannelOutput(Channel<InvokeResponseBody>);

impl OutputSink for ChannelOutput {
    fn send(&self, frame: Vec<u8>) -> bool {
        self.0.send(InvokeResponseBody::Raw(frame)).is_ok()
    }
}

#[tauri::command(rename_all = "snake_case")]
pub async fn session_open(
    state: State<'_, AppState>,
    window: WebviewWindow,
    input: SessionOpenInput,
) -> Result<SessionInfo, CmdError> {
    let sessions = state.sessions()?.clone();
    sessions.open_ssh(window.label(), input).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn session_attach(
    state: State<'_, AppState>,
    window: WebviewWindow,
    input: SessionAttachInput,
) -> Result<SessionInfo, CmdError> {
    let sessions = state.sessions()?.clone();
    let channel: Channel<InvokeResponseBody> = input.on_output.channel_on(window.as_ref().clone());
    sessions.attach(
        window.label(),
        input.session_id,
        Box::new(ChannelOutput(channel)),
    )
}

#[tauri::command(rename_all = "snake_case")]
pub fn session_write(
    state: State<'_, AppState>,
    window: WebviewWindow,
    input: SessionWriteInput,
) -> Result<(), CmdError> {
    state.sessions()?.write(window.label(), input)
}

#[tauri::command(rename_all = "snake_case")]
pub fn session_inject(
    state: State<'_, AppState>,
    window: WebviewWindow,
    input: SessionInjectInput,
) -> Result<(), CmdError> {
    state.sessions()?.inject(window.label(), input)
}

#[tauri::command(rename_all = "snake_case")]
pub fn session_resize(
    state: State<'_, AppState>,
    window: WebviewWindow,
    input: SessionResizeInput,
) -> Result<(), CmdError> {
    state.sessions()?.resize(window.label(), input)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn session_prompt_reply(
    state: State<'_, AppState>,
    window: WebviewWindow,
    input: SessionPromptReplyInput,
) -> Result<(), CmdError> {
    state.sessions()?.prompt_reply(window.label(), input)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn session_cancel(
    state: State<'_, AppState>,
    window: WebviewWindow,
    input: SessionRef,
) -> Result<(), CmdError> {
    state.sessions()?.cancel(window.label(), input.session_id)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn session_close(
    state: State<'_, AppState>,
    window: WebviewWindow,
    input: SessionRef,
) -> Result<(), CmdError> {
    state.sessions()?.close(window.label(), input.session_id)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn session_reconnect(
    state: State<'_, AppState>,
    window: WebviewWindow,
    input: SessionRef,
) -> Result<SessionInfo, CmdError> {
    state
        .sessions()?
        .reconnect(window.label(), input.session_id)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn session_ack(
    state: State<'_, AppState>,
    window: WebviewWindow,
    input: SessionAckInput,
) -> Result<(), CmdError> {
    state
        .sessions()?
        .ack(window.label(), input.session_id, input.bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ack_input_shape() {
        let input: SessionAckInput = serde_json::from_value(serde_json::json!({
            "session_id": "0199f6a1-b2c3-7d4e-8f00-000000000009",
            "bytes": 131072
        }))
        .unwrap();
        assert_eq!(input.bytes, 131_072);
        assert!(
            serde_json::from_value::<SessionAckInput>(serde_json::json!({
                "session_id": "0199f6a1-b2c3-7d4e-8f00-000000000009",
                "bytes": -1
            }))
            .is_err()
        );
    }

    #[test]
    fn attach_input_takes_a_channel_id() {
        let input: SessionAttachInput = serde_json::from_value(serde_json::json!({
            "session_id": "0199f6a1-b2c3-7d4e-8f00-000000000009",
            "on_output": "__CHANNEL__:42"
        }))
        .unwrap();
        assert_eq!(
            input.session_id.to_string(),
            "0199f6a1-b2c3-7d4e-8f00-000000000009"
        );
        assert!(format!("{input:?}").starts_with("SessionAttachInput"));
        assert!(
            serde_json::from_value::<SessionAttachInput>(serde_json::json!({
                "session_id": "0199f6a1-b2c3-7d4e-8f00-000000000009",
                "on_output": "42"
            }))
            .is_err()
        );
    }
}
