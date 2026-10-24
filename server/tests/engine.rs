//! The sync engine against the real server (docs/sync-protocol.md, sections
//! 4.4 and 6.5-6.9): findings of the run 7 report fixed in run 9.

mod support;

use shellpp_core::service::SignInOutcome;
use shellpp_server::db::{DbError, TxKind, Ty, Val};
use support::*;

fn done(outcome: SignInOutcome) {
    assert!(matches!(outcome, SignInOutcome::Done(_)), "signed in");
}

fn recovery_wrap(server: &Server) -> Vec<u8> {
    server
        .db
        .tx(TxKind::Read, |tx| -> Result<Vec<u8>, DbError> {
            let rows = tx.query("SELECT recovery_wrap FROM users", &[], &[Ty::Bytes])?;
            Ok(
                match rows
                    .into_iter()
                    .next()
                    .and_then(|row| row.into_iter().next())
                {
                    Some(Val::Bytes(bytes)) => bytes,
                    _ => Vec::new(),
                },
            )
        })
        .expect("recovery wrap read")
}

fn set_recovery_wrap(server: &Server, wrap: Vec<u8>) {
    server
        .db
        .tx(TxKind::Write, |tx| -> Result<(), DbError> {
            tx.exec("UPDATE users SET recovery_wrap = ?", &[Val::Bytes(wrap)])?;
            Ok(())
        })
        .expect("recovery wrap written");
}

/// Run 7 finding 17 (section 4.4): the server's `recovery_wrap` is checked
/// in every cycle until the check passes, not only after the first pull:
/// a wrong wrap shows the warning, and once the server holds the right one
/// (whatever replaced it) the next cycle clears it.
#[test]
fn the_recovery_wrap_is_checked_until_it_is_right() {
    let server = Server::start();
    let password = new_password();
    let email = email();
    let a = Client::local("A", &password);
    a.register(&server, &email, &password);
    let right = recovery_wrap(&server);
    let mut wrong = right.clone();
    let last = wrong.len() - 1;
    wrong[last] ^= 1;
    set_recovery_wrap(&server, wrong.clone());

    let b = Client::empty("B");
    done(b.sign_in(&server, &email, &password));
    assert!(b.status().recovery_mismatch, "a wrong wrap is reported");
    b.sync();
    assert!(b.status().recovery_mismatch, "still wrong: still reported");

    set_recovery_wrap(&server, right);
    b.sync();
    assert!(!b.status().recovery_mismatch, "checked again and right now");

    // A device that checked keeps the result until the record or the
    // epoch changes, and checks again after an unlock.
    set_recovery_wrap(&server, wrong);
    b.sync();
    assert!(!b.status().recovery_mismatch, "not fetched again");
    let b = b.reopen("B");
    b.core.unlock(password.clone()).expect("unlocked");
    b.sync();
    assert!(
        b.status().recovery_mismatch,
        "checked again after the unlock"
    );
}

fn live_records(server: &Server) -> u64 {
    server
        .db
        .tx(TxKind::Read, |tx| -> Result<u64, DbError> {
            let rows = tx.query(
                "SELECT record_id FROM records WHERE NOT tombstone",
                &[],
                // A blob on SQLite, a `uuid` on PostgreSQL: the layer reads
                // both as `Uuid`.
                &[Ty::Uuid],
            )?;
            Ok(rows.len() as u64)
        })
        .expect("records counted")
}

/// Run 7 finding 19 (section 6.5): an item refused for the quota is not
/// retried by the automatic cycle, but is pushed in the cycle in which this
/// device's deletion frees room.
#[test]
fn quota_refusals_go_again_after_a_pushed_deletion() {
    let mut server = Server::start();
    let password = new_password();
    let email = email();
    let a = Client::local("A", &password);
    a.register(&server, &email, &password);
    let old = a.host("old.example.com");
    a.sync();
    // The quota is what the account holds now.
    let used = live_records(&server);
    server.stop();
    server.config.quota_max_records = Some(used);
    server.resume();

    a.host("new.example.com");
    let report = a.sync();
    assert_eq!(report.failed, 1, "refused for the quota");
    assert_eq!(a.status().pending, 1);
    let report = a.core.sync_auto().expect("automatic cycle");
    assert_eq!(report.pushed, 0, "not retried in every cycle");
    assert_eq!(a.status().failed, 1);

    a.delete(old);
    let report = a.core.sync_auto().expect("automatic cycle");
    assert_eq!(report.pushed, 2, "the deletion, then the refused host");
    let status = a.status();
    assert_eq!((status.pending, status.failed), (0, 0));
}

/// Replaces the server's database by `backup` (an SQLite file copy) and
/// marks the restore as the operator documentation says.
fn restore(server: &mut Server, backup: &std::path::Path) {
    let db_path = server.dir.path().join("server.db");
    server.stop();
    server.db = shellpp_server::db::Db::open(&format!("sqlite:{}", backup.display())).expect("db");
    for suffix in ["", "-wal", "-shm"] {
        let mut name = db_path.clone().into_os_string();
        name.push(suffix);
        let _ = std::fs::remove_file(std::path::PathBuf::from(name));
    }
    std::fs::copy(backup, &db_path).expect("restore");
    server.resume();
    shellpp_server::admin::restored(&server.db).expect("restored");
}

fn label_of(client: &Client, id: uuid::Uuid) -> String {
    client.data(id)["label"]
        .as_str()
        .unwrap_or_default()
        .to_owned()
}

fn losing_labels(client: &Client) -> Vec<String> {
    let mut labels: Vec<String> = client
        .core
        .sync_conflicts()
        .expect("conflicts")
        .iter()
        .filter_map(|conflict| {
            conflict
                .losing_data
                .as_ref()
                .and_then(|data| data.get("label"))
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
        })
        .collect();
    labels.sort();
    labels.dedup();
    labels
}

/// Run 7 finding 11 (D173, item 1.11; sections 6.7 and 6.9): a device back
/// after the server was restored from an older backup pushes its newer
/// versions over the edits another device made on the restored server at
/// a revision not above its own. Those edits are never lost: each is kept
/// as a conflict copy, on both devices.
#[test]
fn edits_on_a_restored_server_are_kept_as_conflicts() {
    if on_postgres() {
        return; // The backup here is a file copy of the SQLite database.
    }
    let mut server = Server::start();
    let password = new_password();
    let email = email();
    let a = Client::local("A", &password);
    let once = a.save(
        None,
        shellpp_core::service::dto::RecordType::Host,
        serde_json::json!({"address": "once.example.com", "label": "first"}),
    );
    let twice = a.save(
        None,
        shellpp_core::service::dto::RecordType::Host,
        serde_json::json!({"address": "twice.example.com", "label": "first"}),
    );
    a.register(&server, &email, &password);
    let backup = server.dir.path().join("backup.db");
    server
        .db
        .batch(&format!("VACUUM INTO '{}'", backup.display()))
        .expect("backup");
    // A's edits after the backup: one on `once`, two on `twice`.
    let edit = |client: &Client, id: uuid::Uuid, label: &str| {
        let mut data = client.data(id);
        data["label"] = serde_json::Value::String(label.into());
        client.save(Some(id), shellpp_core::service::dto::RecordType::Host, data);
    };
    edit(&a, once, "A after the backup");
    edit(&a, twice, "A once");
    edit(&a, twice, "A after the backup");
    a.sync();
    a.core.sync_sign_out(false).expect("signed out");
    restore(&mut server, &backup);

    // B works on the restored server: its edits get revisions A has
    // already synced (equal for `once`, below for `twice`).
    let b = Client::empty("B");
    done(b.sign_in(&server, &email, &password));
    edit(&b, once, "B on the restored server");
    edit(&b, twice, "B on the restored server");
    b.sync();

    done(a.sign_in(&server, &email, &password));
    a.sync();
    assert_eq!(label_of(&a, once), "A after the backup");
    assert_eq!(label_of(&a, twice), "A after the backup");
    assert_eq!(losing_labels(&a), vec!["B on the restored server"]);
    assert_eq!(a.core.sync_conflicts().expect("conflicts").len(), 2);

    b.sync();
    assert_eq!(label_of(&b, once), "A after the backup");
    assert_eq!(label_of(&b, twice), "A after the backup");
    assert_eq!(losing_labels(&b), vec!["B on the restored server"]);
}

/// Review of run 9: an edit made earlier but pushed only after the restore
/// lands at the revision the returning device synced. Its stamp is lower,
/// yet it is another device's write at that revision: kept as a copy too,
/// and only once however often the device resyncs.
#[test]
fn an_older_offline_edit_pushed_after_a_restore_is_kept() {
    if on_postgres() {
        return; // The backup here is a file copy of the SQLite database.
    }
    let mut server = Server::start();
    let password = new_password();
    let email = email();
    let a = Client::local("A", &password);
    let host = a.save(
        None,
        shellpp_core::service::dto::RecordType::Host,
        serde_json::json!({"address": "early.example.com", "label": "first"}),
    );
    a.register(&server, &email, &password);
    let b = Client::empty("B");
    done(b.sign_in(&server, &email, &password));
    let backup = server.dir.path().join("backup.db");
    server
        .db
        .batch(&format!("VACUUM INTO '{}'", backup.display()))
        .expect("backup");
    let edit = |client: &Client, label: &str| {
        let mut data = client.data(host);
        data["label"] = serde_json::Value::String(label.into());
        client.save(
            Some(host),
            shellpp_core::service::dto::RecordType::Host,
            data,
        );
    };
    // B edits first, offline; A edits later and syncs.
    edit(&b, "B offline, earlier");
    edit(&a, "A after the backup");
    a.sync();
    a.core.sync_sign_out(false).expect("signed out");
    restore(&mut server, &backup);
    // B learns the restore and pushes its older edit at the same revision.
    b.sync();
    done(a.sign_in(&server, &email, &password));
    a.sync();
    a.sync();
    assert_eq!(label_of(&a, host), "A after the backup");
    assert_eq!(losing_labels(&a), vec!["B offline, earlier"]);
    assert_eq!(a.core.sync_conflicts().expect("conflicts").len(), 1, "once");
    b.sync();
    assert!(losing_labels(&b).contains(&"B offline, earlier".to_owned()));
}

/// Cross review of run 9 (D184): after a restore the device notices (a new
/// server generation), another device's edits on the restored server
/// reach a revision above the one this device synced, so they replace its
/// version. The replaced version, which the restored server lost, is kept
/// as a conflict copy.
#[test]
fn a_version_replaced_in_a_restore_is_kept_as_a_conflict() {
    if on_postgres() {
        return; // The backup here is a file copy of the SQLite database.
    }
    let mut server = Server::start();
    let password = new_password();
    let email = email();
    let a = Client::local("A", &password);
    let host = a.save(
        None,
        shellpp_core::service::dto::RecordType::Host,
        serde_json::json!({"address": "replaced.example.com", "label": "first"}),
    );
    a.register(&server, &email, &password);
    let backup = server.dir.path().join("backup.db");
    server
        .db
        .batch(&format!("VACUUM INTO '{}'", backup.display()))
        .expect("backup");
    let edit = |client: &Client, label: &str| {
        let mut data = client.data(host);
        data["label"] = serde_json::Value::String(label.into());
        client.save(
            Some(host),
            shellpp_core::service::dto::RecordType::Host,
            data,
        );
    };
    edit(&a, "A once");
    edit(&a, "A after the backup");
    a.sync();
    restore(&mut server, &backup);
    let b = Client::empty("B");
    done(b.sign_in(&server, &email, &password));
    for label in ["B one", "B two", "B three"] {
        edit(&b, label);
    }
    b.sync();
    let report = a.sync();
    assert!(report.restored, "A noticed the restore");
    assert_eq!(label_of(&a, host), "B three");
    assert_eq!(losing_labels(&a), vec!["A after the backup"]);
    b.sync();
    assert_eq!(losing_labels(&b), vec!["A after the backup"]);
}

/// D184, residual 2 (D197): the restore happened while the device was out
/// of the account. Its rebind compares the generation it kept at sign-out
/// with the server's, runs the restore, and keeps the version another
/// device's newer edits on the restored server replace.
#[test]
fn a_version_replaced_at_a_rebind_after_a_restore_is_kept() {
    if on_postgres() {
        return; // The backup here is a file copy of the SQLite database.
    }
    let mut server = Server::start();
    let password = new_password();
    let email = email();
    let a = Client::local("A", &password);
    let host = a.save(
        None,
        shellpp_core::service::dto::RecordType::Host,
        serde_json::json!({"address": "rebind.example.com", "label": "first"}),
    );
    a.register(&server, &email, &password);
    let backup = server.dir.path().join("backup.db");
    server
        .db
        .batch(&format!("VACUUM INTO '{}'", backup.display()))
        .expect("backup");
    let edit = |client: &Client, label: &str| {
        let mut data = client.data(host);
        data["label"] = serde_json::Value::String(label.into());
        client.save(
            Some(host),
            shellpp_core::service::dto::RecordType::Host,
            data,
        );
    };
    edit(&a, "A once");
    edit(&a, "A after the backup");
    a.sync();
    a.core.sync_sign_out(false).expect("signed out");
    restore(&mut server, &backup);
    let b = Client::empty("B");
    done(b.sign_in(&server, &email, &password));
    for label in ["B one", "B two", "B three"] {
        edit(&b, label);
    }
    b.sync();
    done(a.sign_in(&server, &email, &password));
    a.sync();
    assert_eq!(label_of(&a, host), "B three");
    assert_eq!(losing_labels(&a), vec!["A after the backup"]);
    b.sync();
    assert_eq!(losing_labels(&b), vec!["A after the backup"]);
}

/// D197: without a restore, the rebind finds the generation it kept at
/// sign-out unchanged and takes the other device's newer versions without
/// copies.
#[test]
fn a_rebind_without_a_restore_makes_no_copies() {
    let server = Server::start();
    let password = new_password();
    let email = email();
    let a = Client::local("A", &password);
    let host = a.save(
        None,
        shellpp_core::service::dto::RecordType::Host,
        serde_json::json!({"address": "behind.example.com", "label": "first"}),
    );
    a.register(&server, &email, &password);
    let b = Client::empty("B");
    done(b.sign_in(&server, &email, &password));
    a.core.sync_sign_out(false).expect("signed out");
    for label in ["B one", "B two"] {
        let mut data = b.data(host);
        data["label"] = serde_json::Value::String(label.into());
        b.save(
            Some(host),
            shellpp_core::service::dto::RecordType::Host,
            data,
        );
    }
    b.sync();
    done(a.sign_in(&server, &email, &password));
    a.sync();
    assert_eq!(label_of(&a, host), "B two");
    assert!(a.core.sync_conflicts().expect("conflicts").is_empty());
}

/// D197, the fallback: a rebind that does not know the generation of
/// before (a sign-out made by an older version keeps none) cannot tell a
/// restore from edits made meanwhile, so every version a newer server
/// version replaces is kept as a conflict copy.
#[test]
fn a_rebind_without_a_known_generation_keeps_replaced_versions() {
    let server = Server::start();
    let password = new_password();
    let email = email();
    let a = Client::local("A", &password);
    let host = a.save(
        None,
        shellpp_core::service::dto::RecordType::Host,
        serde_json::json!({"address": "unsure.example.com", "label": "first"}),
    );
    a.register(&server, &email, &password);
    let b = Client::empty("B");
    done(b.sign_in(&server, &email, &password));
    a.core.sync_sign_out(false).expect("signed out");
    // What a sign-out of an older version leaves: no generation kept.
    let db = rusqlite::Connection::open(a.dir.path().join("data").join("shellpp.db"))
        .expect("the client's database");
    let kept: String = db
        .query_row(
            "SELECT value FROM meta WHERE key = 'sync_last_generation'",
            [],
            |row| row.get(0),
        )
        .expect("kept at sign-out");
    assert!(!kept.is_empty(), "the sign-out kept the generation");
    db.execute(
        "UPDATE meta SET value = '' WHERE key = 'sync_last_generation'",
        [],
    )
    .expect("forgotten");
    drop(db);
    let mut data = b.data(host);
    data["label"] = serde_json::Value::String("B meanwhile".into());
    b.save(
        Some(host),
        shellpp_core::service::dto::RecordType::Host,
        data,
    );
    b.sync();
    done(a.sign_in(&server, &email, &password));
    a.sync();
    assert_eq!(label_of(&a, host), "B meanwhile");
    assert_eq!(losing_labels(&a), vec!["first"]);
    b.sync();
    assert_eq!(losing_labels(&b), vec!["first"], "the copy reaches B");
}

/// D184, residual 1 (D198): after a restore, a device whose clock is
/// behind edits a record at a revision below the one another device
/// synced, with a lower stamp. The server gave that edit a sequence above
/// the one of the synced version, which only a write after the restore
/// can have, so the edit is kept as a conflict copy.
#[test]
fn a_late_edit_from_a_clock_behind_is_kept_by_its_sequence() {
    if on_postgres() {
        return; // The backup here is a file copy of the SQLite database.
    }
    let mut server = Server::start();
    let password = new_password();
    let email = email();
    let a = Client::local("A", &password);
    let host = a.save(
        None,
        shellpp_core::service::dto::RecordType::Host,
        serde_json::json!({"address": "late.example.com", "label": "first"}),
    );
    a.register(&server, &email, &password);
    let b = Client::empty("B");
    done(b.sign_in(&server, &email, &password));
    let backup = server.dir.path().join("backup.db");
    server
        .db
        .batch(&format!("VACUUM INTO '{}'", backup.display()))
        .expect("backup");
    let edit = |client: &Client, label: &str| {
        let mut data = client.data(host);
        data["label"] = serde_json::Value::String(label.into());
        client.save(
            Some(host),
            shellpp_core::service::dto::RecordType::Host,
            data,
        );
    };
    // Three revisions after the backup, each synced: the restore loses them.
    for label in ["A one", "A two", "A after the backup"] {
        edit(&a, label);
        a.sync();
    }
    restore(&mut server, &backup);
    // B, an hour behind, learns the restore, writes other records (the
    // server's sequence moves past A's version), then edits the host one
    // revision above the backup's: below A's, with a lower stamp.
    b.mem.clock.rewind(std::time::Duration::from_secs(3600));
    b.sync();
    for n in 0..4 {
        b.host(&format!("other{n}.example.com"));
    }
    b.sync();
    edit(&b, "B late, clock behind");
    b.sync();
    a.sync();
    assert_eq!(label_of(&a, host), "A after the backup");
    assert_eq!(losing_labels(&a), vec!["B late, clock behind"]);
    b.sync();
    assert_eq!(label_of(&b, host), "A after the backup");
    assert!(losing_labels(&b).contains(&"B late, clock behind".to_owned()));
}
