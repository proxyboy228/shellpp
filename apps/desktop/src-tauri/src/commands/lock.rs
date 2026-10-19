//! Lock commands (UI 13.1-13.4; docs/security.md, section 8). Passwords and
//! recovery keys arrive as `SecretString` (redacted `Debug`, zeroized on drop)
//! and are never logged.

use shellpp_core::service::dto::{
    FirstRunInput, LockStatus, PasswordChangeInput, PasswordInput, RecoveryKeyResult,
    RecoveryUnlockInput, RememberDeviceInput,
};
use tauri::State;

use super::with_core;
use crate::error::CmdError;
use crate::state::AppState;

#[tauri::command(rename_all = "snake_case")]
pub async fn lock_status(state: State<'_, AppState>) -> Result<LockStatus, CmdError> {
    with_core(&state, "lock_status", |core| Ok(core.lock_status())).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn first_run_setup(
    state: State<'_, AppState>,
    input: FirstRunInput,
) -> Result<RecoveryKeyResult, CmdError> {
    let result = with_core(&state, "first_run_setup", move |core| {
        core.first_run_setup(input.password)
    })
    .await?;
    tracing::info!("first run completed");
    Ok(result)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn unlock(
    state: State<'_, AppState>,
    input: PasswordInput,
) -> Result<LockStatus, CmdError> {
    with_core(&state, "unlock", move |core| core.unlock(input.password)).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn lock(state: State<'_, AppState>) -> Result<LockStatus, CmdError> {
    with_core(&state, "lock", |core| core.lock()).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn recovery_unlock(
    state: State<'_, AppState>,
    input: RecoveryUnlockInput,
) -> Result<LockStatus, CmdError> {
    with_core(&state, "recovery_unlock", move |core| {
        core.recovery_unlock(input.recovery_key, input.new_password)
    })
    .await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn password_change(
    state: State<'_, AppState>,
    input: PasswordChangeInput,
) -> Result<(), CmdError> {
    with_core(&state, "password_change", move |core| {
        core.password_change(input.current_password, input.new_password)
    })
    .await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn recovery_key_show(
    state: State<'_, AppState>,
    input: PasswordInput,
) -> Result<RecoveryKeyResult, CmdError> {
    with_core(&state, "recovery_key_show", move |core| {
        core.recovery_key_show(input.password)
    })
    .await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn recovery_key_reissue(
    state: State<'_, AppState>,
    input: PasswordInput,
) -> Result<RecoveryKeyResult, CmdError> {
    with_core(&state, "recovery_key_reissue", move |core| {
        core.recovery_key_reissue(input.password)
    })
    .await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn remember_device_set(
    state: State<'_, AppState>,
    input: RememberDeviceInput,
) -> Result<LockStatus, CmdError> {
    with_core(&state, "remember_device_set", move |core| {
        core.remember_device_set(input.enabled)
    })
    .await
}
