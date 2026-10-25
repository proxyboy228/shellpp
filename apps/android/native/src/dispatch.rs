//! The command dispatcher of architecture 2: `call(command, input)` runs one
//! command of the core's service API with a JSON input and answers
//! `{"ok": ...}` or `{"error": {"code", "message", "details"}}` with the
//! core's error codes. Plain Rust, tested on the computer.
//!
//! Inputs may carry secrets (the master password, record secrets): the JNI
//! layer hands them over in a buffer it wipes, they are parsed into
//! `SecretString`s, and only the command's name and an error's code are
//! ever logged.

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use shellpp_core::lock::FirstRunOptions;
use shellpp_core::service::dto::{
    FirstRunInput, PasswordInput, RecordMoveInput, RecordRef, RecordSaveInput, RecordsQuery,
    RecordsRef, RememberDeviceInput, SettingsSetInput,
};
use shellpp_core::{Core, CoreError};
use uuid::Uuid;

/// Why a command failed.
enum Failure {
    Core(CoreError),
    /// The input was not the command's JSON.
    Input,
    Unknown,
}

impl From<CoreError> for Failure {
    fn from(error: CoreError) -> Self {
        Self::Core(error)
    }
}

/// The input of `recently_deleted_restore`.
#[derive(serde::Deserialize)]
struct RecordIds {
    record_ids: Vec<Uuid>,
}

fn parse<I: DeserializeOwned>(input: &[u8]) -> Result<I, Failure> {
    serde_json::from_slice(input).map_err(|_| Failure::Input)
}

fn out<O: Serialize>(value: Result<O, CoreError>) -> Result<Value, Failure> {
    let value = value?;
    serde_json::to_value(value).map_err(|_| Failure::Core(CoreError::Internal("answer")))
}

/// Runs `command` on `core` with the JSON `input` (empty for commands that
/// take none) and returns the answer as JSON text.
pub fn call(core: &Core, command: &str, input: &[u8]) -> String {
    let answer = match run(core, command, input) {
        Ok(value) => json!({ "ok": value }),
        Err(failure) => {
            let error = match failure {
                Failure::Core(error) => json!({
                    "code": error.code(),
                    "message": error.to_string(),
                    "details": error.details(),
                }),
                Failure::Input => json!({ "code": "bad_input", "message": "bad input" }),
                Failure::Unknown => {
                    json!({ "code": "unknown_command", "message": "unknown command" })
                }
            };
            tracing::debug!(command, code = %error["code"], "command failed");
            json!({ "error": error })
        }
    };
    answer.to_string()
}

fn run(core: &Core, command: &str, input: &[u8]) -> Result<Value, Failure> {
    match command {
        // Lock (UI spec 4, 5).
        "lock_status" => out(Ok(core.lock_status())),
        "snapshot" => out(Ok(core.snapshot())),
        "first_run_setup" => {
            let input: FirstRunInput = parse(input)?;
            // The phone never offers "Remember on this device" at the first
            // run: fingerprint unlock is turned on in Settings (UI spec 4).
            out(core.first_run_setup_with(
                input.password,
                FirstRunOptions {
                    remember_device: false,
                },
            ))
        }
        "unlock" => {
            let input: PasswordInput = parse(input)?;
            out(core.unlock(input.password))
        }
        "try_device_unlock" => out(core.try_device_unlock()),
        "lock" => out(core.lock()),
        "start_new_vault" => out(core.start_new_vault()),
        "remember_device_set" => {
            let input: RememberDeviceInput = parse(input)?;
            out(core.remember_device_set(input.enabled))
        }
        "activity" => {
            core.activity();
            out(Ok(Value::Null))
        }
        "tick" => {
            core.tick();
            out(Ok(Value::Null))
        }
        "auto_lock_due" => out(Ok(core.auto_lock_due())),
        // Settings.
        "settings_get" => out(core.settings_get()),
        "settings_set" => {
            let input: SettingsSetInput = parse(input)?;
            out(core.settings_set(input))
        }
        // Hosts and groups (UI spec 6).
        "vaults" => out(core.vaults()),
        "records_query" => {
            let input: RecordsQuery = parse(input)?;
            out(core.records_query(input))
        }
        "record_get" => {
            let input: RecordRef = parse(input)?;
            out(core.record_get(input))
        }
        "record_save" => {
            let input: RecordSaveInput = parse(input)?;
            out(core.record_save(input))
        }
        "record_delete" => {
            let input: RecordsRef = parse(input)?;
            out(core.record_delete(input))
        }
        "record_restore" => {
            let input: RecordsRef = parse(input)?;
            out(core.record_restore(input))
        }
        "record_move" => {
            let input: RecordMoveInput = parse(input)?;
            out(core.record_move(input))
        }
        "record_duplicate" => {
            let input: RecordRef = parse(input)?;
            out(core.record_duplicate(input))
        }
        "recently_deleted_list" => out(core.recently_deleted_list()),
        "recently_deleted_restore" => {
            let input: RecordIds = parse(input)?;
            out(core.recently_deleted_restore(&input.record_ids))
        }
        // The Keychain's list (the host editor's key picker).
        "keys_list" => out(core.keys_list()),
        _ => Err(Failure::Unknown),
    }
}
