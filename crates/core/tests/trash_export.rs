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

/// UI 14 in local mode: the entry carries its vault and its last live data
/// with secrets masked, for the read-only details panel; entries older than
/// 90 days leave the list without a sync cycle (no server bound here).
#[test]
fn recently_deleted_lists_masked_data_and_purges_after_90_days() {
    let unlocked = first_run();
    let secret = format!("pw-{}", common::random_hex());
    let old = unlocked.host_with_password(json!({"address": "old.example.com"}), &secret);
    let delete = |id| {
        unlocked
            .core
            .record_delete(RecordsRef {
                vault_id: unlocked.vault_id,
                record_ids: vec![id],
            })
            .expect("deleted")
    };
    delete(old.record_id);
    unlocked
        .env
        .mem
        .clock
        .advance(std::time::Duration::from_secs(2 * 86_400));
    let recent = unlocked.host(json!({"address": "recent.example.com"}));
    delete(recent.record_id);

    let list = unlocked.core.recently_deleted_list().expect("trash");
    let ids: Vec<_> = list.iter().map(|item| item.record_id).collect();
    assert_eq!(ids, [recent.record_id, old.record_id], "newest first");
    let item = &list[1];
    assert_eq!(item.vault_id, unlocked.vault_id);
    assert!(!item.from_sync);
    let data = item.data.as_ref().expect("data");
    assert_eq!(data["address"], "old.example.com");
    assert_eq!(data["ssh"]["password"], "••••••", "the password is masked");
    let listed = serde_json::to_string(&list).expect("json");
    assert!(
        !listed.contains(&secret),
        "the secret never leaves the core"
    );

    // 89 days after the first deletion both stay; past 90 the old one is
    // purged (from the store too: it does not come back), the recent one
    // stays.
    unlocked
        .env
        .mem
        .clock
        .advance(std::time::Duration::from_secs(87 * 86_400));
    assert_eq!(
        unlocked.core.recently_deleted_list().expect("trash").len(),
        2
    );
    unlocked
        .env
        .mem
        .clock
        .advance(std::time::Duration::from_secs(2 * 86_400));
    let left: Vec<_> = unlocked
        .core
        .recently_deleted_list()
        .expect("trash")
        .into_iter()
        .map(|item| item.record_id)
        .collect();
    assert_eq!(left, [recent.record_id]);
    assert!(
        unlocked
            .core
            .recently_deleted_restore(&[old.record_id])
            .expect("restore")
            .is_empty(),
        "the purged entry is gone"
    );
}

/// security 9.1: a trash entry's `Debug` shows no address or data.
#[test]
fn a_recently_deleted_entry_debug_shows_no_host_data() {
    let unlocked = first_run();
    let host = unlocked.host(json!({"address": "secret-host.example.com", "label": "Prod DB"}));
    unlocked
        .core
        .record_delete(RecordsRef {
            vault_id: unlocked.vault_id,
            record_ids: vec![host.record_id],
        })
        .expect("deleted");
    let item = unlocked
        .core
        .recently_deleted_list()
        .expect("trash")
        .remove(0);
    let shown = format!("{item:?}");
    assert!(!shown.contains("secret-host"), "{shown}");
    assert!(!shown.contains("Prod DB"), "{shown}");
}
