//! Notification center commands (UI 1.5; docs/data-model.md, section 16.4).

use shellpp_core::service::dto::NotificationList;
use tauri::State;

use super::with_core;
use crate::error::CmdError;
use crate::state::AppState;

#[tauri::command(rename_all = "snake_case")]
pub async fn notifications_list(state: State<'_, AppState>) -> Result<NotificationList, CmdError> {
    with_core(&state, "notifications_list", |core| {
        Ok(core.notifications_list())
    })
    .await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn notifications_mark_read(
    state: State<'_, AppState>,
) -> Result<NotificationList, CmdError> {
    with_core(&state, "notifications_mark_read", |core| {
        Ok(core.notifications_mark_read())
    })
    .await
}
