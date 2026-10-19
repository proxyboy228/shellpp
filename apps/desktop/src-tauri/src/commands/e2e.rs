//! `e2e_mock` (plan 5.7; docs/architecture.md, section 5.4): lets a
//! WebdriverIO test drive the platform mocks. Compiled only with the
//! `e2e-mocks` feature, which cannot be enabled in a release build, and
//! granted only by `capabilities/e2e.json`.

use std::collections::BTreeMap;
use std::time::Duration;

use serde::Deserialize;
use serde_json::{Value, json};
use shellpp_core::error::ValidationReason;
use shellpp_core::service::dto::{
    KnownHostAddInput, NewNotification, NotificationLevel, NotificationSource,
    NotificationSourceKind,
};
use tauri::{AppHandle, State};

use super::{run_blocking, with_core};
use crate::error::CmdError;
use crate::events::receive_link;
use crate::links::parse_link;
use crate::state::AppState;

/// Longest clock step a test may take.
pub const MAX_ADVANCE_SECONDS: u64 = 86_400;

/// The actions of `e2e_mock`.
#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum E2eMockAction {
    ClipboardRead,
    ClipboardWrite {
        text: String,
    },
    LauncherRequests,
    SecretStoreSetAvailable {
        available: bool,
    },
    SecretStoreAccounts,
    Notify {
        level: NotificationLevel,
        message_key: String,
        #[serde(default)]
        params: BTreeMap<String, String>,
    },
    ClockAdvance {
        seconds: u64,
    },
    InjectLink {
        url: String,
    },
    /// Seeds a known_hosts entry (hashed and revoked scenarios of UI 10.1,
    /// 10.2; stage 2 plan, decision 11).
    KnownHostAdd(KnownHostAddInput),
}

/// Parses the action. An unknown `kind` or a malformed member is
/// `validation_failed` with field `kind`; a clock step outside 0..=86400 s is
/// `validation_failed` with field `seconds`.
pub fn parse_action(input: Value) -> Result<E2eMockAction, CmdError> {
    let action: E2eMockAction = serde_json::from_value(input)
        .map_err(|_| CmdError::validation("kind", ValidationReason::Invalid))?;
    if let E2eMockAction::ClockAdvance { seconds } = &action
        && *seconds > MAX_ADVANCE_SECONDS
    {
        return Err(CmdError::validation(
            "seconds",
            ValidationReason::OutOfRange,
        ));
    }
    if let E2eMockAction::Notify { message_key, .. } = &action
        && (message_key.is_empty() || message_key.len() > 256)
    {
        return Err(CmdError::validation(
            "message_key",
            ValidationReason::Invalid,
        ));
    }
    Ok(action)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn e2e_mock(
    app: AppHandle,
    state: State<'_, AppState>,
    input: Value,
) -> Result<Value, CmdError> {
    let action = parse_action(input)?;
    let mocks = state
        .extras
        .as_ref()
        .map(|extras| extras.e2e.clone())
        .ok_or_else(|| state.core().err().unwrap_or_else(CmdError::internal))?;
    match action {
        E2eMockAction::ClipboardRead => Ok(json!({ "text": mocks.clipboard.text() })),
        E2eMockAction::ClipboardWrite { text } => {
            mocks.clipboard.set_text(text);
            Ok(json!({}))
        }
        E2eMockAction::LauncherRequests => Ok(json!({ "urls": mocks.launcher.urls() })),
        E2eMockAction::SecretStoreSetAvailable { available } => {
            let store = mocks.secret_store.clone();
            run_blocking("e2e_mock", move || {
                store
                    .set_available(available)
                    .map_err(|_| CmdError::unavailable())
            })
            .await?;
            Ok(json!({}))
        }
        E2eMockAction::SecretStoreAccounts => {
            Ok(json!({ "accounts": mocks.secret_store.accounts() }))
        }
        E2eMockAction::Notify {
            level,
            message_key,
            params,
        } => {
            let notification = NewNotification {
                level,
                message_key,
                params,
                source: NotificationSource {
                    kind: NotificationSourceKind::App,
                    reference: None,
                },
            };
            with_core(&state, "e2e_mock", move |core| {
                core.notify(notification);
                Ok(())
            })
            .await?;
            Ok(json!({}))
        }
        E2eMockAction::ClockAdvance { seconds } => {
            mocks.clock.advance(Duration::from_secs(seconds));
            with_core(&state, "e2e_mock", |core| {
                core.tick();
                Ok(())
            })
            .await?;
            Ok(json!({}))
        }
        E2eMockAction::InjectLink { url } => {
            if let Some(link) = parse_link(&url) {
                receive_link(&app, &state.links, &state.windows, link);
            }
            Ok(json!({}))
        }
        E2eMockAction::KnownHostAdd(entry) => {
            with_core(&state, "e2e_mock", move |core| core.known_host_add(entry)).await?;
            Ok(json!({}))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reason(result: Result<E2eMockAction, CmdError>) -> Value {
        serde_json::to_value(result.unwrap_err()).unwrap()
    }

    #[test]
    fn parses_every_action() {
        for input in [
            json!({"kind": "clipboard_read"}),
            json!({"kind": "clipboard_write", "text": "x"}),
            json!({"kind": "launcher_requests"}),
            json!({"kind": "secret_store_set_available", "available": false}),
            json!({"kind": "secret_store_accounts"}),
            json!({"kind": "notify", "level": "info", "message_key": "app.test"}),
            json!({"kind": "notify", "level": "error", "message_key": "app.test", "params": {"a": "b"}}),
            json!({"kind": "clock_advance", "seconds": 61}),
            json!({"kind": "clock_advance", "seconds": 0}),
            json!({"kind": "clock_advance", "seconds": 86400}),
            json!({"kind": "inject_link", "url": "shellpp://host/0199f6a1-b2c3-7d4e-8f00-112233445566"}),
            json!({"kind": "known_host_add", "host": "127.0.0.1", "port": 2222, "key_type": "ssh-ed25519", "public_key": "AAAAC3NzaC1lZDI1NTE5AAAAIA=="}),
            json!({"kind": "known_host_add", "host": "|1|c2FsdA==|aGFzaA==", "key_type": "ssh-ed25519", "public_key": "AAAA", "revoked": true, "comment": "x"}),
        ] {
            assert!(parse_action(input.clone()).is_ok(), "{input}");
        }
    }

    #[test]
    fn refuses_bad_actions() {
        assert_eq!(
            reason(parse_action(json!({"kind": "format_disk"}))),
            json!({"code": "validation_failed", "details": {"errors": [{"field": "kind", "reason": "invalid"}]}})
        );
        assert_eq!(
            reason(parse_action(
                json!({"kind": "clock_advance", "seconds": -1})
            )),
            json!({"code": "validation_failed", "details": {"errors": [{"field": "kind", "reason": "invalid"}]}})
        );
        assert_eq!(
            reason(parse_action(
                json!({"kind": "clock_advance", "seconds": 86401})
            )),
            json!({"code": "validation_failed", "details": {"errors": [{"field": "seconds", "reason": "out_of_range"}]}})
        );
        assert!(
            parse_action(json!({"kind": "notify", "level": "info", "message_key": ""})).is_err()
        );
        assert!(parse_action(json!({"kind": "clipboard_write"})).is_err());
        assert!(parse_action(json!({"kind": "clipboard_write", "text": "x", "extra": 1})).is_err());
        assert!(parse_action(json!("clipboard_read")).is_err());
        assert!(parse_action(json!({"kind": "known_host_add", "host": "h"})).is_err());
    }
}
