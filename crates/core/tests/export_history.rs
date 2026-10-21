//! The options of Export Data and Import Data decided by the owner (D50,
//! D51): the command history and the connection log only when asked for,
//! imported history through the secret detector of live recording, keys
//! this client cannot read kept as "unknown", and the full restore.

mod common;

use std::collections::BTreeMap;

use common::{first_run, save};
use secrecy::{ExposeSecret, SecretString};
use serde_json::{Value, json};
use shellpp_core::keys::generate::KeyKind;
use shellpp_core::service::dto::{
    DataExportInput, DataImportContents, DataImportContentsInput, DataImportInput,
    HistoryCandidate, HistoryListInput, KeyGenerateInput, RecordRef, RecordType, SettingsSetInput,
};
use shellpp_core::service::host_api::{ConnectionLogStart, HistoryDecision, HistoryRecordInput};
use uuid::Uuid;

/// A store with one host, a command of its history and an open connection
/// log entry for it.
fn with_history() -> (common::Unlocked, Uuid) {
    let u = first_run();
    let host = save(
        &u.core,
        u.vault_id,
        None,
        RecordType::Host,
        json!({"label": "web", "address": "web.example.com"}),
        &[],
    )
    .expect("host")
    .record_id();
    let link = RecordRef {
        vault_id: u.vault_id,
        record_id: host,
    };
    let decision = u
        .core
        .history_record(HistoryRecordInput {
            host: Some(link),
            session_id: Uuid::now_v7(),
            candidate: HistoryCandidate {
                command: "ls -la".to_owned(),
                prompt_text: "deploy@web:~$".to_owned(),
                typed_printable: true,
                alt_screen: false,
            },
            injected: false,
            record_history: true,
        })
        .expect("history");
    assert_eq!(decision, HistoryDecision::Recorded);
    u.core
        .connection_log_start(ConnectionLogStart {
            host: Some(link),
            host_label: Some("web".to_owned()),
            address: "web.example.com".to_owned(),
            port: 22,
            username: Some("deploy".to_owned()),
            os: None,
        })
        .expect("log entry");
    (u, host)
}

fn everything() -> DataExportInput {
    DataExportInput {
        include_history: true,
        include_connection_log: true,
        ..DataExportInput::default()
    }
}

fn records_of<'a>(document: &'a Value, kind: &str) -> Vec<&'a Value> {
    document["vaults"][0]["records"]
        .as_array()
        .expect("records")
        .iter()
        .filter(|record| record["type"] == kind)
        .collect()
}

fn contents(core: &shellpp_core::Core, text: &SecretString) -> DataImportContents {
    core.data_import_contents(DataImportContentsInput {
        text: text.clone(),
        password: None,
    })
    .expect("contents")
}

#[test]
fn history_and_the_log_are_exported_only_when_asked_for() {
    let (source, _) = with_history();
    let plain = source
        .core
        .data_export(DataExportInput::default())
        .expect("export");
    let document: Value = serde_json::from_str(plain.text.expose_secret()).unwrap();
    assert!(records_of(&document, "history_entry").is_empty());
    assert!(records_of(&document, "connection_log_entry").is_empty());
    assert_eq!(
        contents(&source.core, &plain.text),
        DataImportContents::default()
    );

    let full = source.core.data_export(everything()).expect("export");
    let document: Value = serde_json::from_str(full.text.expose_secret()).unwrap();
    assert_eq!(records_of(&document, "history_entry").len(), 1);
    assert_eq!(records_of(&document, "connection_log_entry").len(), 1);
    assert_eq!(full.counts.get("history_entry"), Some(&1));
    assert_eq!(
        contents(&source.core, &full.text),
        DataImportContents {
            history: 1,
            connection_log: 1
        }
    );

    // The encrypted file tells the same once its password is given.
    let password = format!("export-{}", common::random_hex());
    let sealed = source
        .core
        .data_export(DataExportInput {
            password: Some(SecretString::from(password.clone())),
            ..everything()
        })
        .expect("encrypted export");
    let told = source
        .core
        .data_import_contents(DataImportContentsInput {
            text: sealed.text.clone(),
            password: Some(SecretString::from(password)),
        })
        .expect("contents");
    assert_eq!(told.history, 1);
}

#[test]
fn imported_history_passes_the_secret_detector_and_names_the_hosts_here() {
    let (source, host) = with_history();
    let export = source.core.data_export(everything()).expect("export");
    let mut document: Value = serde_json::from_str(export.text.expose_secret()).unwrap();
    let template = records_of(&document, "history_entry")[0].clone();
    let records = document["vaults"][0]["records"].as_array_mut().unwrap();
    // Two more entries of the same host: one live recording would have
    // dropped (S1, `-p<value>`), one it would have kept.
    for command in ["mysql -pS3cr3tValue db", "uptime"] {
        let mut entry = template.clone();
        entry["record_id"] = json!(Uuid::now_v7().to_string());
        entry["data"]["command"] = json!(command);
        records.push(entry);
    }
    let text = SecretString::from(document.to_string());

    let target = first_run();
    let summary = target
        .core
        .data_import(DataImportInput::new(text.clone(), None))
        .expect("import");
    assert_eq!(summary.history_left_out, 1);
    assert!(summary.refused.is_empty(), "{:?}", summary.refused);
    // The host link now names the personal vault here.
    let listed = target
        .core
        .history_list(HistoryListInput {
            host: Some(RecordRef {
                vault_id: target.vault_id,
                record_id: host,
            }),
            ..Default::default()
        })
        .expect("history");
    let mut commands: Vec<String> = listed.into_iter().map(|entry| entry.command).collect();
    commands.sort();
    assert_eq!(commands, ["ls -la", "uptime"]);

    // The open log entry arrives ended as "unknown", naming the host here.
    let again = target.core.data_export(everything()).expect("export");
    let document: Value = serde_json::from_str(again.text.expose_secret()).unwrap();
    let log = records_of(&document, "connection_log_entry");
    assert_eq!(log.len(), 1);
    assert_eq!(log[0]["data"]["end_reason"], "unknown");
    assert_eq!(
        log[0]["data"]["host"]["vault_id"],
        target.vault_id.to_string()
    );
    assert_eq!(log[0]["data"]["address"], "web.example.com");

    // A second import finds both already here.
    let second = target
        .core
        .data_import(DataImportInput::new(text, None))
        .expect("import");
    assert_eq!(second.added, 0);
    assert_eq!(second.conflicts, 0);
}

#[test]
fn history_and_the_log_stay_out_when_turned_off() {
    let (source, _) = with_history();
    let export = source.core.data_export(everything()).expect("export");
    // Device records are skipped either way.
    let taken = first_run()
        .core
        .data_import(DataImportInput::new(export.text.clone(), None))
        .expect("import");
    let target = first_run();
    let summary = target
        .core
        .data_import(DataImportInput {
            import_history: false,
            import_connection_log: false,
            ..DataImportInput::new(export.text.clone(), None)
        })
        .expect("import");
    assert_eq!(summary.skipped, taken.skipped + 2, "counted as skipped");
    let again = target.core.data_export(everything()).expect("export");
    let document: Value = serde_json::from_str(again.text.expose_secret()).unwrap();
    assert!(records_of(&document, "history_entry").is_empty());
    assert!(records_of(&document, "connection_log_entry").is_empty());
}

#[test]
fn a_broken_log_entry_is_refused_with_its_field() {
    let (source, _) = with_history();
    let export = source.core.data_export(everything()).expect("export");
    let mut document: Value = serde_json::from_str(export.text.expose_secret()).unwrap();
    let records = document["vaults"][0]["records"].as_array_mut().unwrap();
    let log = records
        .iter_mut()
        .find(|record| record["type"] == "connection_log_entry")
        .unwrap();
    log["data"]["port"] = json!(0);
    let target = first_run();
    let summary = target
        .core
        .data_import(DataImportInput::new(
            SecretString::from(document.to_string()),
            None,
        ))
        .expect("import");
    assert_eq!(summary.refused.len(), 1);
    assert_eq!(summary.refused[0].record_type, "connection_log_entry");
    assert_eq!(summary.refused[0].errors[0].field, "port");
}

#[test]
fn a_key_this_client_cannot_read_is_kept_as_unknown() {
    let source = first_run();
    source
        .core
        .key_generate(KeyGenerateInput {
            vault_id: source.vault_id,
            label: "from a newer client".to_owned(),
            kind: KeyKind::Ed25519,
            passphrase: None,
            save_passphrase: false,
        })
        .expect("key");
    let export = source
        .core
        .data_export(DataExportInput::default())
        .expect("export");
    let mut document: Value = serde_json::from_str(export.text.expose_secret()).unwrap();
    let records = document["vaults"][0]["records"].as_array_mut().unwrap();
    let key = records
        .iter_mut()
        .find(|record| record["type"] == "key")
        .unwrap();
    let unreadable = "-----BEGIN FUTURE PRIVATE KEY-----\nAAAA\n-----END FUTURE PRIVATE KEY-----\n";
    key["data"]["private_key"] = json!(unreadable);
    key["data"]["future_member"] = json!({"kept": true});
    let key_id = key["record_id"].clone();
    // A key without a label is still refused.
    let mut unlabeled = key.clone();
    unlabeled["record_id"] = json!(Uuid::now_v7().to_string());
    unlabeled["data"].as_object_mut().unwrap().remove("label");
    records.push(unlabeled);

    let target = first_run();
    let summary = target
        .core
        .data_import(DataImportInput::new(
            SecretString::from(document.to_string()),
            None,
        ))
        .expect("import");
    assert_eq!(summary.unknown_keys, 1);
    assert_eq!(summary.added, 1);
    assert_eq!(summary.refused.len(), 1);
    assert_eq!(summary.refused[0].errors[0].field, "label");
    // Kept as it was, members of later versions too.
    let again = target
        .core
        .data_export(DataExportInput::default())
        .expect("export");
    let document: Value = serde_json::from_str(again.text.expose_secret()).unwrap();
    let kept = records_of(&document, "key");
    assert_eq!(kept.len(), 1);
    assert_eq!(kept[0]["record_id"], key_id);
    assert_eq!(kept[0]["data"]["private_key"], unreadable);
    assert_eq!(kept[0]["data"]["future_member"]["kept"], true);
}

#[test]
fn a_full_restore_applies_every_setting_of_the_file() {
    let source = first_run();
    source
        .core
        .settings_set(SettingsSetInput {
            values: BTreeMap::from([
                ("terminal.os_detection_enabled".to_owned(), json!(false)),
                ("sftp.confirm_delete".to_owned(), json!(false)),
                ("terminal.font_size".to_owned(), json!(18)),
            ]),
            reset: Vec::new(),
        })
        .expect("settings");
    let export = source
        .core
        .data_export(DataExportInput::default())
        .expect("export");

    let target = first_run();
    let summary = target
        .core
        .data_import(DataImportInput {
            apply_settings: false,
            full_restore: true,
            ..DataImportInput::new(export.text.clone(), None)
        })
        .expect("import");
    assert_eq!(summary.settings_left_out, 0);
    let settings = target.core.settings_get().expect("settings");
    let value = |key: &str| settings[key].clone();
    assert_eq!(value("terminal.os_detection_enabled"), false);
    assert_eq!(value("sftp.confirm_delete"), false);
    assert_eq!(value("terminal.font_size"), 18);
}
