//! Import of PuTTY saved sessions (SPEC "Import"; UI 2.5; run 8 decision
//! D140) over a fake session source (the registry is read only by the
//! desktop app on Windows): SSH and Telnet sessions, a key file (an
//! OpenSSH key generated at run time: `.ppk` conversion has its own tests
//! against puttygen), a SOCKS 5 proxy with its password, tunnels, a
//! character set, sessions that are not imported, and a second import that
//! adds nothing.

mod common;
mod stage2;

use std::collections::BTreeMap;
use std::path::Path;

use common::{as_host, first_run};
use secrecy::ExposeSecret;
use serde_json::{Value, json};
use shellpp_core::import::putty::{FixedSessions, Session, SessionSource, Sessions, Value as Reg};
use shellpp_core::service::dto::{
    HostView, ImportStatus, KeyFileStatus, PuttyImportInput, QueryScope, RecordType, RecordsQuery,
    SortMode,
};

fn text(value: &str) -> Reg {
    Reg::Text(value.to_owned())
}

fn session(key: &str, values: &[(&str, Reg)]) -> Session {
    Session {
        key: key.to_owned(),
        values: values
            .iter()
            .map(|(name, value)| ((*name).to_owned(), value.clone()))
            .collect(),
    }
}

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

/// The fake registry of the test: a template, five sessions to import and
/// four that are not imported, one key that cannot be read.
fn sessions(key_file: &Path, proxy_password: &str) -> FixedSessions {
    FixedSessions(Sessions {
        sessions: vec![
            session(
                "Default%20Settings",
                &[("HostName", text("")), ("Protocol", text("ssh"))],
            ),
            session(
                "web%20prod",
                &[
                    // As in PuTTY, the user in the host name wins.
                    ("HostName", text("deploy@web.example.com")),
                    ("PortNumber", Reg::Number(2222)),
                    ("Protocol", text("ssh")),
                    ("UserName", text("root")),
                    ("PublicKeyFile", text(&key_file.display().to_string())),
                    ("AgentFwd", Reg::Number(1)),
                    // 90 s, as PuTTY writes it.
                    ("PingInterval", Reg::Number(1)),
                    ("PingIntervalSecs", Reg::Number(30)),
                    ("Environment", text("LANG=C.UTF-8,TZ=UTC")),
                    (
                        "PortForwardings",
                        text(
                            "L8080=localhost:80,R2222=10.0.0.5:22,D1080=,Q1=x,Lmybox.lan:7000=db:1",
                        ),
                    ),
                    ("LineCodePage", text("Win1251 (Cyrillic)")),
                    ("X11Forward", Reg::Number(1)),
                    ("ProxyMethod", Reg::Number(2)),
                    ("ProxyHost", text("proxy.example.com")),
                    ("ProxyPort", Reg::Number(1080)),
                    ("ProxyUsername", text("puser")),
                    ("ProxyPassword", text(proxy_password)),
                ],
            ),
            session(
                "router",
                &[
                    ("HostName", text("admin@192.0.2.1")),
                    ("PortNumber", Reg::Number(2323)),
                    ("Protocol", text("telnet")),
                    ("BackspaceIsDelete", Reg::Number(0)),
                ],
            ),
            session(
                "open",
                &[
                    ("HostName", text("[2001:db8::7]")),
                    ("Protocol", text("ssh")),
                    ("PortForwardings", text("L9000=db:5432")),
                    ("LocalPortAcceptAll", Reg::Number(1)),
                    ("LineCodePage", text("Use font encoding")),
                ],
            ),
            session(
                "legacy-http",
                &[
                    ("HostName", text("legacy.example.com")),
                    ("ProxyType", Reg::Number(1)),
                    ("ProxyHost", text("10.0.0.1")),
                    ("ProxyPort", Reg::Number(3128)),
                ],
            ),
            session(
                "cmd-proxy",
                &[
                    ("HostName", text("hidden.example.com")),
                    ("ProxyMethod", Reg::Number(5)),
                    ("ProxyTelnetCommand", text("nc %host %port")),
                ],
            ),
            session(
                "serial",
                &[("Protocol", text("serial")), ("SerialLine", text("COM1"))],
            ),
            session("empty", &[("Protocol", text("ssh"))]),
            // PuTTY never proxies a local address (ProxyLocalhost 0); the
            // `:2222` after the name is dropped, as PuTTY drops it.
            session(
                "local-vm",
                &[
                    ("HostName", text("127.0.0.1:2222")),
                    ("PortNumber", Reg::Number(2222)),
                    ("ProxyMethod", Reg::Number(3)),
                    ("ProxyHost", text("10.0.0.1")),
                    ("ProxyPort", Reg::Number(3128)),
                ],
            ),
            // Telnet hosts connect without a proxy here: refused.
            session(
                "telnet-proxy",
                &[
                    ("HostName", text("192.0.2.2")),
                    ("Protocol", text("telnet")),
                    ("ProxyMethod", Reg::Number(2)),
                    ("ProxyHost", text("10.0.0.1")),
                    ("ProxyPort", Reg::Number(1080)),
                ],
            ),
            session(
                "bad%20address",
                &[("HostName", text("two words")), ("Protocol", text("ssh"))],
            ),
        ],
        unreadable: vec!["broken%21".to_owned()],
        code_page: None,
    })
}

fn hosts(u: &common::Unlocked) -> Vec<HostView> {
    u.core
        .records_query(RecordsQuery {
            vault_id: None,
            types: vec![RecordType::Host],
            scope: QueryScope::All,
            text: None,
            tags: Vec::new(),
            sort: SortMode::Manual,
        })
        .unwrap()
        .into_iter()
        .map(as_host)
        .collect()
}

#[test]
fn sessions_keys_proxies_and_tunnels_are_imported_once() {
    let home = tempfile::tempdir().unwrap();
    let key_path = home.path().join("keys").join("web.ppk");
    let key = stage2::generate(stage2::KeyKind::Ed25519, "web");
    let pass = stage2::passphrase();
    write(
        &key_path,
        &stage2::openssh(&key, Some(pass.expose_secret())),
    );
    // A proxy password made at run time (never a fixed secret).
    let proxy_password = common::random_hex();
    let source = sessions(&key_path, &proxy_password);

    let u = first_run();
    let preview = u.core.putty_preview(&source).expect("preview");
    let names: Vec<&str> = preview
        .hosts
        .iter()
        .map(|host| host.alias.as_str())
        .collect();
    // Default Settings is a template, never a host.
    assert_eq!(
        names,
        [
            "web prod",
            "router",
            "open",
            "legacy-http",
            "cmd-proxy",
            "serial",
            "empty",
            "local-vm",
            "telnet-proxy",
            "bad address"
        ]
    );
    assert_eq!(preview.problems, ["broken!: the session could not be read"]);
    let host = |name: &str| {
        preview
            .hosts
            .iter()
            .find(|host| host.alias == name)
            .unwrap()
            .clone()
    };
    assert_eq!(host("web prod").address, "web.example.com");
    assert_eq!(host("web prod").port, Some(2222));
    assert_eq!(host("web prod").username.as_deref(), Some("deploy"));
    let notes = host("web prod").notes.join("\n");
    assert!(notes.contains("X11 forwarding"), "{notes}");
    assert!(notes.contains("Q1=x"), "{notes}");
    assert!(
        notes.contains("Port forwarding Lmybox.lan:7000=db:1 is not imported"),
        "{notes}"
    );
    assert_eq!(host("local-vm").address, "127.0.0.1");
    assert_eq!(host("local-vm").status, ImportStatus::New);
    let notes = host("local-vm").notes.join("\n");
    assert!(notes.contains("without its proxy"), "{notes}");
    assert_eq!(host("router").address, "192.0.2.1");
    assert_eq!(host("router").username.as_deref(), Some("admin"));
    assert_eq!(host("open").address, "2001:db8::7");
    let notes = host("open").notes.join("\n");
    assert!(notes.contains("listens on all interfaces"), "{notes}");
    assert!(notes.contains("Use font encoding"), "{notes}");
    for name in ["web prod", "router", "open", "legacy-http"] {
        assert_eq!(host(name).status, ImportStatus::New, "{name}");
    }
    for (name, reason) in [
        (
            "cmd-proxy",
            "the proxy is a local command, which is not run or imported",
        ),
        ("serial", "the serial protocol is not supported"),
        ("empty", "the session has no host name"),
        (
            "telnet-proxy",
            "a Telnet session through a proxy is not supported",
        ),
        ("bad address", "invalid address"),
    ] {
        assert_eq!(host(name).status, ImportStatus::Error, "{name}");
        assert_eq!(host(name).reason.as_deref(), Some(reason), "{name}");
    }
    assert_eq!(preview.keys.len(), 1);
    assert_eq!(preview.keys[0].status, KeyFileStatus::NeedsPassphrase);
    assert_eq!(host("web prod").key_files, [preview.keys[0].path.clone()]);

    // Import all but "legacy-http", with the key's passphrase.
    let input = PuttyImportInput {
        sessions: Some(
            ["web prod", "router", "open", "cmd-proxy", "serial"]
                .map(str::to_owned)
                .to_vec(),
        ),
        passphrases: BTreeMap::from([(preview.keys[0].path.clone(), pass.clone())]),
        save_passphrases: false,
    };
    let summary = u.core.putty_import(&source, input.clone()).expect("import");
    assert_eq!(summary.added, 3);
    assert_eq!(summary.skipped, 0);
    assert_eq!(summary.errors.len(), 2, "{:?}", summary.errors);
    assert_eq!(summary.keys_added, 1);
    assert_eq!(summary.commands_skipped, 1, "the proxy command");
    assert_eq!(summary.rules_added, 4, "three of web prod, one of open");

    let hosts = hosts(&u);
    assert_eq!(hosts.len(), 3);
    let data = |label: &str| -> Value {
        hosts
            .iter()
            .find(|host| host.data["label"] == label)
            .unwrap()
            .data
            .clone()
    };
    let web = data("web prod");
    assert_eq!(web["import_key"], "putty:web prod");
    assert_eq!(web["charset"], "windows-1251");
    assert_eq!(web["ssh"]["port"], 2222);
    assert_eq!(web["ssh"]["agent_forwarding"], true);
    assert_eq!(web["ssh"]["agent_forward_system"], true);
    assert_eq!(web["ssh"]["keepalive_s"], 90);
    assert_eq!(
        web["ssh"]["env"],
        json!([{"name": "LANG", "value": "C.UTF-8"}, {"name": "TZ", "value": "UTC"}])
    );
    assert_eq!(web["ssh"]["proxy"]["type"], "socks5");
    assert_eq!(web["ssh"]["proxy"]["address"], "proxy.example.com");
    assert_eq!(web["ssh"]["proxy"]["port"], 1080);
    assert_eq!(web["ssh"]["proxy"]["username"], "puser");
    let key_id = u.core.keys_list().unwrap()[0].record_id;
    assert_eq!(web["ssh"]["key_id"], json!(key_id.to_string()));
    assert!(
        web["notes"]
            .as_str()
            .unwrap()
            .starts_with("Imported from PuTTY.")
    );
    // The proxy password is a secret of the host: stored, never in `data`.
    assert!(!web.to_string().contains(&proxy_password));
    let export = u.core.data_export(Default::default()).unwrap();
    assert!(export.text.expose_secret().contains(&proxy_password));

    let router = data("router");
    assert_eq!(router["primary_protocol"], "telnet");
    assert_eq!(router["telnet"]["port"], 2323);
    assert_eq!(router["telnet"]["username"], "admin");
    assert_eq!(router["backspace"], "ctrl_h");

    // The tunnels: autostart except the one that listens on every interface.
    use shellpp_core::service::snippets::ForwardType;
    let rules = u.core.forwarding_rules_list(u.vault_id).unwrap();
    type Seen = (ForwardType, String, u16, Option<String>, Option<u16>, bool);
    let mut seen: Vec<Seen> = rules
        .iter()
        .map(|rule| {
            (
                rule.kind,
                rule.bind_address.clone(),
                rule.bind_port,
                rule.dest_address.clone(),
                rule.dest_port,
                rule.autostart_on_connect,
            )
        })
        .collect();
    seen.sort_by_key(|rule| rule.2);
    assert_eq!(
        seen,
        [
            (
                ForwardType::Dynamic,
                "127.0.0.1".into(),
                1080,
                None,
                None,
                true
            ),
            (
                ForwardType::Remote,
                "127.0.0.1".into(),
                2222,
                Some("10.0.0.5".into()),
                Some(22),
                true
            ),
            (
                ForwardType::Local,
                "127.0.0.1".into(),
                8080,
                Some("localhost".into()),
                Some(80),
                true
            ),
            // "Local ports accept connections from other hosts".
            (
                ForwardType::Local,
                "0.0.0.0".into(),
                9000,
                Some("db".into()),
                Some(5432),
                false
            ),
        ]
    );

    // A second import adds nothing and changes nothing.
    let again = u.core.putty_import(&source, input).expect("again");
    assert_eq!(again.added, 0);
    assert_eq!(again.skipped, 3);
    assert_eq!(hosts_count(&u), 3);
    let preview = u.core.putty_preview(&source).unwrap();
    for name in ["web prod", "router", "open"] {
        let status = preview
            .hosts
            .iter()
            .find(|host| host.alias == name)
            .unwrap()
            .status;
        assert_eq!(status, ImportStatus::Exists, "{name}");
    }
    assert_eq!(preview.keys[0].status, KeyFileStatus::Exists);
    // Matching is case-insensitive, as registry key names are.
    let renamed = FixedSessions(Sessions {
        sessions: vec![session(
            "WEB%20PROD",
            &[("HostName", text("other.example.com"))],
        )],
        unreadable: Vec::new(),
        code_page: None,
    });
    let preview = u.core.putty_preview(&renamed).unwrap();
    assert_eq!(preview.hosts[0].status, ImportStatus::Exists);
}

fn hosts_count(u: &common::Unlocked) -> usize {
    hosts(u).len()
}

/// A source that cannot be read.
struct Broken;

impl SessionSource for Broken {
    fn sessions(&self) -> std::io::Result<Sessions> {
        Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied))
    }
}

#[test]
fn an_unreadable_source_and_no_sessions() {
    let u = first_run();
    assert!(u.core.putty_preview(&Broken).is_err());
    assert!(
        u.core
            .putty_import(&Broken, PuttyImportInput::default())
            .is_err()
    );
    let none = FixedSessions::default();
    let preview = u.core.putty_preview(&none).unwrap();
    assert!(preview.hosts.is_empty());
    let summary = u
        .core
        .putty_import(&none, PuttyImportInput::default())
        .unwrap();
    assert_eq!(summary.added, 0);
}
