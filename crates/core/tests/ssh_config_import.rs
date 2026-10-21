//! Import of `~/.ssh/config` (SPEC "Import"; UI 2.5; night decision D35):
//! a config in a temporary home folder with Include, Host templates, a
//! basic and an unsupported Match, ProxyJump, ProxyCommand and options that
//! are not imported; key files generated at run time (one encrypted, one
//! missing). The preview, the import, the links between the records and a
//! second import that adds nothing.

mod common;
mod stage2;

use std::collections::BTreeMap;
use std::path::Path;

use common::{as_host, first_run};
use secrecy::ExposeSecret;
use serde_json::{Value, json};
use shellpp_core::service::dto::{
    ImportStatus, KeyFileStatus, QueryScope, RecordType, RecordsQuery, SortMode,
    SshConfigImportInput, SshConfigSource,
};

const CONFIG: &str = "\
# Global options apply to every host.
ServerAliveInterval 45
Include config.d/*

Host bastion
  HostName bastion.example.com
  User admin
  Port 2222
  IdentityFile ~/.ssh/id_bastion

Host web-1 web1
  HostName %h.example.com
  ProxyJump admin@bastion:2222
  ForwardAgent yes
  SetEnv LANG=en_US.UTF-8 TZ=UTC
  SendEnv LC_*
  LocalForward 8080 localhost:80
  Ciphers aes256-ctr

Host legacy
  ProxyCommand ssh -W %h:%p bastion

Host *.example.com
  User nobody-matches-an-alias

Host *
  IdentityFile ~/.ssh/id_default
  User fallback
";

const INCLUDED: &str = "\
Host db
  HostName db.internal
  IdentityFile ~/.ssh/id_db

Match host db.internal
  Port 5432

Match exec \"true\"
  User nobody
";

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

#[test]
fn hosts_keys_and_links_are_imported_once() {
    let home = tempfile::tempdir().unwrap();
    let ssh = home.path().join(".ssh");
    write(&ssh.join("config"), CONFIG);
    write(&ssh.join("config.d").join("extra"), INCLUDED);
    let bastion_key = stage2::generate(stage2::KeyKind::Ed25519, "bastion");
    write(
        &ssh.join("id_bastion"),
        &stage2::openssh(&bastion_key, None),
    );
    let db_key = stage2::generate(stage2::KeyKind::EcdsaP256, "db");
    let pass = stage2::passphrase();
    write(
        &ssh.join("id_db"),
        &stage2::openssh(&db_key, Some(pass.expose_secret())),
    );
    let source = SshConfigSource {
        path: ssh.join("config"),
        home: home.path().to_path_buf(),
    };

    let u = first_run();
    let preview = u.core.ssh_config_preview(source.clone()).expect("preview");
    let aliases: Vec<&str> = preview
        .hosts
        .iter()
        .map(|host| host.alias.as_str())
        .collect();
    // The included file comes first, where its Include line is.
    assert_eq!(aliases, ["db", "bastion", "web-1", "legacy"]);
    assert_eq!(preview.templates, 2, "Host *.example.com and Host *");
    let host = |alias: &str| {
        preview
            .hosts
            .iter()
            .find(|host| host.alias == alias)
            .unwrap()
    };
    assert_eq!(host("bastion").address, "bastion.example.com");
    assert_eq!(host("bastion").port, Some(2222));
    assert_eq!(host("bastion").username.as_deref(), Some("admin"));
    assert_eq!(host("web-1").address, "web-1.example.com");
    assert_eq!(host("web-1").username.as_deref(), Some("fallback"));
    assert_eq!(host("web-1").jump, ["admin@bastion:2222"]);
    let notes = host("web-1").notes.join("\n");
    assert!(notes.contains("SendEnv"), "{notes}");
    // LocalForward becomes a rule of the host (stage 5), not a note.
    assert!(!notes.contains("Forward"), "{notes}");
    assert!(notes.contains("ciphers"), "{notes}");
    // Match host applies on the HostName; Match exec never applies.
    assert_eq!(host("db").port, Some(5432));
    assert_eq!(host("db").username.as_deref(), Some("fallback"));
    assert!(
        preview
            .problems
            .iter()
            .any(|problem| problem.contains("Match exec"))
    );
    assert_eq!(host("legacy").status, ImportStatus::Error);
    assert_eq!(
        host("legacy").reason.as_deref(),
        Some("ProxyCommand is not supported")
    );
    for alias in ["db", "bastion", "web-1"] {
        assert_eq!(host(alias).status, ImportStatus::New, "{alias}");
    }
    let key = |name: &str| {
        preview
            .keys
            .iter()
            .find(|key| key.path.ends_with(name))
            .unwrap_or_else(|| panic!("{name}"))
            .status
            .clone()
    };
    assert_eq!(key("id_bastion"), KeyFileStatus::New);
    assert_eq!(key("id_db"), KeyFileStatus::NeedsPassphrase);
    assert_eq!(key("id_default"), KeyFileStatus::Missing);

    // Import everything with the passphrase of the encrypted key.
    let db_path = preview
        .keys
        .iter()
        .find(|key| key.path.ends_with("id_db"))
        .unwrap()
        .path
        .clone();
    let input = SshConfigImportInput {
        source: source.clone(),
        aliases: None,
        passphrases: BTreeMap::from([(db_path, pass.clone())]),
        save_passphrases: false,
    };
    let summary = u.core.ssh_config_import(input.clone()).expect("import");
    assert_eq!(summary.added, 3);
    assert_eq!(summary.skipped, 0);
    assert_eq!(
        summary.errors,
        [(
            "legacy".to_owned(),
            "ProxyCommand is not supported".to_owned()
        )]
    );
    assert_eq!(summary.keys_added, 2);
    assert_eq!(summary.keys_skipped, 1, "id_default is missing");
    assert_eq!(
        summary.commands_skipped, 2,
        "legacy's ProxyCommand and the Match exec, neither run nor stored"
    );
    assert_eq!(summary.jump_chains_left_out, 0);

    let hosts: Vec<_> = u
        .core
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
        .collect();
    assert_eq!(hosts.len(), 3);
    let data = |label: &str| -> Value {
        hosts
            .iter()
            .find(|host| host.data["label"] == label)
            .unwrap()
            .data
            .clone()
    };
    let bastion = hosts
        .iter()
        .find(|host| host.data["label"] == "bastion")
        .unwrap();
    let keys = u.core.keys_list().unwrap();
    let bastion_key_id = keys
        .iter()
        .find(|key| key.label == "id_bastion")
        .unwrap()
        .record_id;
    assert_eq!(
        data("bastion")["ssh"]["key_id"],
        json!(bastion_key_id.to_string())
    );
    assert_eq!(data("bastion")["ssh"]["keepalive_s"], 45);
    assert_eq!(data("bastion")["import_key"], "ssh_config:bastion");
    assert_eq!(
        data("web-1")["ssh"]["jump_chain"],
        json!([bastion.record_id.to_string()])
    );
    assert_eq!(data("web-1")["ssh"]["agent_forwarding"], true);
    assert_eq!(data("web-1")["ssh"]["agent_forward_system"], true);
    // What the import left out is in the host's own notes (D35).
    let notes = data("web-1")["notes"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    assert!(notes.contains("SendEnv"), "{notes}");
    assert!(notes.contains("ciphers"), "{notes}");
    assert!(
        notes.contains("the user and port of the hop are not kept"),
        "{notes}"
    );
    assert_eq!(
        data("web-1")["ssh"]["env"],
        json!([{"name": "LANG", "value": "en_US.UTF-8"}, {"name": "TZ", "value": "UTC"}])
    );
    assert!(keys.iter().any(|key| key.label == "id_db" && key.encrypted));

    // A second import adds nothing and changes nothing.
    let again = u.core.ssh_config_import(input).expect("again");
    assert_eq!(again.added, 0);
    assert_eq!(again.skipped, 3);
    let preview = u.core.ssh_config_preview(source).unwrap();
    for alias in ["db", "bastion", "web-1"] {
        let status = preview
            .hosts
            .iter()
            .find(|host| host.alias == alias)
            .unwrap()
            .status;
        assert_eq!(status, ImportStatus::Exists, "{alias}");
    }
    assert!(
        preview
            .keys
            .iter()
            .any(|key| key.path.ends_with("id_bastion") && key.status == KeyFileStatus::Exists)
    );
}

#[test]
fn a_missing_file_and_include_loops_are_reported() {
    let u = first_run();
    let home = tempfile::tempdir().unwrap();
    let missing = SshConfigSource {
        path: home.path().join(".ssh").join("config"),
        home: home.path().to_path_buf(),
    };
    assert!(u.core.ssh_config_preview(missing).is_err());
    // A config that includes itself, and a chosen subset of hosts.
    let ssh = home.path().join(".ssh");
    write(
        &ssh.join("config"),
        "Include config\nHost one\n  HostName 192.0.2.1\nHost two\n  HostName 192.0.2.2\n",
    );
    let source = SshConfigSource {
        path: ssh.join("config"),
        home: home.path().to_path_buf(),
    };
    let preview = u.core.ssh_config_preview(source.clone()).unwrap();
    assert!(
        preview
            .problems
            .iter()
            .any(|problem| problem.contains("loops"))
    );
    let summary = u
        .core
        .ssh_config_import(SshConfigImportInput {
            source,
            aliases: Some(vec!["TWO".to_owned()]),
            passphrases: BTreeMap::new(),
            save_passphrases: false,
        })
        .unwrap();
    assert_eq!(summary.added, 1, "only the chosen host");
}

#[test]
fn network_paths_are_never_opened() {
    let u = first_run();
    let home = tempfile::tempdir().unwrap();
    let ssh = home.path().join(".ssh");
    // Documentation names only (RFC 2606); neither path is ever opened.
    write(
        &ssh.join("config"),
        &[
            r"Include \\files.example.invalid\share\more",
            "Include //files.example.invalid/share/other",
            "Host one",
            "  HostName 192.0.2.1",
            r"  IdentityFile \\files.example.invalid\keys\id",
            "Host two",
            "  HostName 192.0.2.2",
            "  IdentityFile //files.example.invalid/keys/id2",
            "Host three",
            "  HostName 192.0.2.3",
            r"  IdentityFile \??\UNC\files.example.invalid\keys\id3",
            "",
        ]
        .join("\n"),
    );
    let source = SshConfigSource {
        path: ssh.join("config"),
        home: home.path().to_path_buf(),
    };
    let preview = u.core.ssh_config_preview(source).unwrap();
    let network: Vec<&String> = preview
        .problems
        .iter()
        .filter(|problem| problem.contains("network path"))
        .collect();
    assert_eq!(network.len(), 2, "{:?}", preview.problems);
    assert_eq!(preview.hosts.len(), 3);
    assert_eq!(preview.keys.len(), 3);
    for key in &preview.keys {
        assert_eq!(
            key.status,
            KeyFileStatus::Invalid("a network path is not read".to_owned()),
            "{}",
            key.path
        );
    }
}

#[test]
fn a_jump_host_that_is_not_imported_leaves_no_silent_direct_connection() {
    let u = first_run();
    let home = tempfile::tempdir().unwrap();
    let ssh = home.path().join(".ssh");
    write(
        &ssh.join("config"),
        "Host bastion\n  HostName 192.0.2.10\n\
         Host legacy\n  HostName 192.0.2.11\n  ProxyCommand nc %h %p\n\
         Host app\n  HostName 10.0.0.5\n  ProxyJump bastion\n\
         Host old-app\n  HostName 10.0.0.6\n  ProxyJump legacy\n\
         Host *\n  PermitLocalCommand no\n  LocalCommand echo hi\n",
    );
    let source = SshConfigSource {
        path: ssh.join("config"),
        home: home.path().to_path_buf(),
    };
    // The preview names the refused hop.
    let preview = u.core.ssh_config_preview(source.clone()).unwrap();
    let old_app = preview
        .hosts
        .iter()
        .find(|host| host.alias == "old-app")
        .unwrap();
    assert!(
        old_app
            .notes
            .iter()
            .any(|note| note.contains("ProxyJump legacy is not imported")),
        "{:?}",
        old_app.notes
    );
    // bastion left unchecked, legacy refused: neither app gets a chain,
    // both say why, and the summary counts them.
    let summary = u
        .core
        .ssh_config_import(SshConfigImportInput {
            source,
            aliases: Some(vec!["app".to_owned(), "old-app".to_owned()]),
            passphrases: BTreeMap::new(),
            save_passphrases: false,
        })
        .unwrap();
    assert_eq!(summary.added, 2);
    assert_eq!(summary.jump_chains_left_out, 2);
    // legacy's ProxyCommand and the one LocalCommand line of Host *, which
    // applies to four hosts; "PermitLocalCommand no" holds no command.
    assert_eq!(summary.commands_skipped, 2);
    let hosts: Vec<_> = u
        .core
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
        .collect();
    for label in ["app", "old-app"] {
        let host = hosts
            .iter()
            .find(|host| host.data["label"] == label)
            .unwrap();
        // The missing hop stays in the chain, so the host is refused at
        // connect (D44) instead of connecting directly.
        assert_eq!(
            host.data["ssh"]["jump_chain"],
            json!([uuid::Uuid::nil().to_string()]),
            "{label}"
        );
        let notes = host.data["notes"].as_str().unwrap_or_default();
        assert!(
            notes.contains("the jump host is missing"),
            "{label}: {notes}"
        );
        let config = u
            .core
            .session_config(&shellpp_core::service::dto::SessionTarget::Host {
                vault_id: u.vault_id,
                record_id: host.record_id,
            })
            .unwrap();
        // The missing hop is named: the connection fails before it starts.
        assert_eq!(
            config.route_problem,
            Some(shellpp_core::service::host_api::RouteProblem::MissingHop { index: 0 }),
            "{label}"
        );
    }
}

/// A jump host that its own rules refuse is not written, and no imported
/// host points at it: the host behind it gets no chain and a note.
#[test]
fn no_jump_chain_points_at_a_host_that_was_refused() {
    let u = first_run();
    let home = tempfile::tempdir().unwrap();
    let ssh = home.path().join(".ssh");
    write(
        &ssh.join("config"),
        &[
            "Host jump",
            "  HostName 192.0.2.70",
            "  SetEnv FOO-BAR=1",
            "Host app",
            "  HostName 10.0.0.8",
            "  ProxyJump jump",
            "",
        ]
        .join("\n"),
    );
    let summary = u
        .core
        .ssh_config_import(SshConfigImportInput {
            source: SshConfigSource {
                path: ssh.join("config"),
                home: home.path().to_path_buf(),
            },
            aliases: None,
            passphrases: BTreeMap::new(),
            save_passphrases: false,
        })
        .unwrap();
    assert_eq!(summary.added, 1, "{summary:?}");
    assert_eq!(summary.errors.len(), 1);
    assert_eq!(summary.errors[0].0, "jump");
    assert_eq!(summary.jump_chains_left_out, 1);
    let hosts: Vec<_> = u
        .core
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
        .collect();
    let app = &hosts[0].data;
    assert_eq!(app["label"], "app");
    assert_eq!(
        app["ssh"]["jump_chain"],
        json!([uuid::Uuid::nil().to_string()])
    );
}

/// `LocalForward`, `RemoteForward` and `DynamicForward` become the host's
/// forwarding rules, started on connect; forms the rules cannot hold are
/// noted (stage 5).
#[test]
fn forwardings_become_rules_of_the_host() {
    use shellpp_core::service::snippets::ForwardType;
    let home = tempfile::tempdir().unwrap();
    let ssh = home.path().join(".ssh");
    write(
        &ssh.join("config"),
        "Host app\n  HostName app.example.com\n  LocalForward 15432 db.internal:5432\n  \
         LocalForward [::1]:8443 localhost:443\n  RemoteForward 9000 127.0.0.1:3000\n  \
         DynamicForward *:1080\n  RemoteForward 1081\n  LocalForward /tmp/sock /run/x.sock\n",
    );
    let source = SshConfigSource {
        path: ssh.join("config"),
        home: home.path().to_path_buf(),
    };
    let u = first_run();
    let summary = u
        .core
        .ssh_config_import(SshConfigImportInput {
            source,
            aliases: None,
            passphrases: BTreeMap::new(),
            save_passphrases: false,
        })
        .expect("import");
    assert_eq!(summary.added, 1);
    assert_eq!(summary.rules_added, 4);
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
            // All interfaces: imported without autostart.
            (
                ForwardType::Dynamic,
                "0.0.0.0".into(),
                1080,
                None,
                None,
                false
            ),
            (
                ForwardType::Local,
                "::1".into(),
                8443,
                Some("localhost".into()),
                Some(443),
                true
            ),
            (
                ForwardType::Remote,
                "127.0.0.1".into(),
                9000,
                Some("127.0.0.1".into()),
                Some(3000),
                true
            ),
            (
                ForwardType::Local,
                "127.0.0.1".into(),
                15432,
                Some("db.internal".into()),
                Some(5432),
                true
            ),
        ]
    );
    assert!(
        rules.iter().any(|rule| rule.wide_bind),
        "*:1080 is all interfaces"
    );
    // The forms rules cannot hold are in the host's notes.
    let hosts = u
        .core
        .records_query(RecordsQuery {
            vault_id: None,
            types: vec![RecordType::Host],
            scope: QueryScope::All,
            text: None,
            tags: Vec::new(),
            sort: SortMode::Manual,
        })
        .unwrap();
    let notes = as_host(hosts.into_iter().next().unwrap()).data["notes"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    assert!(
        notes.contains("RemoteForward 1081 is not imported"),
        "{notes}"
    );
    assert!(
        notes.contains("DynamicForward *:1080 listens on all interfaces"),
        "{notes}"
    );
    assert!(
        notes.contains("LocalForward /tmp/sock /run/x.sock is not imported"),
        "{notes}"
    );
}
