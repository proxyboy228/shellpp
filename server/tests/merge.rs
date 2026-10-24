//! The merge at sign-in on a device with local data (docs/data-model.md,
//! sections 18.5 and 18.6; docs/storage-format.md, sections 6.6 and 8.4):
//! what moves into the account's personal vault, and the recordings that
//! follow it (run 9).

mod support;

use std::path::{Path, PathBuf};

use secrecy::SecretString;
use serde_json::json;
use shellpp_core::recording::{self, Header};
use shellpp_core::service::SignInOutcome;
use shellpp_core::service::dto::{ConnectionLogView, RecordType};
use shellpp_core::service::host_api::ConnectionLogStart;
use shellpp_core::service::snippets::RecordInput;
use support::*;
use uuid::Uuid;

fn done(outcome: SignInOutcome) {
    assert!(matches!(outcome, SignInOutcome::Done(_)), "signed in");
}

fn recordings_dir(client: &Client) -> PathBuf {
    client.dir.path().join("data").join(recording::DIR_NAME)
}

fn recording_path(client: &Client, file_id: Uuid) -> PathBuf {
    recordings_dir(client).join(format!("{file_id}.{}", recording::EXTENSION))
}

fn header_of(path: &Path) -> Header {
    Header::parse(&std::fs::read(path).expect("recording file")).expect("header")
}

/// Records a session to `address` with `output` and returns its file id.
fn record(client: &Client, address: &str, output: &[&str]) -> Uuid {
    let log_id = client
        .core
        .connection_log_start(ConnectionLogStart {
            host: None,
            host_label: None,
            address: address.to_owned(),
            port: 22,
            username: None,
            os: None,
        })
        .expect("log entry");
    let mut writer = client
        .core
        .recording_start(log_id, (120, 40))
        .expect("recording");
    for (index, text) in output.iter().enumerate() {
        writer
            .output_at(index as f64, text.as_bytes())
            .expect("output");
    }
    let file_id = writer.file_id();
    writer.finish().expect("closed");
    file_id
}

fn log_entry(client: &Client, address: &str) -> ConnectionLogView {
    client
        .core
        .connection_log_list()
        .expect("log")
        .into_iter()
        .find(|row| row.address == address)
        .expect("log entry")
}

/// The text, events and completeness of the recording of `address`.
fn contents(client: &Client, address: &str) -> (String, usize, bool, (u16, u16)) {
    let entry = log_entry(client, address);
    let read = client
        .core
        .recording_read(entry.record_id)
        .expect("recording opens")
        .expect("recorded");
    (
        read.plain_text(),
        read.events.len(),
        read.complete,
        read.size,
    )
}

fn rekey_files(client: &Client) -> usize {
    std::fs::read_dir(recordings_dir(client))
        .expect("dir")
        .flatten()
        .filter(|entry| entry.path().extension().and_then(|ext| ext.to_str()) == Some("rekey"))
        .count()
}

/// Run 8, agent C, finding 5 (storage-format.md 8.4): the re-encryption of
/// the recordings at sign-in loses none of them when it fails half-way,
/// keeps every byte of the plaintext (the asciicast header, the events,
/// and the missing final chunk of a file that was never closed), and
/// continues at the next start.
#[test]
fn recordings_survive_a_failed_re_encryption_and_finish_at_the_next_start() {
    let server = Server::start();
    let password = new_password();
    let email = email();
    let a = Client::local("A", &password);
    a.register(&server, &email, &password);

    let local_password: SecretString = new_password();
    let l = Client::local("L", &local_password);
    l.host("local.example.com");
    // One recording of more than one chunk, cut inside its second chunk as
    // a full disk leaves it (a torn fragment, no final chunk: "incomplete").
    let long_line = "x".repeat(1000);
    let long: Vec<&str> = (0..100).map(|_| long_line.as_str()).collect();
    let cut = record(&l, "cut.example.com", &long);
    let cut_path = recording_path(&l, cut);
    let frame = recording::CHUNK as u64 + 16;
    let file = std::fs::OpenOptions::new()
        .write(true)
        .open(&cut_path)
        .expect("open");
    file.set_len(recording::HEADER_LEN as u64 + frame + 100)
        .expect("cut");
    drop(file);
    let small = record(
        &l,
        "small.example.com",
        &["hello\r\n", "\x1b[1mworld\x1b[0m\r\n"],
    );
    let before_cut = contents(&l, "cut.example.com");
    let before_small = contents(&l, "small.example.com");
    assert!(!before_cut.2, "the cut recording is incomplete");
    assert!(before_small.2);
    let old_vault = header_of(&cut_path).vault_id;

    // A failure in the middle: the cut recording cannot be written (as with
    // a full disk), the other one can.
    recording::fail_rekey::once(cut);
    done(l.sign_in(&server, &email, &password));
    let account_vault = l.vault_id();
    assert_ne!(account_vault, old_vault);
    assert_eq!(
        header_of(&recording_path(&l, small)).vault_id,
        account_vault
    );
    assert_eq!(
        header_of(&cut_path).vault_id,
        old_vault,
        "not re-encrypted yet"
    );
    // Nothing is lost meanwhile: both open, with every event.
    assert_eq!(contents(&l, "cut.example.com"), before_cut);
    assert_eq!(contents(&l, "small.example.com"), before_small);
    assert_eq!(rekey_files(&l), 1);

    // The next start finishes it.
    let l = l.reopen("L");
    l.core.unlock(password.clone()).expect("unlocked");
    assert_eq!(header_of(&cut_path).vault_id, account_vault);
    assert_eq!(rekey_files(&l), 0);
    assert_eq!(contents(&l, "cut.example.com"), before_cut);
    assert_eq!(contents(&l, "small.example.com"), before_small);
    let leftovers: Vec<PathBuf> = std::fs::read_dir(recordings_dir(&l))
        .expect("dir")
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) != Some("shpprec"))
        .collect();
    assert!(leftovers.is_empty(), "no temporary files: {leftovers:?}");
}

/// A recording still being written at sign-in keeps its key until it is
/// closed; the next start re-encrypts it, with the output that came after
/// the sign-in.
#[test]
fn a_recording_written_during_the_sign_in_is_re_encrypted_later() {
    let server = Server::start();
    let password = new_password();
    let email = email();
    let a = Client::local("A", &password);
    a.register(&server, &email, &password);

    let l = Client::local("L", &new_password());
    let log_id = l
        .core
        .connection_log_start(ConnectionLogStart {
            host: None,
            host_label: None,
            address: "open.example.com".to_owned(),
            port: 22,
            username: None,
            os: None,
        })
        .expect("log entry");
    let mut writer = l.core.recording_start(log_id, (80, 24)).expect("recording");
    writer
        .output_at(0.5, b"before sign-in\r\n")
        .expect("output");
    let path = recording_path(&l, writer.file_id());
    done(l.sign_in(&server, &email, &password));
    writer.output_at(1.5, b"after sign-in\r\n").expect("output");
    writer.finish().expect("closed");
    let expected = "before sign-in\nafter sign-in\n";
    assert_eq!(contents(&l, "open.example.com").0, expected);

    let l = l.reopen("L");
    l.core.unlock(password.clone()).expect("unlocked");
    assert_eq!(header_of(&path).vault_id, l.vault_id());
    assert_eq!(rekey_files(&l), 0);
    let (text, events, complete, _) = contents(&l, "open.example.com");
    assert_eq!((text.as_str(), events, complete), (expected, 2, true));
}

fn package(client: &Client, name: &str) -> Uuid {
    client
        .core
        .snippet_package_save(RecordInput {
            vault_id: client.vault_id(),
            record_id: None,
            data: json!({ "name": name })
                .as_object()
                .cloned()
                .expect("object"),
        })
        .expect("package")
        .record
        .record_id
}

fn snippet(client: &Client, package_id: Uuid, label: &str) {
    client
        .core
        .snippet_save(RecordInput {
            vault_id: client.vault_id(),
            record_id: None,
            data: json!({
                "package_id": package_id.to_string(),
                "label": label,
                "script": "systemctl restart app",
            })
            .as_object()
            .cloned()
            .expect("object"),
        })
        .expect("snippet");
}

fn count(client: &Client, record_type: RecordType) -> usize {
    client.views(&[record_type]).len()
}

/// Run 7 finding 6: a local record compared before the record it names was
/// matched (here a host whose row comes before its group's) is still found
/// equal once its reference is rewritten: the merge leaves no extra pair.
#[test]
fn the_merge_compares_after_the_references_are_rewritten() {
    let server = Server::start();
    let password = new_password();
    let email = email();
    let a = Client::local("A", &password);
    a.register(&server, &email, &password);
    let prod = a.save(None, RecordType::Group, json!({"name": "Prod"}));
    a.save(
        None,
        RecordType::Host,
        json!({"address": "db.example.com", "group_id": prod.to_string()}),
    );
    a.sync();

    let l = Client::local("L", &new_password());
    // The host first, so its row comes before the group's.
    let host = l.host("db.example.com");
    let group = l.save(None, RecordType::Group, json!({"name": "Prod"}));
    let mut data = l.data(host);
    data["group_id"] = json!(group.to_string());
    l.save(Some(host), RecordType::Host, data);
    done(l.sign_in(&server, &email, &password));

    assert_eq!(count(&l, RecordType::Group), 1, "the same group once");
    assert_eq!(count(&l, RecordType::Host), 1, "the same host once");
    assert!(
        l.core.sync_duplicates().expect("pairs").is_empty(),
        "no extra pair"
    );
}

/// Run 7 finding 16 (data-model 18.6): natural keys compare a host's
/// effective port (inherited from its group) and parents by their natural
/// keys, not by record ids: a nested group, a snippet in a package and a
/// host that only share the key with the account's are matched.
#[test]
fn natural_keys_follow_inheritance_and_parents() {
    let server = Server::start();
    let password = new_password();
    let email = email();
    let a = Client::local("A", &password);
    a.register(&server, &email, &password);
    let prod = a.save(
        None,
        RecordType::Group,
        json!({"name": "Prod", "ssh": {"port": 2222}}),
    );
    a.save(
        None,
        RecordType::Group,
        json!({"name": "Web", "parent_id": prod.to_string()}),
    );
    a.save(
        None,
        RecordType::Host,
        json!({"address": "db.example.com", "label": "Account", "group_id": prod.to_string()}),
    );
    let ops = package(&a, "Ops");
    snippet(&a, ops, "restart");
    a.sync();

    let l = Client::local("L", &new_password());
    let local_prod = l.save(
        None,
        RecordType::Group,
        json!({"name": "Prod", "ssh": {"port": 2222}}),
    );
    l.save(
        None,
        RecordType::Group,
        json!({"name": "Web", "parent_id": local_prod.to_string()}),
    );
    // The same host by its effective port, here set on the host itself.
    l.save(
        None,
        RecordType::Host,
        json!({"address": "db.example.com", "label": "Local", "ssh": {"port": 2222}}),
    );
    let local_ops = package(&l, "Ops");
    snippet(&l, local_ops, "restart");
    done(l.sign_in(&server, &email, &password));

    assert_eq!(count(&l, RecordType::Group), 2, "Prod and Web once each");
    let packages = l
        .core
        .snippet_packages_list(l.vault_id())
        .expect("packages");
    assert_eq!(packages.len(), 1, "the same package once");
    let snippets = l.core.snippets_list(l.vault_id()).expect("snippets");
    assert_eq!(snippets.len(), 1, "the same snippet once");
    // The two hosts differ (labels): both kept, raised as one pair.
    assert_eq!(count(&l, RecordType::Host), 2);
    let pairs = l.core.sync_duplicates().expect("pairs");
    assert_eq!(pairs.len(), 1, "{pairs:?}");
    assert_eq!(pairs[0].type_name, "host");
}

/// Review of run 9: references in a cycle (a group whose jump host is in
/// the group) and two account records with one natural key (the same host
/// in two groups) still leave no extra copy.
#[test]
fn the_merge_matches_cycles_and_every_account_twin() {
    let server = Server::start();
    let password = new_password();
    let email = email();
    let a = Client::local("A", &password);
    a.register(&server, &email, &password);
    let build = |client: &Client| {
        let prod = client.save(None, RecordType::Group, json!({"name": "Prod"}));
        let old = client.save(None, RecordType::Group, json!({"name": "Old"}));
        let bastion = client.save(
            None,
            RecordType::Host,
            json!({"address": "bastion.example.com", "group_id": prod.to_string()}),
        );
        let mut data = client.data(prod);
        data["ssh"] = json!({"jump_chain": [bastion.to_string()]});
        client.save(Some(prod), RecordType::Group, data);
        [prod, old].map(|group| {
            client.save(
                None,
                RecordType::Host,
                json!({"address": "db.example.com", "group_id": group.to_string()}),
            )
        })
    };
    let mut account_db = build(&a);
    a.sync();
    let l = Client::local("L", &new_password());
    build(&l);
    done(l.sign_in(&server, &email, &password));
    assert_eq!(count(&l, RecordType::Group), 2);
    assert_eq!(count(&l, RecordType::Host), 3);
    // Only the account's own pair (one host in two groups) is listed.
    let mut pairs: Vec<[Uuid; 2]> = l
        .core
        .sync_duplicates()
        .expect("pairs")
        .iter()
        .map(|pair| {
            let mut ids = [pair.record_id, pair.other_record_id];
            ids.sort();
            ids
        })
        .collect();
    account_db.sort();
    pairs.dedup();
    assert_eq!(pairs, vec![account_db]);
}

/// Run 7 finding 18 (data-model 18.5): a merge that stops half-way leaves
/// the account vault it began; giving up the sign-in removes it and only
/// it, and a later sign-in merges as usual.
#[test]
fn an_abandoned_merge_leaves_no_extra_vault() {
    let server = Server::start();
    let password = new_password();
    let email = email();
    let a = Client::local("A", &password);
    a.host("account.example.com");
    a.register(&server, &email, &password);

    let l = Client::local("L", &new_password());
    l.host("local.example.com");
    let local_vault = l.vault_id();
    l.core.sync_merge_fail_once_for_test();
    let failed = l.core.sync_sign_in(shellpp_core::service::SignInInput {
        origin: server.origin.clone(),
        email: email.clone(),
        password: password.clone(),
        registration_code: None,
    });
    assert!(failed.is_err(), "the merge stopped");
    let rows = l.core.vault_rows_for_test().expect("vaults");
    assert_eq!(rows.len(), 2, "the begun account vault: {rows:?}");
    assert_eq!(l.vault_id(), local_vault);

    l.core.sync_sign_in_cancel();
    let rows = l.core.vault_rows_for_test().expect("vaults");
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0].0, local_vault);
    assert_eq!(
        l.addresses(),
        vec!["local.example.com"],
        "local data intact"
    );

    done(l.sign_in(&server, &email, &password));
    assert_eq!(
        l.addresses(),
        vec!["account.example.com", "local.example.com"]
    );
    assert_eq!(l.core.vault_rows_for_test().expect("vaults").len(), 1);
}

/// Cross review of run 9: a `.rekey` file may be the only key of a
/// recording of a store set aside by "Start with a new vault" (until D197
/// also by "Also delete the local copy", which now deletes the recordings).
/// A merge into the same account vault that is given up later removes its
/// begun vault, never that file.
#[test]
fn giving_up_a_merge_keeps_the_rekey_of_a_store_set_aside() {
    let server = Server::start();
    let password = new_password();
    let email = email();
    let a = Client::local("A", &password);
    a.register(&server, &email, &password);

    let l = Client::local("L", &new_password());
    let file_id = record(&l, "kept.example.com", &["output\r\n"]);
    recording::fail_rekey::once(file_id);
    done(l.sign_in(&server, &email, &password));
    let account_vault = l.vault_id();
    let rekey = recordings_dir(&l).join(format!("{file_id}.{account_vault}.rekey"));
    assert!(rekey.exists(), "the recording still waits for its key");
    l.core.sync_sign_out(false).expect("signed out");
    l.core.lock().expect("locked");
    l.core.start_new_vault().expect("copy set aside");

    // A new store on the same directory merges into the same account vault
    // and gives up half-way.
    l.core.first_run_setup(new_password()).expect("first run");
    l.core.sync_merge_fail_once_for_test();
    assert!(
        l.core
            .sync_sign_in(shellpp_core::service::SignInInput {
                origin: server.origin.clone(),
                email: email.clone(),
                password: password.clone(),
                registration_code: None,
            })
            .is_err()
    );
    l.core.sync_sign_in_cancel();
    assert_eq!(l.core.vault_rows_for_test().expect("vaults").len(), 1);
    assert!(rekey.exists(), "the set-aside store's key is kept");
}

/// Cross review of run 9: a recording that cannot be read when the merge
/// writes its `.rekey` (here held open by another program without sharing)
/// stops the sign-in before anything changed, instead of leaving it under
/// a key the merge deletes.
#[cfg(windows)]
#[test]
fn an_unreadable_recording_stops_the_merge_with_nothing_changed() {
    use std::os::windows::fs::OpenOptionsExt as _;
    let server = Server::start();
    let password = new_password();
    let email = email();
    let a = Client::local("A", &password);
    a.register(&server, &email, &password);

    let l = Client::local("L", &new_password());
    l.host("local.example.com");
    let file_id = record(&l, "held.example.com", &["output\r\n"]);
    let local_vault = l.vault_id();
    let before = contents(&l, "held.example.com");
    let held = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(recording_path(&l, file_id))
        .expect("held open");
    let result = l.core.sync_sign_in(shellpp_core::service::SignInInput {
        origin: server.origin.clone(),
        email: email.clone(),
        password: password.clone(),
        registration_code: None,
    });
    assert!(result.is_err(), "the merge stopped");
    drop(held);
    assert_eq!(l.vault_id(), local_vault, "still the local vault");
    assert_eq!(l.addresses(), vec!["local.example.com"]);
    assert_eq!(contents(&l, "held.example.com"), before);
}

/// Cross review of run 9: two account groups share one natural key, and
/// the device holds the same two. Whichever comes first among the twins,
/// each local group and the host in one of them are matched to their
/// equals: no extra copy, no extra pair.
#[test]
fn a_record_matches_the_right_one_of_several_twins() {
    for equal_first in [false, true] {
        let server = Server::start();
        let password = new_password();
        let email = email();
        let a = Client::local("A", &password);
        a.register(&server, &email, &password);
        let make = |notes: &str| {
            a.save(
                None,
                RecordType::Group,
                json!({"name": "Prod", "notes": notes}),
            )
        };
        let (first, second) = if equal_first {
            (make("two"), make("one"))
        } else {
            (make("one"), make("two"))
        };
        let two = if equal_first { first } else { second };
        a.save(
            None,
            RecordType::Host,
            json!({"address": "db.example.com", "group_id": two.to_string()}),
        );
        a.sync();

        // The device holds the same two groups, made in the same order (so
        // with the same order keys), and the host in "two".
        let l = Client::local("L", &new_password());
        let make_local = |notes: &str| {
            l.save(
                None,
                RecordType::Group,
                json!({"name": "Prod", "notes": notes}),
            )
        };
        let local_prod = if equal_first {
            let two = make_local("two");
            make_local("one");
            two
        } else {
            make_local("one");
            make_local("two")
        };
        assert_eq!(l.data(local_prod)["order_key"], a.data(two)["order_key"]);
        l.save(
            None,
            RecordType::Host,
            json!({"address": "db.example.com", "group_id": local_prod.to_string()}),
        );
        done(l.sign_in(&server, &email, &password));
        assert_eq!(count(&l, RecordType::Group), 2, "{equal_first}");
        assert_eq!(count(&l, RecordType::Host), 1, "{equal_first}");
        let mut account_pair = [first, second];
        account_pair.sort();
        for pair in l.core.sync_duplicates().expect("pairs") {
            let mut ids = [pair.record_id, pair.other_record_id];
            ids.sort();
            assert_eq!(ids, account_pair, "only the account's own pair");
        }
    }
}
