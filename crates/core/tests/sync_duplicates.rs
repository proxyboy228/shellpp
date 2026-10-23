//! Duplicates from sync (UI 13.7; data-model 18.3): a pair shows both
//! records side by side with secrets masked, the older one on the left;
//! a merge keeps either side and sends the other to Recently deleted;
//! "Keep both" leaves both and the pair is not offered again.

mod common;

use serde_json::json;

use common::{first_run, random_hex};

#[test]
fn a_pair_shows_both_sides_masked_and_merges_into_either() {
    let unlocked = first_run();
    let core = &unlocked.core;
    let (older_secret, newer_secret) = (random_hex(), random_hex());
    let older = unlocked.host_with_password(
        json!({"label": "Web", "address": "dup.example.com"}),
        &older_secret,
    );
    let newer = unlocked.host_with_password(
        json!({"label": "Web (laptop)", "address": "DUP.example.com"}),
        &newer_secret,
    );
    let other = unlocked.host(json!({"address": "other.example.com"}));
    assert_eq!(core.sync_duplicates_scan().expect("scan"), 1);
    assert_eq!(core.sync_status().expect("status").duplicates, 1);

    let pairs = core.sync_duplicates().expect("pairs");
    assert_eq!(pairs.len(), 1);
    let pair = &pairs[0];
    assert_eq!(pair.type_name, "host");
    assert_eq!(
        pair.left.record_id, older.record_id,
        "the older on the left"
    );
    assert_eq!(pair.right.record_id, newer.record_id);
    assert_eq!(pair.left.title, "Web");
    assert!(pair.left.created_at <= pair.right.created_at);
    let field = |path: &str| {
        pair.fields
            .iter()
            .find(|field| field.path == path)
            .unwrap_or_else(|| panic!("{path}"))
    };
    assert!(field("label").differs);
    assert_eq!(field("label").right, "Web (laptop)");
    assert!(field("address").differs, "compared as stored");
    let password = field("ssh.password");
    assert!(password.differs, "different secrets differ");
    assert_eq!(
        (password.left.as_str(), password.right.as_str()),
        ("••••••", "••••••")
    );
    let text = serde_json::to_string(&pairs).expect("json");
    assert!(!text.contains(&older_secret) && !text.contains(&newer_secret));

    // Keep right: the left one goes to Recently deleted.
    core.sync_duplicates_merge(pair.right.record_id, pair.left.record_id)
        .expect("merged");
    assert!(core.sync_duplicates().expect("pairs").is_empty());
    let deleted: Vec<_> = core
        .recently_deleted_list()
        .expect("trash")
        .into_iter()
        .map(|item| item.record_id)
        .collect();
    assert_eq!(deleted, vec![older.record_id]);
    let live = unlocked.query(
        &[shellpp_core::service::dto::RecordType::Host],
        shellpp_core::service::dto::QueryScope::All,
        shellpp_core::service::dto::SortMode::Name,
    );
    assert!(live.contains(&newer.record_id) && live.contains(&other.record_id));
    assert!(!live.contains(&older.record_id));
}

/// An env var's value is a secret (data-model 3.5, `env_var`): the
/// comparison, a conflict and Recently deleted show it masked and its name
/// in the clear. Each side names the device of its last change (as in UI
/// 13.6). Debug shows no host data (docs/security.md, section 9.1).
#[test]
fn env_values_are_masked_and_debug_shows_no_host_data() {
    let unlocked = first_run();
    let core = &unlocked.core;
    let value = random_hex();
    let host = json!({"address": "env.example.com",
        "ssh": {"env": [{"name": "DEPLOY_TOKEN", "value": value}]}});
    let first = unlocked.host(host.clone());
    let _second = unlocked.host(host);
    assert_eq!(core.sync_duplicates_scan().expect("scan"), 1);
    let pairs = core.sync_duplicates().expect("pairs");
    let text = serde_json::to_string(&pairs).expect("json");
    assert!(!text.contains(&value), "the value is masked: {text}");
    let env = pairs[0]
        .fields
        .iter()
        .find(|field| field.path == "ssh.env")
        .expect("the env field");
    assert_eq!(env.left, r#"[{"name":"DEPLOY_TOKEN","value":"••••••"}]"#);
    assert!(!env.differs);
    for side in [&pairs[0].left, &pairs[0].right] {
        assert!(side.this_device, "made here");
        assert!(!side.device_id.is_empty());
    }
    let debug = format!("{pairs:?}");
    for host_data in ["env.example.com", "DEPLOY_TOKEN", value.as_str()] {
        assert!(!debug.contains(host_data), "{host_data} in {debug}");
    }

    // A conflict copy of the host and its entry in Recently deleted.
    let data = first.data.as_object().cloned().expect("data");
    core.sync_conflict_insert_for_test(
        first.record_id,
        Some(data),
        uuid::Uuid::now_v7(),
        std::time::SystemTime::now(),
    )
    .expect("conflict");
    let conflicts = serde_json::to_string(&core.sync_conflicts().expect("conflicts")).unwrap();
    assert!(conflicts.contains("DEPLOY_TOKEN"));
    assert!(!conflicts.contains(&value), "masked in a conflict");
    // security 9.1: a conflict item's Debug shows no data of either side.
    let debug = format!("{:?}", core.sync_conflicts().expect("conflicts"));
    assert!(!debug.contains("DEPLOY_TOKEN"), "{debug}");
    assert!(!debug.contains(".example."), "{debug}");
    core.record_delete(shellpp_core::service::dto::RecordsRef {
        vault_id: unlocked.vault_id,
        record_ids: vec![first.record_id],
    })
    .expect("deleted");
    let trash = serde_json::to_string(&core.recently_deleted_list().expect("trash")).unwrap();
    assert!(trash.contains("DEPLOY_TOKEN"));
    assert!(!trash.contains(&value), "masked in Recently deleted");
}

#[test]
fn keep_both_is_not_offered_again() {
    let unlocked = first_run();
    let core = &unlocked.core;
    let a = unlocked.host(json!({"address": "same.example.com"}));
    let b = unlocked.host(json!({"address": "same.example.com"}));
    assert_eq!(core.sync_duplicates_scan().expect("scan"), 1);
    let pair = core.sync_duplicates().expect("pairs").remove(0);
    assert_eq!(
        (pair.left.record_id, pair.right.record_id),
        (a.record_id, b.record_id)
    );
    assert!(pair.fields.iter().all(|field| !field.differs));
    core.sync_duplicates_keep(pair.record_id, pair.other_record_id)
        .expect("kept");
    assert!(core.sync_duplicates().expect("pairs").is_empty());
    assert_eq!(
        core.sync_duplicates_scan().expect("scan"),
        0,
        "kept stays kept"
    );
    assert!(core.sync_duplicates().expect("pairs").is_empty());
    assert_eq!(core.sync_status().expect("status").duplicates, 0);
}
