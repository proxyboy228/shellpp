//! Export… of Recently deleted (UI 14; stage 7): the chosen entries' last
//! live versions in an ordinary export file, without settings, readable by
//! Import Data.

mod common;

use secrecy::ExposeSecret as _;
use serde_json::{Value, json};
use shellpp_core::service::dto::{DataExportInput, RecordsRef};

use common::first_run;

#[test]
fn recently_deleted_entries_export_alone() {
    let unlocked = first_run();
    let kept = unlocked.host(json!({"address": "kept.example.com"}));
    let gone = unlocked.host(json!({"address": "gone.example.com"}));
    unlocked
        .core
        .record_delete(RecordsRef {
            vault_id: unlocked.vault_id,
            record_ids: vec![gone.record_id],
        })
        .expect("deleted");
    let list = unlocked.core.recently_deleted_list().expect("trash");
    assert_eq!(list.len(), 1);
    let export = unlocked
        .core
        .data_export(DataExportInput {
            password: None,
            recently_deleted: vec![gone.record_id],
            ..DataExportInput::default()
        })
        .expect("export");
    let file: Value = serde_json::from_str(export.text.expose_secret()).expect("json");
    let records = file["vaults"][0]["records"].as_array().expect("records");
    assert_eq!(records.len(), 1, "only the deleted host");
    assert_eq!(records[0]["data"]["address"], "gone.example.com");
    assert_eq!(file["settings"], json!({}), "no settings");
    assert_eq!(export.counts.get("host"), Some(&1));
    let _ = kept;
}
