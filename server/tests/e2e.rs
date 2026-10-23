//! Sync end to end: two (or three) devices of the real client against the
//! real server in this process (docs/sync-protocol.md, section 12, "Sync
//! E2E"; plan run 7, section 4). On SQLite by default, on PostgreSQL with
//! `SHELLPP_TEST_POSTGRES=docker`.

mod support;

use secrecy::SecretString;
use serde_json::{Value, json};
use shellpp_core::CoreError;
use shellpp_core::service::dto::RecordType;
use shellpp_core::service::{ConflictChoice, SignInInput, SignInOutcome};
use shellpp_sync_api::api::Registration;
use support::*;

fn done(outcome: SignInOutcome) {
    assert!(
        matches!(outcome, SignInOutcome::Done(_)),
        "sign-in finished"
    );
}

/// Device A registers with local data; device B restores from sync; edits
/// travel both ways; B's copy opens with the account password.
#[test]
fn register_restore_and_sync_both_ways() {
    let server = Server::start();
    let password = new_password();
    let email = email();
    let a = Client::local("A", &password);
    a.host("a.example.com");
    let status = a.register(&server, &email, &password);
    assert!(status.bound && status.signed_in);
    assert_eq!(status.pending, 0, "the first push sent everything");

    let b = Client::empty("B");
    done(b.sign_in(&server, &email, &password));
    assert_eq!(b.addresses(), vec!["a.example.com"]);
    assert!(
        !b.status().recovery_mismatch,
        "the server's recovery wrap holds the key"
    );

    b.host("b.example.com");
    b.sync();
    a.sync();
    assert_eq!(a.addresses(), vec!["a.example.com", "b.example.com"]);

    // B's local copy opens with the account password after a restart.
    let b = b.reopen("B");
    b.core
        .unlock(password.clone())
        .expect("unlocks with the account password");
    assert_eq!(b.addresses(), vec!["a.example.com", "b.example.com"]);
    // A's copy still opens with the same password too.
    let a = a.reopen("A");
    a.core
        .unlock(password)
        .expect("registration kept the password");
}

/// Registration on an invite-only server needs a code; a wrong password
/// for the local copy is refused before anything is sent.
#[test]
fn invite_only_registration_and_password_check() {
    let server = Server::with(Options {
        registration: Registration::Invite,
        ..Options::default()
    });
    let password = new_password();
    let a = Client::local("A", &password);
    let wrong = a.core.sync_register(SignInInput {
        origin: server.origin.clone(),
        email: email(),
        password: new_password(),
        registration_code: None,
    });
    assert!(matches!(wrong, Err(CoreError::InvalidPassword)));
    let closed = a.core.sync_register(SignInInput {
        origin: server.origin.clone(),
        email: email(),
        password: password.clone(),
        registration_code: None,
    });
    match closed {
        Err(CoreError::Sync(error)) => assert_eq!(error.code(), "registration_closed"),
        other => panic!("expected registration_closed, got {other:?}"),
    }
    let code = shellpp_server::admin::invite(&server.db, 1, 7, None).expect("invite");
    let status = a
        .core
        .sync_register(SignInInput {
            origin: server.origin.clone(),
            email: email(),
            password,
            registration_code: Some(SecretString::from(code)),
        })
        .expect("registered with the code");
    assert!(status.bound);
}

/// Concurrent edits of one host produce one conflict copy, the same on
/// both devices; resolving it on one device clears it on both.
#[test]
fn concurrent_edits_make_one_conflict_copy_on_both_devices() {
    let server = Server::start();
    let password = new_password();
    let email = email();
    let a = Client::local("A", &password);
    let host = a.host("db.example.com");
    a.register(&server, &email, &password);
    let b = Client::empty("B");
    done(b.sign_in(&server, &email, &password));

    a.save(
        Some(host),
        RecordType::Host,
        json!({"address": "db.example.com", "label": "from A"}),
    );
    b.save(
        Some(host),
        RecordType::Host,
        json!({"address": "db.example.com", "label": "from B"}),
    );
    a.sync();
    let report = b.sync();
    assert_eq!(report.conflicts, 1, "B's push meets A's version");
    a.sync();
    b.sync();
    let a_conflicts = a.core.sync_conflicts().expect("conflicts");
    let b_conflicts = b.core.sync_conflicts().expect("conflicts");
    assert_eq!(a_conflicts.len(), 1);
    assert_eq!(b_conflicts.len(), 1);
    assert_eq!(a_conflicts[0].conflict_id, b_conflicts[0].conflict_id);
    assert_eq!(
        a.data(host)["label"],
        b.data(host)["label"],
        "both show the winner"
    );

    // Keep the copy's version on B: A gets it, and the copy is gone on both.
    let losing = b_conflicts[0].losing_data.clone().expect("losing data");
    b.core
        .sync_conflict_resolve(b_conflicts[0].conflict_id, ConflictChoice::Copy)
        .expect("resolved");
    b.sync();
    a.sync();
    assert!(a.core.sync_conflicts().expect("conflicts").is_empty());
    assert!(b.core.sync_conflicts().expect("conflicts").is_empty());
    assert_eq!(a.data(host)["label"], losing["label"]);
}

/// Edit versus delete: the edit wins as a conflict copy only when it is
/// the newer stamp; a delete from the other device goes to Recently
/// deleted, and Restore brings the host back under its id on both.
#[test]
fn deletes_go_to_recently_deleted_and_restore_under_the_same_id() {
    let server = Server::start();
    let password = new_password();
    let email = email();
    let a = Client::local("A", &password);
    let host = a.host("web.example.com");
    a.register(&server, &email, &password);
    let b = Client::empty("B");
    done(b.sign_in(&server, &email, &password));

    a.delete(host);
    a.sync();
    b.sync();
    assert!(b.addresses().is_empty(), "the tombstone reached B");
    let deleted = b.core.recently_deleted_list().expect("list");
    assert_eq!(deleted.len(), 1);
    assert_eq!(deleted[0].record_id, host);
    assert!(deleted[0].from_sync);
    assert_eq!(deleted[0].title, "web.example.com");
    // A keeps its own entry for its local deletion.
    assert_eq!(a.core.recently_deleted_list().expect("list").len(), 1);

    let restored = b.core.recently_deleted_restore(&[host]).expect("restored");
    assert_eq!(restored, vec![host]);
    b.sync();
    a.sync();
    assert_eq!(a.addresses(), vec!["web.example.com"]);
    assert_eq!(b.addresses(), vec!["web.example.com"]);
    assert!(b.core.recently_deleted_list().expect("list").is_empty());
    // A's entry went away when the record came back.
    assert!(a.core.recently_deleted_list().expect("list").is_empty());

    // Edit on B while A deletes: one of them wins, nothing is lost.
    a.delete(host);
    b.save(
        Some(host),
        RecordType::Host,
        json!({"address": "web.example.com", "label": "edited"}),
    );
    a.sync();
    b.sync();
    a.sync();
    let live_a = a.addresses();
    let live_b = b.addresses();
    assert_eq!(live_a, live_b, "both devices agree");
    let conflicts = b.core.sync_conflicts().expect("conflicts").len()
        + b.core.recently_deleted_list().expect("list").len();
    assert!(
        conflicts >= 1,
        "the losing side is kept as a copy or in Recently deleted"
    );
}

/// Sign-in on a device with its own data merges it (data-model 18.5):
/// new record ids, a host with an account host's natural key is dropped and
/// references move to the account's record; nothing local is lost.
#[test]
fn sign_in_with_local_data_merges_without_losing_anything() {
    let server = Server::start();
    let password = new_password();
    let email = email();
    let a = Client::local("A", &password);
    a.host("shared.example.com");
    a.register(&server, &email, &password);

    let b_password = new_password();
    let b = Client::local("B", &b_password);
    let group = b.save(None, RecordType::Group, json!({"name": "Lab"}));
    let local_host = b.save(
        None,
        RecordType::Host,
        json!({"address": "lab.example.com", "group_id": group.to_string()}),
    );
    b.host("shared.example.com");
    let old_vault = b.vault_id();
    done(b.sign_in(&server, &email, &password));
    assert_ne!(
        b.vault_id(),
        old_vault,
        "the account's vault replaced the local one"
    );
    assert_eq!(b.addresses(), vec!["lab.example.com", "shared.example.com"]);
    // New ids, and the group reference follows the group's new id.
    let views = b.views(&[RecordType::Group]);
    assert_eq!(views.len(), 1);
    let new_group = views[0].record_id();
    assert_ne!(new_group, group);
    let lab = b
        .views(&[RecordType::Host])
        .into_iter()
        .find(|view| matches!(view, shellpp_core::service::dto::RecordView::Host(h) if h.data["address"] == "lab.example.com"))
        .expect("lab host");
    assert_ne!(lab.record_id(), local_host);
    match lab {
        shellpp_core::service::dto::RecordView::Host(host) => {
            assert_eq!(host.data["group_id"], Value::String(new_group.to_string()));
        }
        _ => unreachable!(),
    }
    b.sync();
    a.sync();
    assert_eq!(a.addresses(), vec!["lab.example.com", "shared.example.com"]);
    // B's local copy now opens with the account password.
    let b = b.reopen("B");
    b.core.unlock(password).expect("account password");
    assert_eq!(b.addresses(), vec!["lab.example.com", "shared.example.com"]);
    // The pre-merge backup exists.
    let backups = std::fs::read_dir(b.dir.path().join("data").join("backups"))
        .map(|dir| dir.count())
        .unwrap_or(0);
    assert!(backups >= 1, "a backup was written before the merge");
}

/// Log out keeping the local copy, change things offline, sign in again:
/// a rebind with a full resync; the server gets the offline changes.
#[test]
fn sign_out_keep_local_then_rebind() {
    let server = Server::start();
    let password = new_password();
    let email = email();
    let a = Client::local("A", &password);
    a.host("one.example.com");
    a.register(&server, &email, &password);
    let vault = a.vault_id();
    a.core.sync_sign_out(false).expect("signed out");
    assert!(!a.status().bound);
    assert_eq!(
        a.addresses(),
        vec!["one.example.com"],
        "the local copy stays"
    );
    a.host("offline.example.com");
    done(a.sign_in(&server, &email, &password));
    assert_eq!(a.vault_id(), vault, "rebind keeps the vault");
    a.sync();
    let b = Client::empty("B");
    done(b.sign_in(&server, &email, &password));
    assert_eq!(
        b.addresses(),
        vec!["offline.example.com", "one.example.com"]
    );
}

/// A password change on A signs B out; B signs in again with the new
/// password (a rebind) and both keep syncing.
#[test]
fn password_change_signs_the_other_device_out() {
    let server = Server::start();
    let password = new_password();
    let email = email();
    let a = Client::local("A", &password);
    a.host("p.example.com");
    a.register(&server, &email, &password);
    let b = Client::empty("B");
    done(b.sign_in(&server, &email, &password));

    let new = new_password();
    a.core
        .password_change(password.clone(), new.clone())
        .expect("changed");
    a.host("after.example.com");
    a.sync();
    let result = b.core.sync_now();
    assert!(result.is_err(), "B's refresh token was revoked");
    assert!(!b.status().signed_in);
    assert!(b.status().bound, "still bound, signed out");

    done(b.sign_in(&server, &email, &new));
    assert_eq!(b.addresses(), vec!["after.example.com", "p.example.com"]);
    // A opens with the new password.
    let a = a.reopen("A");
    a.core.unlock(new).expect("new password");
}

/// TOTP: enabling it makes the next sign-in ask for a code; a wrong code
/// keeps the ticket, the right one finishes.
#[test]
fn totp_sign_in() {
    let server = Server::start();
    let password = new_password();
    let email = email();
    let a = Client::local("A", &password);
    a.register(&server, &email, &password);
    let setup = a.core.sync_totp_setup(password.clone()).expect("setup");
    let secret = secrecy::ExposeSecret::expose_secret(&setup.secret).to_owned();
    let key = totp_key(&secret);
    a.core
        .sync_totp_enable(&shellpp_server::secrets::totp_code(&key, totp_now()))
        .expect("enabled");

    let b = Client::empty("B");
    let outcome = b.sign_in(&server, &email, &password);
    assert_eq!(outcome, SignInOutcome::TotpRequired);
    // A wrong code keeps the ticket; the right one (the next step: the
    // current one was used by the enable) finishes.
    let right = shellpp_server::secrets::totp_code(&key, totp_now() + 30);
    let wrong = if right == "000000" {
        "111111"
    } else {
        "000000"
    };
    assert!(b.core.sync_sign_in_totp(wrong).is_err());
    done(b.core.sync_sign_in_totp(&right).expect("the right code"));
    assert!(b.status().signed_in);
}

fn totp_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("time")
        .as_secs()
}

fn totp_key(secret: &str) -> [u8; 20] {
    let bytes = data_encoding::BASE32_NOPAD
        .decode(secret.trim_end_matches('=').as_bytes())
        .expect("base32 secret");
    bytes.try_into().expect("20 bytes")
}

/// Deleting the account on A: B learns `account_deleted` and keeps its
/// local copy, unbound.
#[test]
fn account_deletion_keeps_local_copies() {
    let server = Server::start();
    let password = new_password();
    let email = email();
    let a = Client::local("A", &password);
    a.host("keep.example.com");
    a.register(&server, &email, &password);
    let b = Client::empty("B");
    done(b.sign_in(&server, &email, &password));
    a.core
        .sync_account_delete(password.clone(), None)
        .expect("deleted");
    assert!(!a.status().bound);
    let _ = b.core.sync_now();
    assert!(!b.status().bound, "B dropped the account");
    assert_eq!(b.addresses(), vec!["keep.example.com"]);
    assert_eq!(a.addresses(), vec!["keep.example.com"]);
}

/// Revoking B from A's device list signs B out.
#[test]
fn device_revoke_signs_that_device_out() {
    let server = Server::start();
    let password = new_password();
    let email = email();
    let a = Client::local("A", &password);
    a.register(&server, &email, &password);
    let b = Client::empty("B");
    done(b.sign_in(&server, &email, &password));
    let devices = a.core.sync_devices().expect("devices");
    assert_eq!(devices.len(), 2);
    let other = devices.iter().find(|device| !device.current).expect("B");
    a.core.sync_device_revoke(other.device_id).expect("revoked");
    let _ = b.core.sync_now();
    assert!(!b.status().signed_in);
}

/// The same host created offline on both devices is a duplicate pair;
/// Merge moves references to the kept one and deletes the other.
#[test]
fn duplicates_from_two_devices_merge() {
    let server = Server::start();
    let password = new_password();
    let email = email();
    let a = Client::local("A", &password);
    a.register(&server, &email, &password);
    let b = Client::empty("B");
    done(b.sign_in(&server, &email, &password));
    let on_a = a.host("dup.example.com");
    let on_b = b.host("dup.example.com");
    let group = b.save(None, RecordType::Group, json!({"name": "G"}));
    let _ = group;
    a.sync();
    b.sync();
    let pairs = b.core.sync_duplicates().expect("pairs");
    assert_eq!(
        pairs.len(),
        1,
        "B's index had its own host when A's arrived"
    );
    let pair = &pairs[0];
    assert!(
        (pair.record_id == on_a || pair.record_id == on_b)
            && (pair.other_record_id == on_a || pair.other_record_id == on_b)
    );
    b.core.sync_duplicates_merge(on_b, on_a).expect("merged");
    b.sync();
    a.sync();
    assert_eq!(a.addresses(), vec!["dup.example.com"]);
    assert_eq!(b.addresses(), vec!["dup.example.com"]);
    assert!(b.core.sync_duplicates().expect("pairs").is_empty());
}

/// A server quota: the push past it leaves the records pending, counted
/// as refused.
#[test]
fn quota_exceeded_keeps_records_pending() {
    let server = Server::with(Options {
        quota_max_records: Some(5),
        ..Options::default()
    });
    let password = new_password();
    let email = email();
    let a = Client::local("A", &password);
    a.register(&server, &email, &password);
    for i in 0..6 {
        a.host(&format!("q{i}.example.com"));
    }
    let report = a.sync();
    assert!(report.failed > 0, "some items were refused");
    let status = a.status();
    assert!(status.pending > 0, "refused records stay pending");
    assert!(status.failed > 0);
}

/// A server restored from a backup (`admin restored`): the device pushes
/// back what the server lost; no conflict copies (SQLite file copy).
#[test]
fn server_restore_gets_lost_records_back() {
    if on_postgres() {
        return; // The backup here is a file copy of the SQLite database.
    }
    let mut server = Server::start();
    let password = new_password();
    let email = email();
    let a = Client::local("A", &password);
    a.host("before.example.com");
    a.register(&server, &email, &password);
    let db_path = server.dir.path().join("server.db");
    let backup = server.dir.path().join("backup.db");
    server
        .db
        .batch(&format!("VACUUM INTO '{}'", backup.display()))
        .expect("backup");
    a.host("after.example.com");
    a.sync();
    // The restore: the server stops and its file is replaced by the backup.
    server.stop();
    server.db = shellpp_server::db::Db::open(&format!("sqlite:{}", backup.display())).expect("db");
    for suffix in ["", "-wal", "-shm"] {
        let mut name = db_path.clone().into_os_string();
        name.push(suffix);
        let _ = std::fs::remove_file(std::path::PathBuf::from(name));
    }
    std::fs::copy(&backup, &db_path).expect("restore");
    server.resume();
    shellpp_server::admin::restored(&server.db).expect("restored");
    let report = a.sync();
    assert!(report.restored, "the new generation started the restore");
    assert_eq!(report.conflicts, 0);
    let b = Client::empty("B");
    done(b.sign_in(&server, &email, &password));
    assert_eq!(
        b.addresses(),
        vec!["after.example.com", "before.example.com"]
    );
}
