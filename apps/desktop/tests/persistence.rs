//! Owner's stage 1, steps 3.3 and 5.1: everything the Hosts screen and the
//! host panel save is written to the encrypted store at once and is there
//! after a restart: groups (nested, with their fields), hosts with every
//! field of the host panel (UI 3.1, including "Show more", the Telnet block,
//! linked credentials and both secrets), the manual order, the parent group
//! after Move to, Copy to and Duplicate.
//!
//! A restart is a real one: the app state shuts down (the core closes its
//! files), then a new core opens the same temporary profile folder and is
//! unlocked with the master password. Headless GPUI, in-memory secret store.

mod common;

use common::{Ui, map};
use gpui_kit::TestAppContext;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::{Value, json};
use shellpp_core::service::dto::{MovePosition, TagColor};
use shellpp_desktop::state::records_edit::{
    BackspaceMode, EnvVar, HostDraft, HostSecrets, OverrideMode, ProxyKind, SecretChange,
    TelnetDraft,
};
use shellpp_desktop::state::{GroupItem, HostItem};
use shellpp_desktop::testing::TestProfile;
use uuid::Uuid;

/// What a restart must keep: every host and group with its id, data, place
/// and the names of its stored secrets, in manual order.
#[derive(Debug, PartialEq)]
struct Snapshot {
    hosts: Vec<(Uuid, Option<Uuid>, Value, Vec<String>)>,
    groups: Vec<(Uuid, Option<Uuid>, Value)>,
    top_level_order: Vec<Uuid>,
}

fn snapshot(ui: &Ui, cx: &mut TestAppContext) -> Snapshot {
    cx.update(|cx| {
        let state = ui.state.read(cx);
        let mut hosts: Vec<_> = state
            .hosts()
            .iter()
            .map(|host: &HostItem| {
                let mut secrets = host.secrets_set.clone();
                secrets.sort();
                (host.record_id, host.group_id, host.data.clone(), secrets)
            })
            .collect();
        hosts.sort_by_key(|host| host.0);
        let mut groups: Vec<_> = state
            .groups()
            .iter()
            .map(|group: &GroupItem| (group.record_id, group.parent_id, group.data.clone()))
            .collect();
        groups.sort_by_key(|group| group.0);
        Snapshot {
            hosts,
            groups,
            top_level_order: state
                .hosts_in(None)
                .iter()
                .map(|host| host.record_id)
                .collect(),
        }
    })
}

fn random_text(prefix: &str) -> String {
    format!("{prefix}-{}", Uuid::now_v7().simple())
}

/// Every field of the host panel with a non-default value.
fn full_draft(group: Uuid, bastion: Uuid, other_jump: Uuid) -> HostDraft {
    HostDraft {
        address: "db1.example.com".into(),
        label: "Primary database".into(),
        group_id: Some(group),
        tags: vec!["prod".into(), "eu".into()],
        color: TagColor::Ocean,
        backspace: BackspaceMode::CtrlH,
        notes: "Line one\nLine two".into(),
        port: "2222".into(),
        username: "dba".into(),
        identity_id: Some(Uuid::now_v7()),
        key_id: Some(Uuid::now_v7()),
        certificate_key_id: Some(Uuid::now_v7()),
        fido2_key_id: Some(Uuid::now_v7()),
        agent_forwarding: true,
        startup_snippet_id: Some(Uuid::now_v7()),
        jump_chain: Some(vec![bastion, other_jump]),
        proxy_kind: ProxyKind::Socks5,
        proxy_address: "proxy.example.net".into(),
        proxy_port: "1080".into(),
        proxy_username: "proxyuser".into(),
        env: vec![
            EnvVar {
                name: "LANG".into(),
                value: "en_US.UTF-8".into(),
            },
            EnvVar {
                name: "EDITOR".into(),
                value: "vim".into(),
            },
        ],
        charset: "koi8-r".into(),
        keepalive: "45".into(),
        record_history: OverrideMode::On,
        record_output: OverrideMode::Off,
        mosh: true,
        terminal_theme: Some(
            shellpp_desktop::state::records_edit::TERMINAL_THEMES
                .last()
                .expect("a built-in theme")
                .to_string(),
        ),
        telnet: Some(TelnetDraft {
            port: "2323".into(),
            username: "operator".into(),
        }),
    }
}

/// Owner's step 5.1: hosts, groups, every host panel field (Telnet and
/// credential references included), secrets, the manual order and the
/// parent group are the same after a restart.
#[gpui_kit::test]
fn everything_saved_survives_a_restart(cx: &mut TestAppContext) {
    let mut profile = TestProfile::new();
    let password = random_text("host-password");
    let proxy_password = random_text("proxy-password");

    let ui = Ui::launch(cx, &mut profile);
    let (production, databases, staging) = ui.state.update(cx, |state, cx| {
        let production = state
            .save_group(
                None,
                map(json!({"name": "Production", "color": "jade"})),
                cx,
            )
            .expect("group")
            .record_id;
        let databases = state
            .save_group(
                None,
                map(json!({
                    "name": "Databases",
                    "parent_id": production,
                    "color": "plum",
                    "ssh": {"username": "postgres", "port": 2200}
                })),
                cx,
            )
            .expect("nested group")
            .record_id;
        let staging = state
            .save_group(None, map(json!({"name": "Staging"})), cx)
            .expect("group")
            .record_id;
        (production, databases, staging)
    });

    // Plain hosts at the top level, created in this order.
    let [a, b, c, bastion] = ["a", "b", "c", "bastion"].map(|name| {
        ui.state.update(cx, |state, cx| {
            state
                .save_host(
                    None,
                    map(json!({"address": format!("{name}.example.com"), "label": name})),
                    cx,
                )
                .expect("host")
                .record_id
        })
    });

    // The full host through the host panel's save path.
    let draft = full_draft(databases, bastion, a);
    let secrets = HostSecrets {
        password: SecretChange::Set(SecretString::from(password.clone())),
        proxy_password: SecretChange::Set(SecretString::from(proxy_password.clone())),
    };
    let full = ui.state.update(cx, |state, cx| {
        let report = state.save_host_draft(None, &draft, &secrets, cx);
        assert!(report.errors.is_empty(), "{:?}", report.errors);
        report.saved.expect("the host was created").record_id
    });

    // Manual order: C before A. Move to: B into Staging. Duplicate and
    // Copy to: copies of A (top level) and of A into Production.
    ui.state.update(cx, |state, cx| {
        state
            .move_records_to(vec![c], None, MovePosition::Before { record_id: a }, cx)
            .expect("reorder");
        state
            .move_records(vec![b], Some(staging), cx)
            .expect("move");
        state.duplicate_record(a, cx).expect("duplicate");
        state
            .copy_hosts_to(vec![a], Some(production), cx)
            .expect("copy to");
    });

    let before = snapshot(&ui, cx);
    // The order and places as set above.
    let top: Vec<Uuid> = before
        .top_level_order
        .iter()
        .copied()
        .filter(|id| [a, c, bastion].contains(id))
        .collect();
    assert_eq!(top, [c, a, bastion], "manual order at the top level");
    ui.close(cx);

    // -- Restart ----------------------------------------------------------------
    let ui = Ui::launch(cx, &mut profile);
    let after = snapshot(&ui, cx);
    assert_eq!(after, before, "the same records after a restart");

    let host = ui.host(cx, full).expect("the full host");
    assert_eq!(host.group_id, Some(databases), "parent group");
    let data = &host.data;
    assert_eq!(data["address"], "db1.example.com");
    assert_eq!(data["label"], "Primary database");
    assert_eq!(data["tags"], json!(["prod", "eu"]));
    assert_eq!(data["color"], "ocean");
    assert_eq!(data["notes"], "Line one\nLine two");
    assert_eq!(data["charset"], "koi8-r");
    assert_eq!(data["record_history"], "on");
    assert_eq!(data["record_output"], "off");
    let ssh = &data["ssh"];
    assert_eq!(ssh["port"], 2222);
    assert_eq!(ssh["username"], "dba");
    for member in [
        "identity_id",
        "key_id",
        "certificate_key_id",
        "fido2_key_id",
        "startup_snippet_id",
    ] {
        assert!(ssh[member].is_string(), "ssh.{member} is kept");
    }
    assert_eq!(ssh["jump_chain"], json!([bastion, a]));
    assert_eq!(ssh["agent_forwarding"], true);
    assert_eq!(ssh["mosh"], true);
    assert_eq!(ssh["keepalive_s"], 45);
    assert_eq!(ssh["proxy"]["type"], "socks5");
    assert_eq!(ssh["proxy"]["address"], "proxy.example.net");
    assert_eq!(ssh["proxy"]["port"], 1080);
    assert_eq!(ssh["proxy"]["username"], "proxyuser");
    assert_eq!(ssh["env"].as_array().map(Vec::len), Some(2));
    assert_eq!(data["telnet"]["port"], 2323);
    assert_eq!(data["telnet"]["username"], "operator");
    // The panel reads back what it saved.
    let reread = HostDraft::from_host(&host);
    assert_eq!(reread.identity_id, draft.identity_id);
    assert_eq!(reread.key_id, draft.key_id);
    assert_eq!(reread.certificate_key_id, draft.certificate_key_id);
    assert_eq!(reread.fido2_key_id, draft.fido2_key_id);
    assert_eq!(reread.startup_snippet_id, draft.startup_snippet_id);
    assert_eq!(reread.telnet, draft.telnet);
    assert_eq!(reread.env, draft.env);
    assert_eq!(reread.terminal_theme, draft.terminal_theme);
    assert_eq!(reread.backspace, BackspaceMode::CtrlH);
    assert!(reread.mosh && reread.agent_forwarding);
    // Secrets: both stored, readable only through the core.
    for (path, expected) in [
        ("ssh.password", &password),
        ("ssh.proxy.password", &proxy_password),
    ] {
        let revealed = cx
            .update(|cx| ui.state.read(cx).reveal_secret(full, path))
            .expect("the secret");
        assert_eq!(revealed.expose_secret(), expected, "{path}");
    }

    // Groups: nesting and fields.
    let databases_group = cx
        .update(|cx| ui.state.read(cx).group(databases).cloned())
        .expect("nested group");
    assert_eq!(databases_group.parent_id, Some(production));
    assert_eq!(databases_group.data["ssh"]["username"], "postgres");
    // Move to and Copy to.
    assert_eq!(ui.host(cx, b).unwrap().group_id, Some(staging));
    let copies: Vec<HostItem> = ui
        .hosts(cx)
        .into_iter()
        .filter(|host| host.address == "a.example.com" && host.record_id != a)
        .collect();
    assert_eq!(copies.len(), 2, "Duplicate and Copy to");
    assert!(
        copies
            .iter()
            .any(|copy| copy.label.as_deref() == Some("a Copy") && copy.group_id.is_none()),
        "Duplicate: label + \" Copy\" at the top level"
    );
    assert!(
        copies
            .iter()
            .any(|copy| copy.label.as_deref() == Some("a") && copy.group_id == Some(production)),
        "Copy to: the same label in the target group"
    );
    ui.close(cx);

    let path = profile.path().to_path_buf();
    drop(profile);
    assert!(!path.exists(), "the test profile is removed");
}

/// Edits after a restart keep working on the same records (no duplicate on
/// save), and a second restart shows them.
#[gpui_kit::test]
fn edits_after_a_restart_are_saved_too(cx: &mut TestAppContext) {
    let mut profile = TestProfile::new();
    let ui = Ui::launch(cx, &mut profile);
    let host = ui.state.update(cx, |state, cx| {
        state
            .save_host(None, map(json!({"address": "web1.example.com"})), cx)
            .expect("host")
            .record_id
    });
    ui.close(cx);

    let ui = Ui::launch(cx, &mut profile);
    let item = ui.host(cx, host).expect("the host after a restart");
    let mut data = item.data_map();
    data.insert("label".into(), json!("Web"));
    ui.state.update(cx, |state, cx| {
        state.save_host(Some(host), data, cx).expect("edit");
    });
    ui.close(cx);

    let ui = Ui::launch(cx, &mut profile);
    let hosts = ui.hosts(cx);
    assert_eq!(hosts.len(), 1);
    assert_eq!(hosts[0].record_id, host);
    assert_eq!(hosts[0].label.as_deref(), Some("Web"));
    ui.close(cx);
}
