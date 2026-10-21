//! Session support in the core (stage 2 plan, section 4.4): effective
//! session configuration, quick-connect validation, prompted passwords, OS
//! detection results, the minimal connection log and Recent connections,
//! and the last session of Restore (`last_session`).

mod common;
mod stage2;

use std::time::Duration;

use common::{Unlocked, first_run, reasons};
use secrecy::{ExposeSecret, SecretString};
use serde_json::{Value, json};
use shellpp_core::os_detect::{self, DetectedOs};
use shellpp_core::service::dto::{
    Backspace, CoreEventKind, KeyImportInput, OsFamily, OsSource, RecordRef, RecordView,
    SecretFieldRef, SessionTarget, SettingsSetInput,
};
use shellpp_core::service::host_api::{ConnectionEndReason, ConnectionLogStart, SessionConfig};
use shellpp_core::{CoreError, ValidationReason};
use stage2::{KeyKind, generate, openssh};
use uuid::Uuid;

fn host_target(u: &Unlocked, record_id: Uuid) -> SessionTarget {
    SessionTarget::Host {
        vault_id: u.vault_id,
        record_id,
    }
}

fn config(u: &Unlocked, record_id: Uuid) -> SessionConfig {
    u.core.session_config(&host_target(u, record_id)).unwrap()
}

fn set(u: &Unlocked, values: Value) {
    u.core
        .settings_set(SettingsSetInput {
            values: values
                .as_object()
                .unwrap()
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
            reset: vec![],
        })
        .unwrap();
}

fn import_key(u: &Unlocked, comment: &str) -> Uuid {
    u.core
        .key_import(KeyImportInput {
            vault_id: u.vault_id,
            label: None,
            private_key: SecretString::from(openssh(&generate(KeyKind::Ed25519, comment), None)),
            passphrase: None,
            save_passphrase: false,
        })
        .unwrap()
        .key
        .record_id
}

#[test]
fn defaults_of_a_minimal_host() {
    let u = first_run();
    let host = u.host(json!({"address": "min.example.com"}));
    let config = config(&u, host.record_id);
    assert_eq!(
        config.host,
        Some(RecordRef {
            vault_id: u.vault_id,
            record_id: host.record_id
        })
    );
    assert_eq!(config.title, "min.example.com");
    assert_eq!(config.address, "min.example.com");
    assert_eq!(config.port, 22);
    assert_eq!(config.username, None);
    assert!(config.password.is_none());
    assert!(config.key.is_none());
    // Session defaults stored without a settings screen (UI 12.4).
    assert_eq!(config.keepalive_s, 30);
    assert_eq!(config.term, "xterm-256color");
    assert!(!config.agent_forwarding);
    assert!(config.env.is_empty());
    assert_eq!(config.charset, "utf-8");
    assert_eq!(config.backspace, Backspace::Default);
    assert_eq!(config.terminal_theme, json!({"builtin": "graphite"}));
    assert!(config.record_history);
    assert_eq!(
        config.os_detection_script.as_deref(),
        Some(os_detect::DEFAULT_SCRIPT)
    );
    assert_eq!(config.os, None);
    assert!(config.known_key_types.is_empty());
    assert!(config.can_save_password);
    let values = u.core.settings_get().unwrap();
    assert_eq!(values["terminal.autoreconnect"], json!(true));
    assert_eq!(values["terminal.keepalive_s"], json!(30));
    assert_eq!(values["terminal.term"], json!("xterm-256color"));
    // The last session is an encrypted record, not a setting (UI 6).
    assert!(!values.contains_key("session.last"));
}

#[test]
fn inheritance_of_port_username_and_key() {
    let u = first_run();
    let group_key = import_key(&u, "group key");
    let own_key = import_key(&u, "own key");
    let top = u.group(
        json!({"name": "top", "ssh": {"port": 2200, "username": "root", "key_id": group_key}}),
    );
    let mid =
        u.group(json!({"name": "mid", "parent_id": top.record_id, "ssh": {"username": "deploy"}}));
    let inherits = u.host(json!({"address": "a.example.com", "group_id": mid.record_id}));
    let config_a = config(&u, inherits.record_id);
    assert_eq!(config_a.port, 2200);
    assert_eq!(config_a.username.as_deref(), Some("deploy"));
    let key = config_a.key.expect("inherited key");
    assert_eq!(key.key.record_id, group_key);
    assert_eq!(key.label, "group key");

    let own = u.host(
        json!({"address": "b.example.com", "group_id": mid.record_id,
        "ssh": {"port": 2022, "username": "alice", "key_id": own_key}}),
    );
    let config_b = config(&u, own.record_id);
    assert_eq!(config_b.port, 2022);
    assert_eq!(config_b.username.as_deref(), Some("alice"));
    assert_eq!(config_b.key.unwrap().key.record_id, own_key);

    // A reference to a missing key counts as set: no fallback to the group's
    // key, and the session has no key (docs/data-model.md, section 8.3).
    let missing = u.host(
        json!({"address": "c.example.com", "group_id": mid.record_id,
        "ssh": {"key_id": Uuid::now_v7()}}),
    );
    let config_c = config(&u, missing.record_id);
    assert!(config_c.key.is_none());
    assert_eq!(config_c.port, 2200);
}

#[test]
fn per_host_values_and_fallbacks() {
    let u = first_run();
    let host = u.host_with_password(
        json!({
            "address": "full.example.com",
            "label": "Full",
            "charset": "Windows-1251",
            "backspace": "ctrl_h",
            "terminal_theme": {"builtin": "paper"},
            "record_history": "off",
            "ssh": {"keepalive_s": 0, "agent_forwarding": true,
                "env": [{"name": "LANG", "value": "C.UTF-8"}, {"name": "TZ", "value": "UTC"}]}
        }),
        "pw-generated-at-run-time",
    );
    let config = config(&u, host.record_id);
    assert_eq!(config.title, "Full");
    assert_eq!(
        config
            .password
            .as_ref()
            .map(|p| p.expose_secret().to_owned()),
        Some("pw-generated-at-run-time".to_owned())
    );
    assert_eq!(config.keepalive_s, 0);
    assert!(config.agent_forwarding);
    assert_eq!(config.charset, "windows-1251");
    assert_eq!(config.backspace, Backspace::CtrlH);
    assert_eq!(config.terminal_theme, json!({"builtin": "paper"}));
    assert!(!config.record_history);
    let env: Vec<(String, String)> = config
        .env
        .iter()
        .map(|(name, value)| (name.clone(), value.expose_secret().to_owned()))
        .collect();
    assert_eq!(
        env,
        vec![
            ("LANG".to_owned(), "C.UTF-8".to_owned()),
            ("TZ".to_owned(), "UTC".to_owned())
        ]
    );
    // Debug output carries no secret.
    assert!(!format!("{config:?}").contains("pw-generated"));

    // Settings change the fallbacks.
    set(
        &u,
        json!({"terminal.keepalive_s": 15, "terminal.term": "vt100",
            "terminal.default_theme": {"builtin": "paper"}, "terminal.record_history": false}),
    );
    let plain = u.host(json!({"address": "plain.example.com", "record_history": "on"}));
    let config = self::config(&u, plain.record_id);
    assert_eq!(config.keepalive_s, 15);
    assert_eq!(config.term, "vt100");
    assert_eq!(config.terminal_theme, json!({"builtin": "paper"}));
    assert!(config.record_history);
    let default = u.host(json!({"address": "default.example.com", "record_history": "default"}));
    assert!(!self::config(&u, default.record_id).record_history);

    // A theme record that does not exist falls back like a missing record.
    let themed = u.host(json!({"address": "t.example.com",
        "terminal_theme": {"record_id": Uuid::now_v7()}}));
    assert_eq!(
        self::config(&u, themed.record_id).terminal_theme,
        json!({"builtin": "paper"})
    );
    set(
        &u,
        json!({"terminal.default_theme": {"record_id": Uuid::now_v7()}}),
    );
    assert_eq!(
        self::config(&u, themed.record_id).terminal_theme,
        json!({"builtin": "graphite"})
    );
}

#[test]
fn os_detection_gating() {
    let u = first_run();
    let host = u.host(json!({"address": "os.example.com"}));
    let host_ref = RecordRef {
        vault_id: u.vault_id,
        record_id: host.record_id,
    };
    assert!(config(&u, host.record_id).os_detection_script.is_some());
    set(&u, json!({"terminal.os_detection_script": "uname -s"}));
    assert_eq!(
        config(&u, host.record_id).os_detection_script.as_deref(),
        Some("uname -s")
    );
    set(&u, json!({"terminal.os_detection_script": "   "}));
    assert_eq!(
        config(&u, host.record_id).os_detection_script.as_deref(),
        Some(os_detect::DEFAULT_SCRIPT)
    );
    set(&u, json!({"terminal.os_detection_enabled": false}));
    assert!(config(&u, host.record_id).os_detection_script.is_none());
    set(&u, json!({"terminal.os_detection_enabled": true}));

    // A detected OS turns detection off for the host.
    let detected = os_detect::from_banner("SSH-2.0-OpenSSH_9.6p1 Ubuntu-3ubuntu13").unwrap();
    u.env.take_events();
    assert!(
        u.core
            .host_os_detected(host_ref, detected, OsSource::Banner)
            .unwrap()
    );
    assert!(u.env.take_events().iter().any(|event| matches!(
        &event.kind,
        CoreEventKind::RecordsChanged { classes, .. } if classes == &vec![2]
    )));
    let config = config(&u, host.record_id);
    assert!(config.os_detection_script.is_none());
    let os = config.os.expect("os");
    assert_eq!(os.family, OsFamily::Linux);
    assert_eq!(os.distro.as_deref(), Some("ubuntu"));
    assert_eq!(os.source, OsSource::Banner);
    // The host view shows it; the UI cannot overwrite it with record_save.
    let RecordView::Host(view) = u.core.record_get(host_ref).unwrap() else {
        panic!("host expected")
    };
    assert_eq!(view.data["os"]["family"], json!("linux"));

    // The same family and distro: not written again (a new version too).
    let again = DetectedOs {
        family: OsFamily::Linux,
        distro: Some("ubuntu".into()),
        version: Some("24.04".into()),
    };
    assert!(
        !u.core
            .host_os_detected(host_ref, again, OsSource::Script)
            .unwrap()
    );
    // Another distro: written; untrusted values are constrained.
    let other = DetectedOs {
        family: OsFamily::Linux,
        distro: Some("Debian".into()),
        version: Some("bad\u{1b}[31m".into()),
    };
    assert!(
        u.core
            .host_os_detected(host_ref, other, OsSource::Script)
            .unwrap()
    );
    let os = self::config(&u, host.record_id).os.unwrap();
    assert_eq!(os.distro.as_deref(), Some("debian"));
    assert_eq!(os.version, None);
    assert_eq!(os.source, OsSource::Script);
    let windows = os_detect::from_script_output("MINGW64_NT-10.0\n").unwrap();
    assert!(
        u.core
            .host_os_detected(host_ref, windows, OsSource::Script)
            .unwrap()
    );
    assert!(matches!(
        u.core
            .host_os_detected(
                RecordRef {
                    vault_id: u.vault_id,
                    record_id: Uuid::now_v7()
                },
                os_detect::from_script_output("Linux\n").unwrap(),
                OsSource::Script
            )
            .unwrap_err(),
        CoreError::NotFound
    ));
}

#[test]
fn quick_connect_targets() {
    let u = first_run();
    let quick = |address: &str, port: Option<u16>, username: Option<&str>| {
        u.core.session_config(&SessionTarget::Quick {
            address: address.to_owned(),
            port,
            username: username.map(str::to_owned),
        })
    };
    let config = quick(" [2001:db8::7] ", Some(2222), Some(" alice ")).unwrap();
    assert_eq!(config.host, None);
    assert_eq!(config.address, "2001:db8::7");
    assert_eq!(config.title, "2001:db8::7");
    assert_eq!(config.port, 2222);
    assert_eq!(config.username.as_deref(), Some("alice"));
    assert!(!config.can_save_password);
    assert!(config.os_detection_script.is_none());
    assert!(config.key.is_none() && config.password.is_none());
    assert_eq!(config.keepalive_s, 30);
    let defaults = quick("192.0.2.5", None, Some("  ")).unwrap();
    assert_eq!(defaults.port, 22);
    assert_eq!(defaults.username, None);

    assert_eq!(
        reasons(quick("", None, None).unwrap_err()),
        vec![("address".into(), ValidationReason::Required)]
    );
    assert_eq!(
        reasons(quick("user@example.com", Some(0), Some("a\u{7}")).unwrap_err()),
        vec![
            ("address".into(), ValidationReason::Invalid),
            ("port".into(), ValidationReason::OutOfRange),
            ("username".into(), ValidationReason::Invalid),
        ]
    );
    assert_eq!(
        reasons(quick("example.com:22", None, Some(&"u".repeat(257))).unwrap_err()),
        vec![
            ("address".into(), ValidationReason::Invalid),
            ("username".into(), ValidationReason::TooLong),
        ]
    );
}

#[test]
fn not_found_and_locked() {
    let u = first_run();
    let host = u.host(json!({"address": "x.example.com"}));
    let group = u.group(json!({"name": "g"}));
    assert!(matches!(
        u.core
            .session_config(&host_target(&u, Uuid::now_v7()))
            .unwrap_err(),
        CoreError::NotFound
    ));
    assert!(matches!(
        u.core
            .session_config(&host_target(&u, group.record_id))
            .unwrap_err(),
        CoreError::NotFound
    ));
    assert!(matches!(
        u.core
            .session_config(&SessionTarget::Host {
                vault_id: Uuid::now_v7(),
                record_id: host.record_id
            })
            .unwrap_err(),
        CoreError::NotFound
    ));
    u.core.lock().unwrap();
    assert!(matches!(
        u.core
            .session_config(&host_target(&u, host.record_id))
            .unwrap_err(),
        CoreError::Locked
    ));
    assert!(matches!(
        u.core.recent_connections().unwrap_err(),
        CoreError::Locked
    ));
}

#[test]
fn prompted_password_is_saved_on_the_host() {
    let u = first_run();
    let host = u.host(json!({"address": "pw.example.com", "ssh": {"username": "alice"}}));
    let host_ref = RecordRef {
        vault_id: u.vault_id,
        record_id: host.record_id,
    };
    let password = format!("pw-{}", common::random_hex());
    u.env.take_events();
    u.core
        .save_prompted_password(host_ref, SecretString::from(password.clone()))
        .unwrap();
    assert!(u.env.take_events().iter().any(|event| matches!(
        &event.kind,
        CoreEventKind::RecordsChanged { classes, .. } if classes == &vec![2]
    )));
    let RecordView::Host(view) = u.core.record_get(host_ref).unwrap() else {
        panic!("host expected")
    };
    assert_eq!(view.secrets_set, vec!["ssh.password".to_owned()]);
    assert_eq!(view.data["ssh"]["username"], json!("alice"));
    assert!(!view.data.to_string().contains(&password));
    let revealed = u
        .core
        .secret_reveal(SecretFieldRef {
            vault_id: u.vault_id,
            record_id: host.record_id,
            path: "ssh.password".into(),
        })
        .unwrap();
    assert_eq!(revealed.expose_secret(), password);
    assert_eq!(
        config(&u, host.record_id)
            .password
            .map(|p| p.expose_secret().to_owned()),
        Some(password)
    );
    assert_eq!(
        reasons(
            u.core
                .save_prompted_password(host_ref, SecretString::from("p".repeat(4097)))
                .unwrap_err()
        ),
        vec![("ssh.password".into(), ValidationReason::TooLong)]
    );
    assert!(matches!(
        u.core
            .save_prompted_password(
                RecordRef {
                    vault_id: u.vault_id,
                    record_id: Uuid::now_v7()
                },
                SecretString::from("x".to_owned())
            )
            .unwrap_err(),
        CoreError::NotFound
    ));
}

fn start(
    u: &Unlocked,
    host: Option<RecordRef>,
    label: Option<&str>,
    address: &str,
    port: u16,
) -> Uuid {
    u.core
        .connection_log_start(ConnectionLogStart {
            host,
            host_label: label.map(str::to_owned),
            address: address.to_owned(),
            port,
            username: Some("deploy".into()),
            os: None,
        })
        .unwrap()
}

#[test]
fn recent_connections_group_and_order() {
    let u = first_run();
    let top = u.group(json!({"name": "Prod"}));
    let web = u.group(json!({"name": "Web", "parent_id": top.record_id}));
    let host =
        u.host(json!({"address": "w1.example.com", "label": "W1", "group_id": web.record_id}));
    let host_link = RecordRef {
        vault_id: u.vault_id,
        record_id: host.record_id,
    };
    let host_ref = Some(host_link);
    assert!(u.core.recent_connections().unwrap().is_empty());
    let first = start(&u, host_ref, Some("W1 old label"), "w1.example.com", 22);
    u.core
        .connection_log_end(first, ConnectionEndReason::Closed, None);
    u.env.mem.clock.advance(Duration::from_secs(5));
    let quick = start(&u, None, None, "203.0.113.4", 2222);
    u.core
        .connection_log_end(quick, ConnectionEndReason::ExitStatus, Some(3));
    u.env.mem.clock.advance(Duration::from_secs(5));
    let second = start(&u, host_ref, Some("W1"), "w1.example.com", 22);
    u.env.mem.clock.advance(Duration::from_secs(5));
    // The same quick address and port again is the same row.
    start(&u, None, None, "203.0.113.4", 2222);
    start(&u, None, None, "203.0.113.4", 22);
    let _ = second;

    let rows = u.core.recent_connections().unwrap();
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0].host, None);
    assert_eq!(rows[0].address, "203.0.113.4");
    assert_eq!(rows[0].port, 22);
    assert_eq!(rows[1].port, 2222);
    assert_eq!(rows[1].title, "203.0.113.4");
    assert!(rows[1].path.is_empty());
    assert_eq!(rows[2].host, host_ref);
    assert_eq!(rows[2].title, "W1");
    assert_eq!(rows[2].path, vec!["Personal", "Prod", "Web"]);
    assert_eq!(rows[2].username.as_deref(), Some("deploy"));
    assert!(rows[0].last_at >= rows[2].last_at);

    // The title and OS follow the host now; a removed host keeps its snapshot.
    common::save(
        &u.core,
        u.vault_id,
        Some(host.record_id),
        shellpp_core::service::dto::RecordType::Host,
        json!({"address": "w1.example.com", "label": "Renamed", "group_id": web.record_id}),
        &[],
    )
    .unwrap();
    u.core
        .host_os_detected(
            host_link,
            os_detect::from_script_output("Linux\nID=debian\n").unwrap(),
            OsSource::Script,
        )
        .unwrap();
    let rows = u.core.recent_connections().unwrap();
    assert_eq!(rows[2].title, "Renamed");
    assert_eq!(
        rows[2].os.as_ref().map(|os| os.family),
        Some(OsFamily::Linux)
    );
    u.core
        .record_delete(shellpp_core::service::dto::RecordsRef {
            vault_id: u.vault_id,
            record_ids: vec![host.record_id],
        })
        .unwrap();
    let rows = u.core.recent_connections().unwrap();
    assert_eq!(rows[2].title, "W1");
    assert!(rows[2].path.is_empty());
    // The removed host cannot be opened: the row connects to its snapshot.
    assert_eq!(rows[2].host, None);
    assert_eq!(rows[2].address, "w1.example.com");
    assert_eq!(rows[2].port, 22);
    assert_eq!(rows[2].username.as_deref(), Some("deploy"));

    // At most ten rows.
    for n in 0..12 {
        start(&u, None, None, &format!("198.51.100.{n}"), 22);
    }
    assert_eq!(u.core.recent_connections().unwrap().len(), 10);
}

#[test]
fn connection_log_events_and_locked_ends() {
    let mut u = first_run();
    u.env.take_events();
    let id = start(&u, None, Some("quick"), "192.0.2.44", 22);
    assert!(u.env.take_events().iter().any(|event| matches!(
        &event.kind,
        CoreEventKind::RecordsChanged { classes, record_ids, .. }
            if classes == &vec![7] && record_ids == &vec![id]
    )));
    // While locked the end is held and written at unlock.
    u.core.lock().unwrap();
    u.core
        .connection_log_end(id, ConnectionEndReason::Dropped, None);
    u.env.take_events();
    u.core.unlock(u.password.clone()).unwrap();
    let events = u.env.take_events();
    assert!(events.iter().any(|event| matches!(
        &event.kind,
        CoreEventKind::RecordsChanged { classes, record_ids, .. }
            if classes == &vec![7] && record_ids == &vec![id]
    )));
    // An unknown id ends silently.
    u.core
        .connection_log_end(Uuid::now_v7(), ConnectionEndReason::Closed, None);
    assert_eq!(u.core.recent_connections().unwrap().len(), 1);
    u.reopen();
    u.core.unlock(u.password.clone()).unwrap();
    assert_eq!(u.core.recent_connections().unwrap().len(), 1);
    u.core.lock().unwrap();
    assert!(matches!(
        u.core
            .connection_log_start(ConnectionLogStart {
                host: None,
                host_label: None,
                address: "192.0.2.1".into(),
                port: 22,
                username: None,
                os: None,
            })
            .unwrap_err(),
        CoreError::Locked
    ));
}

#[test]
fn last_session_round_trip() {
    use shellpp_core::service::last_session::{
        SavedPane, SavedSession, SavedTab, SavedWindow, SplitDirection,
    };
    let u = first_run();
    assert_eq!(u.core.last_session().unwrap(), None);
    let host = u.host(json!({"address": "r.example.com"}));
    let windows = vec![
        SavedWindow {
            tabs: vec![
                SavedTab {
                    title: Some("prod".to_owned()),
                    root: SavedPane::Leaf {
                        session: SavedSession::Host(RecordRef {
                            vault_id: u.vault_id,
                            record_id: host.record_id,
                        }),
                    },
                },
                SavedTab {
                    title: None,
                    root: SavedPane::Split {
                        direction: SplitDirection::Row,
                        sizes: vec![0.3, 0.7],
                        children: vec![
                            SavedPane::Leaf {
                                session: SavedSession::Local { shell: None },
                            },
                            SavedPane::Leaf {
                                session: SavedSession::Quick {
                                    address: "192.0.2.5".to_owned(),
                                    port: Some(2200),
                                    username: Some("admin".to_owned()),
                                },
                            },
                        ],
                    },
                },
            ],
        },
        // An empty window is not kept.
        SavedWindow { tabs: vec![] },
    ];
    u.core.last_session_save(windows.clone()).unwrap();
    let saved = u.core.last_session().unwrap().expect("saved");
    assert_eq!(saved.windows, windows[..1].to_vec());
    assert!(saved.has_tabs());
    // One record, replaced by the next save.
    u.core
        .last_session_save(vec![SavedWindow {
            tabs: windows[0].tabs[1..].to_vec(),
        }])
        .unwrap();
    let saved = u.core.last_session().unwrap().expect("saved");
    assert_eq!(saved.windows[0].tabs.len(), 1);
    // A bad value is refused and the saved one stays.
    let error = u
        .core
        .last_session_save(vec![SavedWindow {
            tabs: vec![SavedTab {
                title: Some("a\u{7}".to_owned()),
                root: windows[0].tabs[0].root.clone(),
            }],
        }])
        .unwrap_err();
    assert_eq!(
        reasons(error),
        vec![("windows.0.tabs.0.title".into(), ValidationReason::Invalid)]
    );
    assert!(u.core.last_session().unwrap().is_some());
    // Nothing open: nothing to restore.
    u.core.last_session_save(vec![]).unwrap();
    assert_eq!(u.core.last_session().unwrap(), None);
    // It needs the keys.
    u.core.lock().unwrap();
    assert!(matches!(
        u.core.last_session_save(windows).unwrap_err(),
        CoreError::Locked
    ));
}

/// The hops of the effective jump chain (own, else inherited; `[]` stops
/// the lookup), each with its own configuration (stage 5).
#[test]
fn the_effective_jump_chain_becomes_the_route() {
    let u = first_run();
    let bastion = u.host(json!({"address": "192.0.2.60"}));
    let chain = json!([bastion.record_id.to_string()]);
    let group = u.group(json!({"name": "behind", "ssh": {"jump_chain": chain}}));
    let own = u.host(json!({"address": "10.0.0.5", "ssh": {"jump_chain": chain}}));
    let inherited = u.host(json!({"address": "10.0.0.6", "group_id": group.record_id.to_string()}));
    let stopped = u.host(json!({
        "address": "10.0.0.7",
        "group_id": group.record_id.to_string(),
        "ssh": {"jump_chain": []}
    }));
    let hops = |id: Uuid| -> Vec<String> {
        let config = config(&u, id);
        assert_eq!(config.route_problem, None);
        config
            .jump_hosts
            .iter()
            .map(|hop| hop.address.clone())
            .collect()
    };
    assert_eq!(hops(own.record_id), ["192.0.2.60"]);
    assert_eq!(hops(inherited.record_id), ["192.0.2.60"]);
    assert!(hops(stopped.record_id).is_empty());
    assert!(hops(bastion.record_id).is_empty());
    // A bastion inside the group whose chain names it is not its own hop.
    let own = u.group(json!({"name": "with its bastion"}));
    let inner = u.host(json!({"address": "192.0.2.61", "group_id": own.record_id.to_string()}));
    let neighbour = u.host(json!({"address": "192.0.2.62", "group_id": own.record_id.to_string()}));
    common::save(
        &u.core,
        u.vault_id,
        Some(own.record_id),
        shellpp_core::service::dto::RecordType::Group,
        json!({"name": "with its bastion", "ssh": {"jump_chain": [inner.record_id.to_string()]}}),
        &[],
    )
    .unwrap();
    assert!(hops(inner.record_id).is_empty());
    assert_eq!(hops(neighbour.record_id), ["192.0.2.61"]);
}

/// Each hop uses its own effective port and credentials and has no route of
/// its own; a missing or repeated hop is named (data-model 7.3, 8.3).
#[test]
fn hops_have_their_own_credentials_and_bad_chains_are_named() {
    use shellpp_core::service::host_api::RouteProblem;
    let u = first_run();
    let group = u.group(json!({"name": "dmz", "ssh": {"port": 2200, "username": "ops"}}));
    let first =
        u.host(json!({"address": "bastion.example", "group_id": group.record_id.to_string()}));
    let second = common::save(
        &u.core,
        u.vault_id,
        None,
        shellpp_core::service::dto::RecordType::Host,
        json!({"address": "inner.example", "ssh": {"port": 2222, "username": "jump",
            "jump_chain": [first.record_id.to_string()]}}),
        &[("ssh.password", Some("hop-secret"))],
    )
    .unwrap();
    let target = u.host(json!({"address": "app.example", "ssh": {"username": "app",
        "env": [{"name": "LANG", "value": "C"}],
        "jump_chain": [first.record_id.to_string(), second.record_id().to_string()]}}));
    let route = config(&u, target.record_id);
    assert_eq!(route.route_problem, None);
    assert_eq!(route.jump_hosts.len(), 2);
    let (hop1, hop2) = (&route.jump_hosts[0], &route.jump_hosts[1]);
    // The first hop inherits from its group; the second has its own values.
    assert_eq!(
        (hop1.address.as_str(), hop1.port),
        ("bastion.example", 2200)
    );
    assert_eq!(hop1.username.as_deref(), Some("ops"));
    assert_eq!((hop2.address.as_str(), hop2.port), ("inner.example", 2222));
    assert_eq!(hop2.username.as_deref(), Some("jump"));
    assert_eq!(
        hop2.password
            .as_ref()
            .map(|p| p.expose_secret().to_owned())
            .as_deref(),
        Some("hop-secret")
    );
    // A hop's own chain and proxy are ignored; it runs no session.
    assert!(hop2.jump_hosts.is_empty() && hop2.proxy.is_none());
    assert!(hop2.env.is_empty() && hop2.os_detection_script.is_none());
    assert_eq!(route.env.len(), 1);

    // A hop that is gone.
    let gone = u.host(json!({"address": "gone.example"}));
    let behind_gone = u.host(json!({"address": "x.example",
        "ssh": {"jump_chain": [first.record_id.to_string(), gone.record_id.to_string()]}}));
    u.core
        .record_delete(shellpp_core::service::dto::RecordsRef {
            vault_id: u.vault_id,
            record_ids: vec![gone.record_id],
        })
        .unwrap();
    let route = config(&u, behind_gone.record_id);
    assert_eq!(
        route.route_problem,
        Some(RouteProblem::MissingHop { index: 1 })
    );
    assert!(route.jump_hosts.is_empty());
    // A hop named twice (a loop) is refused when it is saved; the route
    // check refuses it too, for data that bypassed the model.
    let looped = common::save(
        &u.core,
        u.vault_id,
        None,
        shellpp_core::service::dto::RecordType::Host,
        json!({"address": "y.example",
            "ssh": {"jump_chain": [first.record_id.to_string(), second.record_id().to_string(),
                first.record_id.to_string()]}}),
        &[],
    );
    assert!(
        matches!(looped, Err(CoreError::Validation(ref errors))
            if errors.iter().any(|error| error.field == "ssh.jump_chain")),
        "{looped:?}"
    );
}

/// `host.ssh.proxy` with its password reaches the session; groups do not
/// pass it on (data-model 8.1).
#[test]
fn the_proxy_is_the_hosts_own() {
    use shellpp_core::service::host_api::ProxyType;
    let u = first_run();
    let host = common::save(
        &u.core,
        u.vault_id,
        None,
        shellpp_core::service::dto::RecordType::Host,
        json!({"address": "behind-proxy.example", "ssh": {"proxy": {"type": "socks5",
            "address": "proxy.example", "port": 1080, "username": "puser"}}}),
        &[("ssh.proxy.password", Some("proxy-secret"))],
    )
    .unwrap();
    let route = config(&u, host.record_id());
    let proxy = route.proxy.expect("proxy");
    assert_eq!(proxy.kind, ProxyType::Socks5);
    assert_eq!(
        (proxy.address.as_str(), proxy.port),
        ("proxy.example", 1080)
    );
    assert_eq!(proxy.username.as_deref(), Some("puser"));
    assert_eq!(
        proxy
            .password
            .as_ref()
            .map(|p| p.expose_secret().to_owned())
            .as_deref(),
        Some("proxy-secret")
    );
    assert!(!format!("{proxy:?}").contains("proxy-secret"));
    let plain = u.host(json!({"address": "direct.example"}));
    assert!(config(&u, plain.record_id).proxy.is_none());
}

/// The Logs screen's list (UI 11.1): every entry newest first with its end,
/// "now" while active, and the bookmark.
#[test]
fn the_connection_log_lists_every_entry_and_keeps_the_bookmark() {
    let u = first_run();
    let host = u.host(json!({"address": "w1.example.com", "label": "W1"}));
    let host_ref = Some(RecordRef {
        vault_id: u.vault_id,
        record_id: host.record_id,
    });
    let first = start(&u, host_ref, Some("W1"), "w1.example.com", 22);
    u.core
        .connection_log_end(first, ConnectionEndReason::ExitStatus, Some(2));
    u.env.mem.clock.advance(Duration::from_secs(5));
    let active = start(&u, None, None, "203.0.113.4", 2222);
    let rows = u.core.connection_log_list().unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].record_id, active);
    assert!(rows[0].is_active() && rows[0].this_device);
    assert_eq!(rows[0].title, "203.0.113.4");
    assert_eq!(rows[1].record_id, first);
    assert_eq!(rows[1].title, "W1");
    assert_eq!(rows[1].end_reason.as_deref(), Some("exit_status"));
    assert_eq!(rows[1].exit_status, Some(2));
    assert!(rows[1].ended_at.is_some() && !rows[1].is_active());
    assert!(rows.iter().all(|row| !row.saved));
    u.core.connection_log_set_saved(first, true).unwrap();
    let rows = u.core.connection_log_list().unwrap();
    assert!(rows[1].saved && !rows[0].saved);
    u.core.connection_log_set_saved(first, false).unwrap();
    assert!(!u.core.connection_log_list().unwrap()[1].saved);
}
