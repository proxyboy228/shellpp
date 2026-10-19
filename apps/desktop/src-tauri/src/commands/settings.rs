//! Settings commands (docs/data-model.md, section 16.1). Readable in every
//! lock state, writable only while unlocked; the core validates every key.

use shellpp_core::service::dto::{SettingsSetInput, SettingsValues};
use tauri::State;

use super::with_core;
use crate::error::CmdError;
use crate::state::AppState;

#[tauri::command(rename_all = "snake_case")]
pub async fn settings_get(state: State<'_, AppState>) -> Result<SettingsValues, CmdError> {
    with_core(&state, "settings_get", |core| core.settings_get()).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn settings_set(
    state: State<'_, AppState>,
    input: SettingsSetInput,
) -> Result<SettingsValues, CmdError> {
    with_core(&state, "settings_set", move |core| core.settings_set(input)).await
}
