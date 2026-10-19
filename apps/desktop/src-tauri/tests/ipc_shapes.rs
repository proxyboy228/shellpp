//! IPC shapes: one JSON sample per command input and output, written from
//! `apps/desktop/src/ipc/types.ts` (plan decision 6: the TypeScript types are
//! hand-written, so these tests catch drift between the two sides). The W4
//! mock backend uses the same shapes.

use std::collections::BTreeMap;

use secrecy::{ExposeSecret, SecretString};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use shellpp_core::platform::OsKind;
use shellpp_core::service::dto::*;
use shellpp_desktop::commands::app::{ExternalOpenInput, StateSnapshot, WindowOpenResult};
use shellpp_desktop::commands::clipboard::{ClipboardText, ClipboardWriteInput};
use shellpp_desktop::error::CmdError;
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
    };
    assert_eq!(
        output(&snapshot),
        json!({
            "v": 42,
            "lock": {"state": "unlocked", "remember_device_available": true, "remember_device_enabled": false},
            "settings": {"appearance.ui_theme": "auto", "security.auto_lock_minutes": 15},
            "notifications": {"items": [], "unread": 0},
            "vaults": [{"vault_id": VAULT, "kind": "personal", "status": "active", "name": "Personal", "color": "graphite"}],
            "window_label": "w-1"
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
