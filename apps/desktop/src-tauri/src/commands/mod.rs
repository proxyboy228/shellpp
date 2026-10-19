//! Commands (docs/architecture.md, section 3.4). Every command:
//!
//! - is declared in one of the area modules below, registered once in
//!   `generate_handler!` (src/lib.rs) and listed in `build.rs`, which gives it
//!   its own `allow-<command>` permission;
//! - takes at most one argument, named `input` (`invoke(name, { input })`);
//! - is `async` and runs every `Core` call on a `spawn_blocking` thread;
//! - passes the lock guard below before doing anything (architecture 3.3):
//!   the UI is not trusted to enforce the lock;
//! - rejects with [`CmdError`] (`{"code", "details"}`).
//!
//! The guard is defense in depth: the core enforces the same rules for its
//! own methods; the host is the only enforcement for `external_open_url` and
//! the clipboard commands.

pub mod app;
pub mod clipboard;
#[cfg(feature = "e2e-mocks")]
pub mod e2e;
pub mod lock;
pub mod notifications;
pub mod records;
pub mod settings;

use std::sync::Arc;

use shellpp_core::Core;
use shellpp_core::error::CoreError;
use shellpp_core::service::dto::LockState;

use crate::error::CmdError;
use crate::state::AppState;

/// In which lock states a command is accepted (architecture 3.3; plan 5.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
    /// `first_run`, `locked` and `unlocked`.
    Any,
    /// Only at `first_run` (`first_run_setup`); else `wrong_state`.
    FirstRun,
    /// Only while `locked` (`unlock`, `recovery_unlock`); else `wrong_state`.
    Locked,
    /// `locked` or `unlocked` (`lock`); `wrong_state` at first run.
    AfterFirstRun,
    /// Only while `unlocked`; else `locked`.
    Unlocked,
}

/// The access rule of every command in `build.rs` (tests check that the two
/// lists agree).
pub const COMMAND_ACCESS: &[(&str, Access)] = &[
    // app
    ("app_info", Access::Any),
    ("state_snapshot", Access::Any),
    ("window_open", Access::Any),
    ("activity", Access::Any),
    ("app_exit", Access::Any),
    ("external_open_url", Access::Unlocked),
    // lock
    ("lock_status", Access::Any),
    ("first_run_setup", Access::FirstRun),
    ("unlock", Access::Locked),
    ("lock", Access::AfterFirstRun),
    ("recovery_unlock", Access::Locked),
    ("password_change", Access::Unlocked),
    ("recovery_key_show", Access::Unlocked),
    ("recovery_key_reissue", Access::Unlocked),
    ("remember_device_set", Access::Unlocked),
    // records
    ("records_query", Access::Unlocked),
    ("record_get", Access::Unlocked),
    ("record_save", Access::Unlocked),
    ("record_delete", Access::Unlocked),
    ("record_restore", Access::Unlocked),
    ("record_move", Access::Unlocked),
    ("record_duplicate", Access::Unlocked),
    ("secret_reveal", Access::Unlocked),
    ("secret_copy", Access::Unlocked),
    // settings
    ("settings_get", Access::Any),
    ("settings_set", Access::Unlocked),
    // notifications
    ("notifications_list", Access::Unlocked),
    ("notifications_mark_read", Access::Unlocked),
    // clipboard
    ("clipboard_write_text", Access::Unlocked),
    ("clipboard_read_text", Access::Unlocked),
    // e2e only
    ("e2e_mock", Access::Any),
];

/// The access rule of a command (`None` for an unknown name).
pub fn access_of(command: &str) -> Option<Access> {
    COMMAND_ACCESS
        .iter()
        .find(|(name, _)| *name == command)
        .map(|(_, access)| *access)
}

/// Decides whether a command with `access` may run in `state`.
pub fn check(access: Access, state: LockState) -> Result<(), CmdError> {
    match (access, state) {
        (Access::Any, _)
        | (Access::FirstRun, LockState::FirstRun)
        | (Access::Locked, LockState::Locked)
        | (Access::AfterFirstRun, LockState::Locked | LockState::Unlocked)
        | (Access::Unlocked, LockState::Unlocked) => Ok(()),
        (Access::Unlocked, _) => Err(CmdError::locked()),
        (Access::FirstRun | Access::Locked | Access::AfterFirstRun, _) => {
            Err(CmdError::wrong_state())
        }
    }
}

/// Runs the guard against the core's current state. `Unlocked` uses the
/// cheap `is_unlocked`; the rare state-specific commands read the full status.
fn guard(core: &Core, access: Access) -> Result<(), CmdError> {
    match access {
        Access::Any => Ok(()),
        Access::Unlocked => {
            if core.is_unlocked() {
                Ok(())
            } else {
                Err(CmdError::locked())
            }
        }
        Access::FirstRun | Access::Locked | Access::AfterFirstRun => {
            check(access, core.lock_status().state)
        }
    }
}

/// Runs `f` on a blocking thread after the lock guard. A panic inside the
/// core becomes `internal` instead of tearing down the command.
pub async fn with_core<T, F>(state: &AppState, command: &'static str, f: F) -> Result<T, CmdError>
where
    T: Send + 'static,
    F: FnOnce(&Core) -> Result<T, CoreError> + Send + 'static,
{
    let core = state.core()?;
    let access = access_of(command).unwrap_or(Access::Unlocked);
    run_blocking(command, move || {
        guard(&core, access)?;
        f(&core).map_err(CmdError::from)
    })
    .await
}

/// Like [`with_core`] for host-side work that needs the guard but returns
/// host errors.
pub async fn with_guard<T, F>(state: &AppState, command: &'static str, f: F) -> Result<T, CmdError>
where
    T: Send + 'static,
    F: FnOnce(&Arc<Core>) -> Result<T, CmdError> + Send + 'static,
{
    let core = state.core()?;
    let access = access_of(command).unwrap_or(Access::Unlocked);
    run_blocking(command, move || {
        guard(&core, access)?;
        f(&core)
    })
    .await
}

/// `spawn_blocking` with panic and cancellation mapped to `internal`; logs
/// the command name and error code only.
pub async fn run_blocking<T, F>(command: &'static str, f: F) -> Result<T, CmdError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, CmdError> + Send + 'static,
{
    let result = match tauri::async_runtime::spawn_blocking(f).await {
        Ok(result) => result,
        Err(_) => {
            tracing::error!(command, "command panicked");
            Err(CmdError::internal())
        }
    };
    if let Err(error) = &result {
        tracing::debug!(command, code = error.code, "command rejected");
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    const STATES: [LockState; 3] = [LockState::FirstRun, LockState::Locked, LockState::Unlocked];

    fn code(result: Result<(), CmdError>) -> &'static str {
        match result {
            Ok(()) => "ok",
            Err(error) => error.code,
        }
    }

    #[test]
    fn guard_table_matches_the_lock_states() {
        let expected = [
            (Access::Any, ["ok", "ok", "ok"]),
            (Access::FirstRun, ["ok", "wrong_state", "wrong_state"]),
            (Access::Locked, ["wrong_state", "ok", "wrong_state"]),
            (Access::AfterFirstRun, ["wrong_state", "ok", "ok"]),
            (Access::Unlocked, ["locked", "locked", "ok"]),
        ];
        for (access, codes) in expected {
            for (state, want) in STATES.iter().zip(codes) {
                assert_eq!(code(check(access, *state)), want, "{access:?} at {state:?}");
            }
        }
    }

    /// Commands accepted before unlock (architecture 3.3 and plan 5.1).
    #[test]
    fn commands_accepted_while_locked() {
        let mut accepted: Vec<&str> = COMMAND_ACCESS
            .iter()
            .filter(|(_, access)| check(*access, LockState::Locked).is_ok())
            .map(|(name, _)| *name)
            .collect();
        accepted.sort_unstable();
        assert_eq!(
            accepted,
            vec![
                "activity",
                "app_exit",
                "app_info",
                "e2e_mock",
                "lock",
                "lock_status",
                "recovery_unlock",
                "settings_get",
                "state_snapshot",
                "unlock",
                "window_open",
            ]
        );
        let mut first_run: Vec<&str> = COMMAND_ACCESS
            .iter()
            .filter(|(_, access)| check(*access, LockState::FirstRun).is_ok())
            .map(|(name, _)| *name)
            .collect();
        first_run.sort_unstable();
        assert_eq!(
            first_run,
            vec![
                "activity",
                "app_exit",
                "app_info",
                "e2e_mock",
                "first_run_setup",
                "lock_status",
                "settings_get",
                "state_snapshot",
                "window_open",
            ]
        );
    }

    #[test]
    fn host_enforced_commands_need_unlock() {
        for command in [
            "external_open_url",
            "clipboard_write_text",
            "clipboard_read_text",
        ] {
            assert_eq!(access_of(command), Some(Access::Unlocked), "{command}");
        }
        assert_eq!(access_of("no_such_command"), None);
    }

    #[test]
    fn command_names_are_unique() {
        let mut names: Vec<&str> = COMMAND_ACCESS.iter().map(|(name, _)| *name).collect();
        let before = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), before);
    }
}
