//! Local terminal commands (UI 12.4; stage 2 plan, section 5.1):
//! `local_shells`, `local_open`. Desktop targets only; mobile builds answer
//! `unavailable` (the local terminal is desktop-only, decision 5).

use shellpp_core::service::dto::{LocalOpenInput, LocalShellsResult, SessionInfo};
use tauri::{State, WebviewWindow};

use crate::error::CmdError;
use crate::state::AppState;

#[tauri::command(rename_all = "snake_case")]
pub async fn local_shells(state: State<'_, AppState>) -> Result<LocalShellsResult, CmdError> {
    let sessions = state.sessions()?.clone();
    #[cfg(desktop)]
    {
        sessions.local_shells().await
    }
    #[cfg(not(desktop))]
    {
        sessions.require_unlocked()?;
        Err(CmdError::unavailable())
    }
}

#[tauri::command(rename_all = "snake_case")]
pub async fn local_open(
    state: State<'_, AppState>,
    window: WebviewWindow,
    input: LocalOpenInput,
) -> Result<SessionInfo, CmdError> {
    let sessions = state.sessions()?.clone();
    #[cfg(desktop)]
    {
        sessions.open_local(window.label(), input).await
    }
    #[cfg(not(desktop))]
    {
        let _ = (window, input);
        sessions.require_unlocked()?;
        Err(CmdError::unavailable())
    }
}
