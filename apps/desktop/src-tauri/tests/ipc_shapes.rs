//! IPC shapes: one JSON sample per command input and output, written from
//! `apps/desktop/src/ipc/types.ts` (plan decision 6: the TypeScript types are
//! hand-written, so these tests catch drift between the two sides). The W4
//! mock backend uses the same shapes.

use std::collections::BTreeMap;

use secrecy::{ExposeSecret, SecretString};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use shellpp_core::error::{CoreError, SshReason};
use shellpp_core::platform::OsKind;
use shellpp_core::service::dto::*;
use shellpp_desktop::commands::app::{
    ExternalOpenInput, LinkSource, StateSnapshot, WindowOpenResult,
};
use shellpp_desktop::commands::clipboard::{ClipboardText, ClipboardWriteInput};
use shellpp_desktop::commands::sessions::SessionAttachInput;
use shellpp_desktop::error::CmdError;
use shellpp_desktop::events::{SESSIONS_CHANGED_EVENT, WINDOW_CLOSE_REQUESTED_EVENT};
use shellpp_desktop::links::Link;
use uuid::Uuid;

const VAULT: &str = "0199f6a1-b2c3-7d4e-8f00-000000000001";
const HOST: &str = "0199f6a1-b2c3-7d4e-8f00-000000000002";
const GROUP: &str = "0199f6a1-b2c3-7d4e-8f00-000000000003";
const PARENT: &str = "0199f6a1-b2c3-7d4e-8f00-000000000004";
const TS: &str = "2026-10-18T22:31:05.123Z";

fn id(text: &str) -> Uuid {
    Uuid::parse_str(text).unwrap()
}

fn input<T: DeserializeOwned>(sample: Value) -> T {
    match serde_json::from_value(sample.clone()) {
        Ok(value) => value,
        Err(error) => panic!("sample {sample} does not deserialize: {error}"),
    }
}

fn output<T: serde::Serialize>(value: &T) -> Value {
    serde_json::to_value(value).expect("serializable")
}

// ---------------------------------------------------------------------------
// Inputs
// ---------------------------------------------------------------------------

#[test]
fn app_inputs() {
    let open: ExternalOpenInput = input(json!({"url": "https://example.com/docs"}));
    assert_eq!(open.url, "https://example.com/docs");
    let write: ClipboardWriteInput = input(json!({"text": "shellpp://host/x"}));
    assert_eq!(write.text, "shellpp://host/x");
}

#[test]
fn lock_inputs() {
    let first: FirstRunInput = input(json!({"password": "pw-sample-1"}));
    assert_eq!(first.password.expose_secret(), "pw-sample-1");
    let unlock: PasswordInput = input(json!({"password": "pw-sample-2"}));
    assert_eq!(unlock.password.expose_secret(), "pw-sample-2");
    let recovery: RecoveryUnlockInput =
        input(json!({"recovery_key": "RK1-AAAAA", "new_password": "pw-sample-3"}));
    assert_eq!(recovery.recovery_key.expose_secret(), "RK1-AAAAA");
    assert_eq!(recovery.new_password.expose_secret(), "pw-sample-3");
    let change: PasswordChangeInput =
        input(json!({"current_password": "pw-a", "new_password": "pw-b"}));
    assert_eq!(change.current_password.expose_secret(), "pw-a");
    assert_eq!(change.new_password.expose_secret(), "pw-b");
    let remember: RememberDeviceInput = input(json!({"enabled": true}));
    assert!(remember.enabled);
    // Debug output never shows the secrets.
    assert!(!format!("{first:?}{unlock:?}{recovery:?}{change:?}").contains("pw-"));
}

#[test]
fn records_query_inputs() {
    let minimal: RecordsQuery = input(json!({"types": ["host"]}));
    assert_eq!(minimal.vault_id, None);
    assert_eq!(minimal.types, vec![RecordType::Host]);
    assert_eq!(minimal.scope, QueryScope::All);
    assert_eq!(minimal.text, None);
    assert!(minimal.tags.is_empty());
    assert_eq!(minimal.sort, SortMode::Manual);

    let full: RecordsQuery = input(json!({
        "vault_id": VAULT,
        "types": ["host", "group"],
        "scope": {"kind": "group", "group_id": GROUP},
        "text": "web",
        "tags": ["prod", "db"],
        "sort": "modified"
    }));
    assert_eq!(full.vault_id, Some(id(VAULT)));
    assert_eq!(full.types, vec![RecordType::Host, RecordType::Group]);
    assert_eq!(
        full.scope,
        QueryScope::Group {
            group_id: id(GROUP)
        }
    );
    assert_eq!(full.text.as_deref(), Some("web"));
    assert_eq!(full.tags, vec!["prod", "db"]);
    assert_eq!(full.sort, SortMode::Modified);
    assert!(!format!("{full:?}").contains("web"));

    for (scope, expected) in [
        (json!({"kind": "all"}), QueryScope::All),
        (json!({"kind": "top_level"}), QueryScope::TopLevel),
    ] {
        let query: RecordsQuery = input(json!({"types": ["group"], "scope": scope}));
        assert_eq!(query.scope, expected);
    }
    for (sort, expected) in [
        ("manual", SortMode::Manual),
        ("name", SortMode::Name),
        ("created", SortMode::Created),
        ("modified", SortMode::Modified),
    ] {
        let query: RecordsQuery = input(json!({"types": [], "sort": sort}));
        assert_eq!(query.sort, expected);
    }
}

#[test]
fn record_ref_inputs() {
    let one: RecordRef = input(json!({"vault_id": VAULT, "record_id": HOST}));
    assert_eq!((one.vault_id, one.record_id), (id(VAULT), id(HOST)));
    let many: RecordsRef = input(json!({"vault_id": VAULT, "record_ids": [HOST, GROUP]}));
    assert_eq!(many.record_ids, vec![id(HOST), id(GROUP)]);
    let field: SecretFieldRef =
        input(json!({"vault_id": VAULT, "record_id": HOST, "path": "ssh.password"}));
    assert_eq!(field.path, "ssh.password");
}

#[test]
fn record_save_inputs() {
    let host: RecordSaveInput = input(json!({
        "vault_id": VAULT,
        "record_id": null,
        "type": "host",
        "data": {
            "address": "192.0.2.10",
            "label": "web",
            "group_id": GROUP,
            "tags": ["prod"],
            "color": "jade",
            "ssh": {"port": 2222, "username": "deploy", "env": [{"name": "LANG", "value": "C"}]},
            "x_unknown": {"kept": true}
        },
        "secrets": {"ssh.password": "pw-sample", "ssh.proxy.password": null}
    }));
    assert_eq!(host.record_id, None);
    assert_eq!(host.record_type, RecordType::Host);
    assert_eq!(host.data["address"], "192.0.2.10");
    assert_eq!(host.data["x_unknown"]["kept"], true);
    assert_eq!(
        host.secrets["ssh.password"]
            .as_ref()
            .map(|secret| secret.expose_secret().to_owned()),
        Some("pw-sample".to_owned())
    );
    assert!(host.secrets["ssh.proxy.password"].is_none());
    let debug = format!("{host:?}");
    assert!(!debug.contains("pw-sample") && !debug.contains("192.0.2.10"));

    let group: RecordSaveInput = input(json!({
        "vault_id": VAULT,
        "record_id": GROUP,
        "type": "group",
        "data": {"name": "prod", "parent_id": PARENT, "ssh": {"port": 22}}
    }));
    assert_eq!(group.record_id, Some(id(GROUP)));
    assert_eq!(group.record_type, RecordType::Group);
    assert!(group.secrets.is_empty());

    let omitted: RecordSaveInput =
        input(json!({"vault_id": VAULT, "type": "host", "data": {"address": "example.com"}}));
    assert_eq!(omitted.record_id, None);
}

#[test]
fn record_move_inputs() {
    let top: RecordMoveInput = input(json!({"vault_id": VAULT, "record_ids": [HOST]}));
    assert_eq!(top.target_group_id, None);
    assert_eq!(top.position, MovePosition::End);
    let null_target: RecordMoveInput =
        input(json!({"vault_id": VAULT, "record_ids": [HOST], "target_group_id": null}));
    assert_eq!(null_target.target_group_id, None);
    for (position, expected) in [
        (json!({"kind": "end"}), MovePosition::End),
        (
            json!({"kind": "before", "record_id": HOST}),
            MovePosition::Before {
                record_id: id(HOST),
            },
        ),
        (
            json!({"kind": "after", "record_id": HOST}),
            MovePosition::After {
                record_id: id(HOST),
            },
        ),
    ] {
        let moved: RecordMoveInput = input(json!({
            "vault_id": VAULT,
            "record_ids": [GROUP],
            "target_group_id": PARENT,
            "position": position
        }));
        assert_eq!(moved.target_group_id, Some(id(PARENT)));
        assert_eq!(moved.position, expected);
    }
}

#[test]
fn secret_copy_inputs() {
    let field: SecretCopyTarget = input(json!({
        "kind": "record_field", "vault_id": VAULT, "record_id": HOST, "path": "ssh.password"
    }));
    assert!(
        matches!(field, SecretCopyTarget::RecordField { ref path, .. } if path == "ssh.password")
    );
    let recovery: SecretCopyTarget = input(json!({"kind": "recovery_key"}));
    assert!(matches!(recovery, SecretCopyTarget::RecoveryKey));
}

#[test]
fn settings_inputs() {
    let set: SettingsSetInput = input(json!({
        "values": {"security.auto_lock_minutes": null, "view.hosts.layout": "list"},
        "reset": ["appearance.ui_zoom"]
    }));
    assert_eq!(set.values["security.auto_lock_minutes"], Value::Null);
    assert_eq!(set.values["view.hosts.layout"], "list");
    assert_eq!(set.reset, vec!["appearance.ui_zoom"]);
    let empty: SettingsSetInput = input(json!({}));
    assert!(empty.values.is_empty() && empty.reset.is_empty());
}

#[cfg(feature = "e2e-mocks")]
#[test]
fn e2e_mock_inputs() {
    use shellpp_desktop::commands::e2e::parse_action;
    for sample in [
        json!({"kind": "clipboard_read"}),
        json!({"kind": "clipboard_write", "text": "t"}),
        json!({"kind": "launcher_requests"}),
        json!({"kind": "secret_store_set_available", "available": true}),
        json!({"kind": "secret_store_accounts"}),
        json!({"kind": "notify", "level": "warning", "message_key": "app.k", "params": {"n": "1"}}),
        json!({"kind": "clock_advance", "seconds": 31}),
        json!({"kind": "inject_link", "url": format!("shellpp://host/{HOST}")}),
    ] {
        assert!(parse_action(sample.clone()).is_ok(), "{sample}");
    }
}

// ---------------------------------------------------------------------------
// Outputs
// ---------------------------------------------------------------------------

fn lock_status() -> LockStatus {
    LockStatus {
        state: LockState::Unlocked,
        remember_device_available: true,
        remember_device_enabled: false,
    }
}

fn notification() -> Notification {
    Notification {
        id: 3,
        level: NotificationLevel::Success,
        message_key: "app.test".to_owned(),
        params: BTreeMap::from([("n".to_owned(), "1".to_owned())]),
        source: NotificationSource {
            kind: NotificationSourceKind::App,
            reference: None,
        },
        created_at: TS.to_owned(),
        read: false,
    }
}

fn notification_json() -> Value {
    json!({
        "id": 3,
        "level": "success",
        "message_key": "app.test",
        "params": {"n": "1"},
        "source": {"kind": "app"},
        "created_at": TS,
        "read": false
    })
}

#[test]
fn app_outputs() {
    let info = AppInfo {
        name: "Shell++".to_owned(),
        version: "0.1.0".to_owned(),
        os: OsKind::Windows,
        arch: "x86_64".to_owned(),
        debug: true,
        e2e: false,
    };
    assert_eq!(
        output(&info),
        json!({"name": "Shell++", "version": "0.1.0", "os": "windows", "arch": "x86_64", "debug": true, "e2e": false})
    );
    assert_eq!(
        output(&WindowOpenResult {
            label: "w-2".to_owned()
        }),
        json!({"label": "w-2"})
    );

    let snapshot = StateSnapshot {
        core: CoreSnapshot {
            v: 42,
            lock: lock_status(),
            settings: BTreeMap::from([
                ("appearance.ui_theme".to_owned(), json!("auto")),
                ("security.auto_lock_minutes".to_owned(), json!(15)),
            ]),
            notifications: NotificationList {
                items: vec![],
                unread: 0,
            },
            vaults: vec![VaultView {
                vault_id: id(VAULT),
                kind: VaultKind::Personal,
                status: VaultStatus::Active,
                role: None,
                name: "Personal".to_owned(),
                color: TagColor::Graphite,
            }],
        },
        window_label: "w-1".to_owned(),
        launched_at: "2026-10-19T08:00:00.000Z".to_owned(),
        sessions: vec![],
        intent: None,
    };
    assert_eq!(
        output(&snapshot),
        json!({
            "v": 42,
            "lock": {"state": "unlocked", "remember_device_available": true, "remember_device_enabled": false},
            "settings": {"appearance.ui_theme": "auto", "security.auto_lock_minutes": 15},
            "notifications": {"items": [], "unread": 0},
            "vaults": [{"vault_id": VAULT, "kind": "personal", "status": "active", "name": "Personal", "color": "graphite"}],
            "window_label": "w-1",
            "launched_at": "2026-10-19T08:00:00.000Z",
            "sessions": []
        })
    );
}

#[test]
fn lock_outputs() {
    for (state, text) in [
        (LockState::FirstRun, "first_run"),
        (LockState::Locked, "locked"),
        (LockState::Unlocked, "unlocked"),
    ] {
        let status = LockStatus {
            state,
            ..lock_status()
        };
        assert_eq!(output(&status)["state"], text);
    }
    let key = RecoveryKeyResult {
        recovery_key: SecretString::from("RK1-AAAAA-BBBBB"),
    };
    assert_eq!(output(&key), json!({"recovery_key": "RK1-AAAAA-BBBBB"}));
    assert!(!format!("{key:?}").contains("RK1"));
}

#[test]
fn record_outputs() {
    let host = RecordView::Host(HostView {
        vault_id: id(VAULT),
        record_id: id(HOST),
        revision: 2,
        created_at: TS.to_owned(),
        modified_at: TS.to_owned(),
        data: json!({"address": "192.0.2.10", "group_id": GROUP, "order_key": "V"}),
        secrets_set: vec!["ssh.password".to_owned()],
        inherited: InheritedValues {
            port: Some(Inherited {
                value: 2222,
                from_group_id: id(PARENT),
            }),
            username: Some(Inherited {
                value: "deploy".to_owned(),
                from_group_id: id(GROUP),
            }),
            ..InheritedValues::default()
        },
        path: vec![id(PARENT), id(GROUP)],
    });
    assert_eq!(
        output(&host),
        json!({
            "type": "host",
            "vault_id": VAULT,
            "record_id": HOST,
            "revision": 2,
            "created_at": TS,
            "modified_at": TS,
            "data": {"address": "192.0.2.10", "group_id": GROUP, "order_key": "V"},
            "secrets_set": ["ssh.password"],
            "inherited": {
                "port": {"value": 2222, "from_group_id": PARENT},
                "username": {"value": "deploy", "from_group_id": GROUP}
            },
            "path": [PARENT, GROUP]
        })
    );
    assert!(!format!("{host:?}").contains("192.0.2.10"));

    let group = RecordView::Group(GroupView {
        vault_id: id(VAULT),
        record_id: id(GROUP),
        revision: 1,
        created_at: TS.to_owned(),
        modified_at: TS.to_owned(),
        data: json!({"name": "prod", "parent_id": PARENT}),
        inherited: InheritedValues::default(),
        path: vec![id(PARENT)],
        host_count: 4,
        child_count: 2,
    });
    assert_eq!(
        output(&group),
        json!({
            "type": "group",
            "vault_id": VAULT,
            "record_id": GROUP,
            "revision": 1,
            "created_at": TS,
            "modified_at": TS,
            "data": {"name": "prod", "parent_id": PARENT},
            "inherited": {},
            "path": [PARENT],
            "host_count": 4,
            "child_count": 2
        })
    );

    let deleted = RecordDeleteResult {
        deleted: vec![id(GROUP)],
        moved: vec![id(HOST)],
    };
    assert_eq!(
        output(&deleted),
        json!({"deleted": [GROUP], "moved": [HOST]})
    );
    let secret = SecretValue {
        value: SecretString::from("pw-sample"),
    };
    assert_eq!(output(&secret), json!({"value": "pw-sample"}));
    assert!(!format!("{secret:?}").contains("pw-sample"));
}

#[test]
fn notification_and_clipboard_outputs() {
    let list = NotificationList {
        items: vec![notification()],
        unread: 1,
    };
    assert_eq!(
        output(&list),
        json!({"items": [notification_json()], "unread": 1})
    );
    let with_ref = NotificationSource {
        kind: NotificationSourceKind::Session,
        reference: Some("tab-1".to_owned()),
    };
    assert_eq!(
        output(&with_ref),
        json!({"kind": "session", "ref": "tab-1"})
    );

    assert_eq!(output(&ClipboardText { text: None }), json!({"text": null}));
}

#[test]
fn event_payloads() {
    let events = [
        (
            CoreEventKind::LockChanged {
                locked: true,
                state: LockState::Locked,
            },
            "lock:changed",
            json!({"v": 1, "locked": true, "state": "locked"}),
        ),
        (
            CoreEventKind::RecordsChanged {
                vault_id: id(VAULT),
                record_ids: vec![id(HOST)],
                classes: vec![2],
            },
            "records:changed",
            json!({"v": 1, "vault_id": VAULT, "record_ids": [HOST], "classes": [2]}),
        ),
        (
            CoreEventKind::VaultsChanged {
                vault_ids: vec![id(VAULT)],
            },
            "vaults:changed",
            json!({"v": 1, "vault_ids": [VAULT]}),
        ),
        (
            CoreEventKind::SettingsChanged {
                keys: vec!["view.hosts.sort".to_owned()],
            },
            "settings:changed",
            json!({"v": 1, "keys": ["view.hosts.sort"]}),
        ),
        (
            CoreEventKind::NotificationsChanged {
                unread: 0,
                latest: None,
            },
            "notifications:changed",
            json!({"v": 1, "unread": 0}),
        ),
        (
            CoreEventKind::NotificationsChanged {
                unread: 1,
                latest: Some(notification()),
            },
            "notifications:changed",
            json!({"v": 1, "unread": 1, "latest": notification_json()}),
        ),
    ];
    for (kind, name, payload) in events {
        let event = CoreEvent { v: 1, kind };
        assert_eq!(event.name(), name);
        assert_eq!(event.payload(), payload);
    }
    assert_eq!(
        output(&Link::Host {
            record_id: id(HOST)
        }),
        json!({"kind": "host", "record_id": HOST})
    );
}

#[test]
fn error_shape() {
    assert_eq!(
        output(&CmdError::locked()),
        json!({"code": "locked", "details": {}})
    );
}

// ---------------------------------------------------------------------------
// Stage 2 (stage 2 plan, section 5; `apps/desktop/src/ipc/types.ts`)
// ---------------------------------------------------------------------------

const SESSION: &str = "0199f6a1-b2c3-7d4e-8f00-000000000010";
const KEY: &str = "0199f6a1-b2c3-7d4e-8f00-000000000011";

fn host_target_json() -> Value {
    json!({"kind": "host", "vault_id": VAULT, "record_id": HOST})
}

#[test]
fn session_inputs() {
    let open: SessionOpenInput =
        input(json!({"target": host_target_json(), "cols": 80, "rows": 24}));
    assert_eq!(
        open.target,
        SessionTarget::Host {
            vault_id: id(VAULT),
            record_id: id(HOST)
        }
    );
    assert_eq!((open.cols, open.rows), (80, 24));
    let quick: SessionOpenInput = input(json!({
        "target": {"kind": "quick", "address": "example.com", "port": 2222, "username": "deploy"},
        "cols": 120, "rows": 40
    }));
    assert_eq!(
        quick.target,
        SessionTarget::Quick {
            address: "example.com".into(),
            port: Some(2222),
            username: Some("deploy".into())
        }
    );
    // Debug never shows the quick-connect address.
    assert!(!format!("{quick:?}").contains("example.com"));
    let bare: SessionOpenInput = input(json!({
        "target": {"kind": "quick", "address": "192.0.2.1"}, "cols": 1, "rows": 1
    }));
    assert_eq!(
        bare.target,
        SessionTarget::Quick {
            address: "192.0.2.1".into(),
            port: None,
            username: None
        }
    );

    let attach: SessionAttachInput =
        input(json!({"session_id": SESSION, "on_output": "__CHANNEL__:7"}));
    assert_eq!(attach.session_id, id(SESSION));

    let reference: SessionRef = input(json!({"session_id": SESSION}));
    assert_eq!(reference.session_id, id(SESSION));

    let write: SessionWriteInput = input(json!({
        "session_id": SESSION,
        "data": "ls -la\r",
        "binary": false,
        "history": {"command": "ls -la", "prompt_text": "$ ", "typed_printable": true, "alt_screen": false}
    }));
    assert_eq!(write.data, "ls -la\r");
    assert!(!write.binary);
    let history = write.history.as_ref().unwrap();
    assert_eq!(history.command, "ls -la");
    assert_eq!(history.prompt_text, "$ ");
    assert!(history.typed_printable);
    assert!(!history.alt_screen);
    assert!(!format!("{write:?}").contains("ls -la"));
    let minimal: SessionWriteInput = input(json!({"session_id": SESSION, "data": "\u{1b}[A"}));
    assert!(!minimal.binary);
    assert!(minimal.history.is_none());
    let binary: SessionWriteInput =
        input(json!({"session_id": SESSION, "data": "\u{1b}[M", "binary": true}));
    assert!(binary.binary);

    let inject: SessionInjectInput = input(json!({"session_id": SESSION, "data": "git status"}));
    assert_eq!(inject.data, "git status");
    assert!(!format!("{inject:?}").contains("git"));

    let resize: SessionResizeInput = input(json!({"session_id": SESSION, "cols": 132, "rows": 43}));
    assert_eq!((resize.cols, resize.rows), (132, 43));
}

#[test]
fn prompt_reply_inputs() {
    for (decision, expected) in [
        ("accept_save", HostKeyDecision::AcceptSave),
        ("connect_once", HostKeyDecision::ConnectOnce),
        ("replace", HostKeyDecision::Replace),
        ("cancel", HostKeyDecision::Cancel),
    ] {
        let reply: SessionPromptReplyInput = input(json!({
            "session_id": SESSION, "prompt_id": 3,
            "reply": {"kind": "host_key", "decision": decision}
        }));
        assert_eq!(reply.prompt_id, 3);
        assert!(matches!(reply.reply, PromptReply::HostKey { decision } if decision == expected));
    }
    let text: SessionPromptReplyInput = input(json!({
        "session_id": SESSION, "prompt_id": 4,
        "reply": {"kind": "text", "value": "pw-sample-9", "save": true}
    }));
    match &text.reply {
        PromptReply::Text { value, save } => {
            assert_eq!(value.expose_secret(), "pw-sample-9");
            assert!(*save);
        }
        _ => panic!("text reply"),
    }
    assert!(!format!("{text:?}").contains("pw-sample"));
    let unsaved: SessionPromptReplyInput = input(json!({
        "session_id": SESSION, "prompt_id": 5, "reply": {"kind": "text", "value": "alice"}
    }));
    assert!(matches!(
        unsaved.reply,
        PromptReply::Text { save: false, .. }
    ));
    let cancel: SessionPromptReplyInput = input(json!({
        "session_id": SESSION, "prompt_id": 6, "reply": {"kind": "cancel"}
    }));
    assert!(matches!(cancel.reply, PromptReply::Cancel));
}

#[test]
fn local_and_window_inputs() {
    let local: LocalOpenInput = input(json!({"shell_id": "pwsh", "cols": 80, "rows": 24}));
    assert_eq!(local.shell_id.as_deref(), Some("pwsh"));
    let default: LocalOpenInput = input(json!({"shell_id": null, "cols": 80, "rows": 24}));
    assert_eq!(default.shell_id, None);
    let omitted: LocalOpenInput = input(json!({"cols": 80, "rows": 24}));
    assert_eq!(omitted.shell_id, None);

    let open: WindowOpenInput =
        input(json!({"intent": {"kind": "open_session", "target": host_target_json()}}));
    assert_eq!(
        open.intent,
        Some(WindowIntent::OpenSession {
            target: SessionTarget::Host {
                vault_id: id(VAULT),
                record_id: id(HOST)
            }
        })
    );
    let local_intent: WindowOpenInput =
        input(json!({"intent": {"kind": "open_local", "shell_id": "cmd"}}));
    assert_eq!(
        local_intent.intent,
        Some(WindowIntent::OpenLocal {
            shell_id: Some("cmd".into())
        })
    );
    let bare: WindowOpenInput = input(json!({"intent": {"kind": "open_local"}}));
    assert_eq!(
        bare.intent,
        Some(WindowIntent::OpenLocal { shell_id: None })
    );
    let empty: WindowOpenInput = input(json!({}));
    assert_eq!(empty.intent, None);

    let exit: ExitInput = input(json!({"confirmed": true}));
    assert!(exit.confirmed);
    let exit: ExitInput = input(json!({}));
    assert!(!exit.confirmed);

    let terminal: ExternalOpenInput =
        input(json!({"url": "http://example.com/x", "source": "terminal"}));
    assert_eq!(terminal.source, Some(LinkSource::Terminal));
    let plain: ExternalOpenInput = input(json!({"url": "https://example.com/x"}));
    assert_eq!(plain.source, None);
}

#[test]
fn key_and_history_inputs() {
    let import: KeyImportInput = input(json!({
        "vault_id": VAULT,
        "label": "deploy key",
        "private_key": "-----BEGIN OPENSSH PRIVATE KEY-----\nsample\n-----END OPENSSH PRIVATE KEY-----\n",
        "passphrase": "pp-sample-1",
        "save_passphrase": true
    }));
    assert_eq!(import.vault_id, id(VAULT));
    assert_eq!(import.label.as_deref(), Some("deploy key"));
    assert!(import.private_key.expose_secret().contains("sample"));
    assert_eq!(
        import.passphrase.as_ref().unwrap().expose_secret(),
        "pp-sample-1"
    );
    assert!(import.save_passphrase);
    assert!(!format!("{import:?}").contains("sample"));
    let minimal: KeyImportInput = input(json!({"vault_id": VAULT, "private_key": "k"}));
    assert_eq!(minimal.label, None);
    assert!(minimal.passphrase.is_none());
    assert!(!minimal.save_passphrase);

    let list: HistoryListInput = input(json!({
        "host": {"vault_id": VAULT, "record_id": HOST}, "text": "git", "limit": 100
    }));
    assert_eq!(
        list.host,
        Some(RecordRef {
            vault_id: id(VAULT),
            record_id: id(HOST)
        })
    );
    assert_eq!(list.text.as_deref(), Some("git"));
    assert_eq!(list.limit, Some(100));
    let none: HistoryListInput = input(json!({"host": null}));
    assert_eq!(none.host, None);
    let empty: HistoryListInput = input(json!({}));
    assert_eq!((empty.host, empty.text, empty.limit), (None, None, None));

    let clear: HistoryClearInput = input(json!({"host": {"vault_id": VAULT, "record_id": HOST}}));
    assert!(clear.host.is_some());
    let clear: HistoryClearInput = input(json!({}));
    assert!(clear.host.is_none());

    let remove: RecordsRef = input(json!({"vault_id": VAULT, "record_ids": [KEY]}));
    assert_eq!(remove.record_ids, vec![id(KEY)]);
}

#[cfg(feature = "e2e-mocks")]
#[test]
fn e2e_known_host_add_input() {
    use shellpp_desktop::commands::e2e::parse_action;
    for sample in [
        json!({"kind": "known_host_add", "host": "127.0.0.1", "port": 2222, "key_type": "ssh-ed25519", "public_key": "AAAA"}),
        json!({"kind": "known_host_add", "host": "|1|c2FsdA==|aGFzaA==", "key_type": "ssh-ed25519", "public_key": "AAAA", "revoked": true, "comment": "c"}),
    ] {
        assert!(parse_action(sample.clone()).is_ok(), "{sample}");
    }
}

fn host_key_info() -> HostKeyInfo {
    HostKeyInfo {
        address: "192.0.2.5".into(),
        port: 2222,
        key_type: "ssh-ed25519".into(),
        fingerprint: "SHA256:abc".into(),
    }
}

fn host_key_json() -> Value {
    json!({"address": "192.0.2.5", "port": 2222, "key_type": "ssh-ed25519", "fingerprint": "SHA256:abc"})
}

fn session_info() -> SessionInfo {
    SessionInfo {
        session_id: id(SESSION),
        rev: 7,
        kind: SessionKind::Ssh,
        window: "w-1".into(),
        state: SessionState::Connecting,
        step: Some(ConnectStep::HostKey),
        title: "web".into(),
        target: Some(SessionTarget::Host {
            vault_id: id(VAULT),
            record_id: id(HOST),
        }),
        shell_id: None,
        address: Some("192.0.2.5".into()),
        port: Some(2222),
        username: Some("alice".into()),
        os: Some(HostOs {
            family: OsFamily::Linux,
            distro: Some("ubuntu".into()),
            version: Some("24.04".into()),
            source: OsSource::Banner,
            detected_at: TS.into(),
        }),
        terminal_theme: json!({"builtin": "graphite"}),
        backspace: Backspace::CtrlH,
        record_history: true,
        prompt: Some(SessionPrompt {
            prompt_id: 2,
            kind: PromptKind::HostKeyUnknown {
                key: host_key_info(),
            },
        }),
        error: None,
        host_key: None,
        exit_status: None,
        reconnect: None,
    }
}

#[test]
fn session_outputs() {
    assert_eq!(
        output(&session_info()),
        json!({
            "session_id": SESSION,
            "rev": 7,
            "kind": "ssh",
            "window": "w-1",
            "state": "connecting",
            "step": "host_key",
            "title": "web",
            "target": host_target_json(),
            "address": "192.0.2.5",
            "port": 2222,
            "username": "alice",
            "os": {"family": "linux", "distro": "ubuntu", "version": "24.04", "source": "banner", "detected_at": TS},
            "terminal_theme": {"builtin": "graphite"},
            "backspace": "ctrl_h",
            "record_history": true,
            "prompt": {"prompt_id": 2, "kind": "host_key_unknown", "key": host_key_json()}
        })
    );
    // Debug carries ids and states only.
    let debug = format!("{:?}", session_info());
    assert!(!debug.contains("192.0.2.5") && !debug.contains("alice") && !debug.contains("web"));

    let dropped = SessionInfo {
        state: SessionState::Dropped,
        step: None,
        prompt: None,
        error: Some(SessionError {
            code: "ssh_error".into(),
            reason: Some("disconnected".into()),
        }),
        reconnect: Some(ReconnectInfo {
            attempt: 2,
            next_in_ms: Some(2000),
            waiting_for_unlock: false,
            in_progress: false,
        }),
        ..session_info()
    };
    let json = output(&dropped);
    assert_eq!(
        json["error"],
        json!({"code": "ssh_error", "reason": "disconnected"})
    );
    assert_eq!(
        json["reconnect"],
        json!({"attempt": 2, "next_in_ms": 2000, "waiting_for_unlock": false, "in_progress": false})
    );
    assert!(json.get("prompt").is_none() && json.get("step").is_none());

    let waiting = ReconnectInfo {
        attempt: 3,
        next_in_ms: None,
        waiting_for_unlock: true,
        in_progress: false,
    };
    assert_eq!(
        output(&waiting),
        json!({"attempt": 3, "waiting_for_unlock": true, "in_progress": false})
    );

    let revoked = SessionInfo {
        state: SessionState::Failed,
        step: None,
        prompt: None,
        error: Some(SessionError {
            code: "host_key_revoked".into(),
            reason: None,
        }),
        host_key: Some(host_key_info()),
        ..session_info()
    };
    let json = output(&revoked);
    assert_eq!(json["error"], json!({"code": "host_key_revoked"}));
    assert_eq!(json["host_key"], host_key_json());

    let local = SessionInfo {
        kind: SessionKind::Local,
        state: SessionState::Ended,
        step: None,
        title: "PowerShell".into(),
        target: None,
        shell_id: Some("powershell".into()),
        address: None,
        port: None,
        username: None,
        os: None,
        backspace: Backspace::Default,
        prompt: None,
        exit_status: Some(3),
        ..session_info()
    };
    assert_eq!(
        output(&local),
        json!({
            "session_id": SESSION, "rev": 7, "kind": "local", "window": "w-1", "state": "ended",
            "title": "PowerShell", "shell_id": "powershell",
            "terminal_theme": {"builtin": "graphite"}, "backspace": "default",
            "record_history": true, "exit_status": 3
        })
    );
    for (state, text) in [
        (SessionState::Connected, "connected"),
        (SessionState::Closed, "closed"),
    ] {
        assert_eq!(output(&state), json!(text));
    }
    for (step, text) in [
        (ConnectStep::Resolving, "resolving"),
        (ConnectStep::Connecting, "connecting"),
        (ConnectStep::Authenticating, "authenticating"),
        (ConnectStep::OpeningSession, "opening_session"),
    ] {
        assert_eq!(output(&step), json!(text));
    }
    assert_eq!(output(&Backspace::Del), json!("del"));
}

#[test]
fn prompt_outputs() {
    let prompt = |kind| output(&SessionPrompt { prompt_id: 9, kind });
    assert_eq!(
        prompt(PromptKind::HostKeyChanged {
            key: host_key_info(),
            old_key_type: "ssh-rsa".into(),
            old_fingerprint: "SHA256:old".into()
        }),
        json!({"prompt_id": 9, "kind": "host_key_changed", "key": host_key_json(),
               "old_key_type": "ssh-rsa", "old_fingerprint": "SHA256:old"})
    );
    assert_eq!(
        prompt(PromptKind::Username),
        json!({"prompt_id": 9, "kind": "username"})
    );
    assert_eq!(
        prompt(PromptKind::Password {
            username: Some("alice".into()),
            error: true,
            can_save: false
        }),
        json!({"prompt_id": 9, "kind": "password", "username": "alice", "error": true, "can_save": false})
    );
    assert_eq!(
        prompt(PromptKind::Passphrase {
            key_label: None,
            error: false,
            can_save: true
        }),
        json!({"prompt_id": 9, "kind": "passphrase", "error": false, "can_save": true})
    );
    assert_eq!(
        prompt(PromptKind::KeyboardInteractive {
            name: "srv".into(),
            instruction: "2FA".into(),
            prompt: "Code: ".into(),
            echo: false,
            index: 1,
            count: 2,
            error: false
        }),
        json!({"prompt_id": 9, "kind": "keyboard_interactive", "name": "srv", "instruction": "2FA",
               "prompt": "Code: ", "echo": false, "index": 1, "count": 2, "error": false})
    );
}

#[test]
fn data_outputs() {
    let shells = LocalShellsResult {
        shells: vec![LocalShell {
            id: "powershell".into(),
            label: "PowerShell".into(),
        }],
        default_id: Some("powershell".into()),
    };
    assert_eq!(
        output(&shells),
        json!({"shells": [{"id": "powershell", "label": "PowerShell"}], "default_id": "powershell"})
    );
    assert_eq!(
        output(&LocalShellsResult {
            shells: vec![],
            default_id: None
        }),
        json!({"shells": []})
    );

    let known = KnownHostView {
        vault_id: id(VAULT),
        record_id: id(KEY),
        created_at: TS.into(),
        modified_at: TS.into(),
        host: "192.0.2.5".into(),
        port: Some(2222),
        hashed: false,
        display_address: Some("[192.0.2.5]:2222".into()),
        key_type: "ssh-ed25519".into(),
        fingerprint: "SHA256:abc".into(),
        revoked: false,
        source: KnownHostSource::Tofu,
        comment: None,
        host_ids: vec![id(HOST)],
    };
    assert_eq!(
        output(&known),
        json!({
            "vault_id": VAULT, "record_id": KEY, "created_at": TS, "modified_at": TS,
            "host": "192.0.2.5", "port": 2222, "hashed": false, "display_address": "[192.0.2.5]:2222",
            "key_type": "ssh-ed25519", "fingerprint": "SHA256:abc", "revoked": false,
            "source": "tofu", "host_ids": [HOST]
        })
    );
    let hashed = KnownHostView {
        host: "|1|c2FsdA==|aGFzaA==".into(),
        port: None,
        hashed: true,
        display_address: None,
        revoked: true,
        source: KnownHostSource::Import,
        comment: Some("c".into()),
        host_ids: vec![],
        ..known.clone()
    };
    let json = output(&hashed);
    assert!(json.get("port").is_none() && json.get("display_address").is_none());
    assert_eq!(json["source"], "import");
    assert_eq!(json["comment"], "c");
    assert_eq!(json["revoked"], true);
    assert_eq!(output(&KnownHostSource::Replaced), json!("replaced"));

    let key = KeyView {
        vault_id: id(VAULT),
        record_id: id(KEY),
        created_at: TS.into(),
        modified_at: TS.into(),
        label: "deploy".into(),
        algorithm: "ssh-ed25519".into(),
        fingerprint: Some("SHA256:key".into()),
        encrypted: true,
        passphrase_saved: false,
    };
    let key_json = json!({
        "vault_id": VAULT, "record_id": KEY, "created_at": TS, "modified_at": TS,
        "label": "deploy", "algorithm": "ssh-ed25519", "fingerprint": "SHA256:key",
        "encrypted": true, "passphrase_saved": false
    });
    assert_eq!(output(&key), key_json);
    assert_eq!(
        output(&KeyImportResult {
            key: key.clone(),
            created: true
        }),
        json!({"key": key_json, "created": true})
    );

    let entry = HistoryEntryView {
        vault_id: id(VAULT),
        record_id: id(KEY),
        command: "git status".into(),
        executed_at: TS.into(),
    };
    assert_eq!(
        output(&entry),
        json!({"vault_id": VAULT, "record_id": KEY, "command": "git status", "executed_at": TS})
    );
    assert!(!format!("{entry:?}").contains("git"));
    assert_eq!(
        output(&HistoryClearResult { removed: 4 }),
        json!({"removed": 4})
    );

    let recent = RecentConnection {
        host: Some(RecordRef {
            vault_id: id(VAULT),
            record_id: id(HOST),
        }),
        title: "web".into(),
        address: "192.0.2.5".into(),
        port: 22,
        username: Some("alice".into()),
        os: None,
        path: vec!["Personal".into(), "Prod".into()],
        last_at: TS.into(),
    };
    assert_eq!(
        output(&recent),
        json!({
            "host": {"vault_id": VAULT, "record_id": HOST}, "title": "web", "address": "192.0.2.5",
            "port": 22, "username": "alice", "path": ["Personal", "Prod"], "last_at": TS
        })
    );

    assert_eq!(
        output(&ExitResult {
            needs_confirmation: true,
            active_sessions: 2
        }),
        json!({"needs_confirmation": true, "active_sessions": 2})
    );
}

#[test]
fn snapshot_with_sessions_and_intent() {
    let snapshot = StateSnapshot {
        core: CoreSnapshot {
            v: 1,
            lock: lock_status(),
            settings: BTreeMap::new(),
            notifications: NotificationList {
                items: vec![],
                unread: 0,
            },
            vaults: vec![],
        },
        window_label: "w-2".to_owned(),
        launched_at: "2026-10-19T08:00:00.000Z".to_owned(),
        sessions: vec![session_info()],
        intent: Some(WindowIntent::OpenLocal {
            shell_id: Some("cmd".into()),
        }),
    };
    let json = output(&snapshot);
    assert_eq!(json["window_label"], "w-2");
    assert_eq!(json["sessions"][0]["session_id"], SESSION);
    assert_eq!(json["sessions"][0]["rev"], 7);
    assert_eq!(
        json["intent"],
        json!({"kind": "open_local", "shell_id": "cmd"})
    );
    let session_intent = WindowIntent::OpenSession {
        target: SessionTarget::Quick {
            address: "example.com".into(),
            port: None,
            username: None,
        },
    };
    assert_eq!(
        output(&session_intent),
        json!({"kind": "open_session", "target": {"kind": "quick", "address": "example.com"}})
    );
}

#[test]
fn stage_2_event_names_and_errors() {
    assert_eq!(SESSIONS_CHANGED_EVENT, "sessions:changed");
    assert_eq!(WINDOW_CLOSE_REQUESTED_EVENT, "window:close-requested");
    assert_eq!(
        output(&CmdError::from(CoreError::Cancelled)),
        json!({"code": "cancelled", "details": {}})
    );
    assert_eq!(
        output(&CmdError::from(CoreError::Ssh(SshReason::Refused))),
        json!({"code": "ssh_error", "details": {"reason": "refused"}})
    );
    assert_eq!(
        output(&CmdError::from(CoreError::HostKeyRevoked)),
        json!({"code": "host_key_revoked", "details": {}})
    );
}
