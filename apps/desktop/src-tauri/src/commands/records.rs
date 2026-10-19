//! Record commands (UI 2-3; plan 5.4-5.6). Validation and normalization of
//! record data happen in the core; the host only deserializes the typed
//! inputs (unknown shapes are refused by serde).

use shellpp_core::service::dto::{
    RecordDeleteResult, RecordMoveInput, RecordRef, RecordSaveInput, RecordView, RecordsQuery,
    RecordsRef, SecretCopyTarget, SecretFieldRef, SecretValue,
};
use tauri::State;

use super::with_core;
use crate::error::CmdError;
use crate::state::AppState;

#[tauri::command(rename_all = "snake_case")]
pub async fn records_query(
    state: State<'_, AppState>,
    input: RecordsQuery,
) -> Result<Vec<RecordView>, CmdError> {
    with_core(&state, "records_query", move |core| {
        core.records_query(input)
    })
    .await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn record_get(
    state: State<'_, AppState>,
    input: RecordRef,
) -> Result<RecordView, CmdError> {
    with_core(&state, "record_get", move |core| core.record_get(input)).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn record_save(
    state: State<'_, AppState>,
    input: RecordSaveInput,
) -> Result<RecordView, CmdError> {
    with_core(&state, "record_save", move |core| core.record_save(input)).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn record_delete(
    state: State<'_, AppState>,
    input: RecordsRef,
) -> Result<RecordDeleteResult, CmdError> {
    with_core(&state, "record_delete", move |core| {
        core.record_delete(input)
    })
    .await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn record_restore(
    state: State<'_, AppState>,
    input: RecordsRef,
) -> Result<Vec<RecordView>, CmdError> {
    with_core(&state, "record_restore", move |core| {
        core.record_restore(input)
    })
    .await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn record_move(
    state: State<'_, AppState>,
    input: RecordMoveInput,
) -> Result<Vec<RecordView>, CmdError> {
    with_core(&state, "record_move", move |core| core.record_move(input)).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn record_duplicate(
    state: State<'_, AppState>,
    input: RecordRef,
) -> Result<RecordView, CmdError> {
    with_core(&state, "record_duplicate", move |core| {
        core.record_duplicate(input)
    })
    .await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn secret_reveal(
    state: State<'_, AppState>,
    input: SecretFieldRef,
) -> Result<SecretValue, CmdError> {
    with_core(&state, "secret_reveal", move |core| {
        core.secret_reveal(input).map(|value| SecretValue { value })
    })
    .await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn secret_copy(
    state: State<'_, AppState>,
    input: SecretCopyTarget,
) -> Result<(), CmdError> {
    with_core(&state, "secret_copy", move |core| core.secret_copy(input)).await
}
