//! Merging duplicates from sync (UI 13.7; data-model 18.3): records are
//! written through the session (copies of one key arrive only by sync, and
//! an import matches keys by fingerprint), and a pull is run through
//! `engine::apply_page` with envelopes sealed here, without a server.
//! Passwords and passphrases are generated at run time.

use std::collections::HashMap;

use secrecy::SecretString;
use serde_json::{Map, Value, json};
use shellpp_sync_api::api::{B64, SeqEnvelope};
use uuid::Uuid;

use super::dto::KeyGenerateInput;
use super::records::{Plan, parts};
use super::{Core, CoreConfig};
use crate::model::types::RecordKind;
use crate::platform::OsKind;
use crate::platform::memory::MemoryPlatform;
use crate::store::SyncStateRow;
use crate::sync::engine::{self, Selection};

fn random_text() -> String {
    crate::crypto::random_array::<12>()
        .unwrap()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn core() -> (tempfile::TempDir, Core) {
    let dir = tempfile::tempdir().unwrap();
    let mem = MemoryPlatform::new();
    let core = Core::open(
        CoreConfig {
            data_dir: dir.path().join("data"),
            app_version: "0.1.0".into(),
        },
        mem.platform(OsKind::Linux, "TEST"),
    )
    .unwrap();
    core.first_run_setup(SecretString::from(format!("Pw-{}", random_text())))
        .unwrap();
    (dir, core)
}

fn object(value: Value) -> Map<String, Value> {
    value.as_object().cloned().unwrap()
}

/// Writes a record as stored (no validation), as a pull leaves it.
fn put(core: &Core, kind: RecordKind, data: Value) -> Uuid {
    let now = core.now();
    let mut guard = core.inner();
    let mut parts = parts(&mut guard).unwrap();
    let id = parts.ids.next(now);
    let mut plan = Plan::default();
    plan.set(kind, id, object(data));
    let writes = plan.prepare(&mut parts, now).unwrap();
    core.commit_records(&mut parts, writes, Vec::new(), Vec::new())
        .unwrap();
    id
}

fn data(core: &Core, id: Uuid) -> Option<Map<String, Value>> {
    let guard = core.inner();
    let session = guard.session.as_ref().unwrap();
    session.index.get(&id).map(|record| record.data().clone())
}

fn deleted(core: &Core) -> Vec<Uuid> {
    let mut ids: Vec<Uuid> = core
        .recently_deleted_list()
        .unwrap()
        .into_iter()
        .map(|item| item.record_id)
        .collect();
    ids.sort();
    ids
}

/// Two copies of one key, the newer with a certificate and the saved
/// passphrase: whichever side is kept, the key rule of 7.2 applies (data-
/// model 18.3). The older one (the smaller id) stays and takes the
/// certificate and the passphrase; every reference (host key slots, a
/// certificate slot, a group's key, an identity's key) points to it; the
/// newer one is in Recently deleted.
#[test]
fn key_pairs_merge_by_the_key_rule_whichever_side_is_kept() {
    let (_dir, core) = core();
    let vault_id = core.vaults().unwrap()[0].vault_id;
    let passphrase = format!("Pp-{}", random_text());
    let older = core
        .key_generate(KeyGenerateInput {
            vault_id,
            label: "deploy".into(),
            kind: crate::keys::generate::KeyKind::Ed25519,
            passphrase: Some(SecretString::from(passphrase.clone())),
            save_passphrase: false,
        })
        .unwrap()
        .key
        .record_id;
    let mut copy = data(&core, older).unwrap();
    assert!(copy.get("passphrase").is_none_or(Value::is_null));
    let certificate = format!(
        "ssh-ed25519-cert-v01@openssh.com AAAA{} deploy",
        random_text()
    );
    copy.insert("label".into(), json!("deploy (laptop)"));
    copy.insert("certificate".into(), json!(certificate));
    copy.insert("passphrase".into(), json!(passphrase));
    let newer = put(&core, RecordKind::Key, Value::Object(copy));
    assert!(newer > older);
    let (older_s, newer_s) = (older.to_string(), newer.to_string());
    let host_old = put(
        &core,
        RecordKind::Host,
        json!({"address": "a.example.com", "ssh": {"key_id": older_s}}),
    );
    let host_new = put(
        &core,
        RecordKind::Host,
        json!({"address": "b.example.com",
            "ssh": {"key_id": newer_s, "certificate_key_id": newer_s}}),
    );
    let group = put(
        &core,
        RecordKind::Group,
        json!({"name": "Ops", "ssh": {"key_id": newer_s}}),
    );
    let identity = put(
        &core,
        RecordKind::Identity,
        json!({"label": "ops", "username": "ops", "key_id": newer_s}),
    );
    assert_eq!(core.sync_duplicates_scan().unwrap(), 1);
    let pairs = core.sync_duplicates().unwrap();
    assert_eq!(pairs.len(), 1);
    assert_eq!(pairs[0].type_name, "key");
    assert_eq!(
        (pairs[0].left.record_id, pairs[0].right.record_id),
        (older, newer)
    );

    // Keep right: the rule still keeps the older one.
    core.sync_duplicates_merge(newer, older).unwrap();
    let kept = data(&core, older).expect("the smallest id stays");
    assert!(data(&core, newer).is_none(), "the other one is removed");
    assert_eq!(kept["label"], json!("deploy"), "its own fields stay");
    assert_eq!(kept["certificate"], json!(certificate), "taken over");
    assert_eq!(kept["passphrase"], json!(passphrase), "taken over");
    assert_eq!(
        data(&core, host_old).unwrap()["ssh"]["key_id"],
        json!(older_s)
    );
    let ssh = data(&core, host_new).unwrap()["ssh"].clone();
    assert_eq!(ssh["key_id"], json!(older_s));
    assert_eq!(ssh["certificate_key_id"], json!(older_s));
    assert_eq!(data(&core, group).unwrap()["ssh"]["key_id"], json!(older_s));
    assert_eq!(data(&core, identity).unwrap()["key_id"], json!(older_s));
    assert_eq!(deleted(&core), vec![newer]);
    assert!(core.sync_duplicates().unwrap().is_empty());
    assert_eq!(core.keys_list().unwrap().len(), 1);
}

/// A merge moves every link to the kept record (UI 13.7; data-model
/// 18.3): a group's hosts and child groups, a host's identity, jump host
/// and startup snippet, a forwarding rule's host. The others go to
/// Recently deleted; the kept ones' fields stay as they are.
#[test]
fn a_merge_moves_every_link_to_the_kept_record() {
    let (_dir, core) = core();
    let group_old = put(&core, RecordKind::Group, json!({"name": "Ops"}));
    let group_new = put(&core, RecordKind::Group, json!({"name": "Ops"}));
    let host_old = put(
        &core,
        RecordKind::Host,
        json!({"address": "dup.example.com", "label": "Web"}),
    );
    let host_new = put(
        &core,
        RecordKind::Host,
        json!({"address": "dup.example.com", "label": "Web (laptop)"}),
    );
    let identity_old = put(
        &core,
        RecordKind::Identity,
        json!({"label": "ops", "username": "ops"}),
    );
    let identity_new = put(
        &core,
        RecordKind::Identity,
        json!({"label": "ops", "username": "ops"}),
    );
    let snippet_old = put(
        &core,
        RecordKind::Snippet,
        json!({"label": "Disk", "script": "df -h"}),
    );
    let snippet_new = put(
        &core,
        RecordKind::Snippet,
        json!({"label": "Disk", "script": "df -h"}),
    );
    let child = put(
        &core,
        RecordKind::Group,
        json!({"name": "Child", "parent_id": group_new.to_string()}),
    );
    let web = put(
        &core,
        RecordKind::Host,
        json!({"address": "web.example.com", "group_id": group_new.to_string(), "ssh": {
            "identity_id": identity_new.to_string(),
            "jump_chain": [host_new.to_string()],
            "startup_snippet_id": snippet_new.to_string(),
        }}),
    );
    let rule = put(
        &core,
        RecordKind::ForwardingRule,
        json!({"type": "local", "host_id": host_new.to_string(), "bind_port": 8080,
            "dest_address": "127.0.0.1", "dest_port": 80}),
    );
    let known = json!({"host": "kh.example.com", "port": 22, "key_type": "ssh-ed25519",
        "public_key": format!("AAAAC3NzaC1lZDI1NTE5AAAA{}", random_text())});
    let known_old = put(&core, RecordKind::KnownHost, known.clone());
    let known_new = put(&core, RecordKind::KnownHost, known);
    assert_eq!(core.sync_duplicates_scan().unwrap(), 5);

    // Identities and known hosts are types `record_delete` does not take.
    for (keep, other) in [
        (group_old, group_new),
        (host_old, host_new),
        (identity_old, identity_new),
        (snippet_old, snippet_new),
        (known_old, known_new),
    ] {
        core.sync_duplicates_merge(keep, other)
            .unwrap_or_else(|error| panic!("{other}: {error:?}"));
    }
    let web = data(&core, web).unwrap();
    assert_eq!(
        web["group_id"],
        json!(group_old.to_string()),
        "a group's hosts"
    );
    assert_eq!(web["ssh"]["identity_id"], json!(identity_old.to_string()));
    assert_eq!(web["ssh"]["jump_chain"], json!([host_old.to_string()]));
    assert_eq!(
        web["ssh"]["startup_snippet_id"],
        json!(snippet_old.to_string())
    );
    assert_eq!(
        data(&core, child).unwrap()["parent_id"],
        json!(group_old.to_string()),
        "a group's child groups"
    );
    assert_eq!(
        data(&core, rule).unwrap()["host_id"],
        json!(host_old.to_string())
    );
    assert_eq!(
        data(&core, host_old).unwrap()["label"],
        json!("Web"),
        "fields are not combined"
    );
    assert!(data(&core, known_old).is_some() && data(&core, known_new).is_none());
    let mut gone = vec![group_new, host_new, identity_new, snippet_new, known_new];
    gone.sort();
    assert_eq!(deleted(&core), gone);
    assert!(core.sync_duplicates().unwrap().is_empty());
}

/// One pull page carrying `envelope`, as the sync cycle applies it.
fn pull(core: &Core, seq: u64, envelope: &[u8]) {
    let now = core.now();
    let mut guard = core.inner();
    let inner = &mut *guard;
    let session = inner.session.as_mut().unwrap();
    let state = SyncStateRow {
        vault_id: session.ring.vault_id(),
        cursor: seq - 1,
        ..SyncStateRow::default()
    };
    engine::apply_page(
        &mut inner.store,
        session,
        &[SeqEnvelope {
            seq,
            envelope: B64(envelope.to_vec()),
        }],
        seq,
        &state,
        &Selection::all(),
        &HashMap::new(),
        now,
    )
    .unwrap();
}

/// A host sealed on "another device" (not stored here): its id and
/// envelope.
fn sealed_host(core: &Core, address: &str) -> (Uuid, Vec<u8>) {
    let now = core.now();
    let mut guard = core.inner();
    let parts = parts(&mut guard).unwrap();
    let id = parts.ids.next(now);
    let write = parts
        .session
        .prepare_write(
            parts.store,
            RecordKind::Host,
            id,
            object(json!({ "address": address })),
            now,
        )
        .unwrap();
    (id, write.row.envelope)
}

/// Drops a record's local row and index entry, so that a pull inserts it
/// again (as a pull that brings it anew).
fn forget(core: &Core, id: Uuid) -> Vec<u8> {
    let mut guard = core.inner();
    let inner = &mut *guard;
    let session = inner.session.as_mut().unwrap();
    let vault_id = session.ring.vault_id();
    let envelope = inner
        .store
        .record(&vault_id, &id)
        .unwrap()
        .unwrap()
        .envelope;
    inner
        .store
        .write(|tx| tx.record_delete(&vault_id, &id))
        .unwrap();
    session.index.remove(&id);
    envelope
}

/// "Keep both" holds when a pull raises the pair again, in either order
/// (data-model 16.8): the pair is not offered again.
#[test]
fn a_kept_pair_stays_kept_when_a_pull_raises_it_again() {
    let (_dir, core) = core();
    let here = put(
        &core,
        RecordKind::Host,
        json!({"address": "same.example.com"}),
    );
    let (pulled, envelope) = sealed_host(&core, "same.example.com");
    pull(&core, 1, &envelope);
    let pairs = core.sync_duplicates().unwrap();
    assert_eq!(pairs.len(), 1, "the pull raised the pair");
    assert_eq!(
        (pairs[0].record_id, pairs[0].other_record_id),
        (here, pulled)
    );
    core.sync_duplicates_keep(here, pulled).unwrap();
    assert!(core.sync_duplicates().unwrap().is_empty());

    // The pulled one arrives again: the same pair.
    let again = forget(&core, pulled);
    pull(&core, 2, &again);
    assert!(data(&core, pulled).is_some());
    assert!(
        core.sync_duplicates().unwrap().is_empty(),
        "kept stays kept"
    );
    // This device's one arrives as an insert: the pair the other way round.
    let again = forget(&core, here);
    pull(&core, 3, &again);
    assert!(data(&core, here).is_some());
    assert!(
        core.sync_duplicates().unwrap().is_empty(),
        "in either order"
    );
    assert_eq!(core.sync_status().unwrap().duplicates, 0);
}
