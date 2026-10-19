//! Key commands of the short "+ -> Key" path (UI 3.1, D3; stage 2 plan,
//! section 5.1): `keys_list`, `key_import`. The private key text and the
//! passphrase arrive as `SecretString` and never leave Rust again.

use shellpp_core::service::dto::{KeyImportInput, KeyImportResult, KeyView};
use tauri::State;

use super::with_core;
use crate::error::CmdError;
use crate::state::AppState;

#[tauri::command(rename_all = "snake_case")]
pub async fn keys_list(state: State<'_, AppState>) -> Result<Vec<KeyView>, CmdError> {
    with_core(&state, "keys_list", |core| core.keys_list()).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn key_import(
    state: State<'_, AppState>,
    input: KeyImportInput,
) -> Result<KeyImportResult, CmdError> {
    with_core(&state, "key_import", move |core| core.key_import(input)).await
}
