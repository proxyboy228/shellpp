//! History and Recent connections commands (UI 4.2, 6; stage 2 plan,
//! section 5.1): `history_list`, `history_delete`, `history_clear`,
//! `recent_connections`. History entries are written only by the session
//! manager (`session_write` candidates), never by a command.

use shellpp_core::service::dto::{
    HistoryClearInput, HistoryClearResult, HistoryEntryView, HistoryListInput, RecentConnection,
    RecordsRef,
};
use tauri::State;

use super::with_core;
use crate::error::CmdError;
use crate::state::AppState;

#[tauri::command(rename_all = "snake_case")]
pub async fn history_list(
    state: State<'_, AppState>,
    input: HistoryListInput,
) -> Result<Vec<HistoryEntryView>, CmdError> {
    with_core(&state, "history_list", move |core| core.history_list(input)).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn history_delete(state: State<'_, AppState>, input: RecordsRef) -> Result<(), CmdError> {
    with_core(&state, "history_delete", move |core| {
        core.history_delete(input)
    })
    .await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn history_clear(
    state: State<'_, AppState>,
    input: HistoryClearInput,
) -> Result<HistoryClearResult, CmdError> {
    with_core(&state, "history_clear", move |core| {
        core.history_clear(input)
    })
    .await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn recent_connections(
    state: State<'_, AppState>,
) -> Result<Vec<RecentConnection>, CmdError> {
    with_core(&state, "recent_connections", |core| {
        core.recent_connections()
    })
    .await
}
