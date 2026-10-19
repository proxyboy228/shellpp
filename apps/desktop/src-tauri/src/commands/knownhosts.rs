//! Known Hosts commands (UI 10.1; stage 2 plan, section 5.1):
//! `known_hosts_list`, `known_host_remove`.

use shellpp_core::service::dto::{KnownHostView, RecordsRef};
use tauri::State;

use super::with_core;
use crate::error::CmdError;
use crate::state::AppState;

#[tauri::command(rename_all = "snake_case")]
pub async fn known_hosts_list(state: State<'_, AppState>) -> Result<Vec<KnownHostView>, CmdError> {
    with_core(&state, "known_hosts_list", |core| core.known_hosts_list()).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn known_host_remove(
    state: State<'_, AppState>,
    input: RecordsRef,
) -> Result<(), CmdError> {
    with_core(&state, "known_host_remove", move |core| {
        core.known_host_remove(input)
    })
    .await
}
