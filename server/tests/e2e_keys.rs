//! The recovery key and the password change end to end (plan run 9,
//! section 3.2; D173, item 2): "Lost or stolen", "My old password may be
//! compromised" and recovery key reissues, with two or three devices of the
//! real client against the real server in this process. On SQLite by
//! default, on PostgreSQL with `SHELLPP_TEST_POSTGRES=docker`.

mod support;

use secrecy::{ExposeSecret as _, SecretString};
use shellpp_core::CoreError;
use shellpp_core::service::{KeyChangeFailure, SignInInput, SignInOutcome};
use shellpp_server::db::{DbError, TxKind, Ty, Val};
use support::*;
use uuid::Uuid;

fn done(outcome: SignInOutcome) {
    assert!(
        matches!(outcome, SignInOutcome::Done(_)),
        "sign-in finished"
    );
}

fn text(key: &SecretString) -> String {
    key.expose_secret().to_owned()
}

/// The status of `recover/begin` with a recovery key text: `200` when the
/// server still accepts the key (section 5.2).
fn recover_begin_status(server: &Server, email: &str, key: &SecretString) -> u16 {
    let key = shellpp_core::crypto::decode_recovery_key(key.expose_secret()).expect("a key");
    let origin = shellpp_core::sync::http::normalize_origin(&server.origin).expect("origin");
    let auth = shellpp_core::crypto::derive_recovery_auth_key(&key, &origin);
    let body = serde_json::json!({
        "email": email,
        "recovery_auth_key": data_encoding::BASE64URL_NOPAD.encode(auth.expose()),
        "device_id": Uuid::now_v7(),
    });
    raw_http(
        server,
        "POST",
        "/v1/auth/recover/begin",
        &[("content-type", "application/json")],
        body.to_string().as_bytes(),
    )
    .status
}

/// The personal vault's current epoch on the server and its epoch rows.
fn server_epochs(server: &Server, vault_id: Uuid) -> (i64, i64) {
    server
        .db
        .tx(TxKind::Read, |tx| -> Result<(i64, i64), DbError> {
            let current = tx.query(
                "SELECT current_epoch FROM vaults WHERE vault_id = ?",
                &[Val::Uuid(vault_id)],
                &[Ty::Int],
            )?;
            let rows = tx.query(
                "SELECT COUNT(*) FROM vault_key_epochs WHERE vault_id = ?",
                &[Val::Uuid(vault_id)],
                &[Ty::Int],
            )?;
            Ok((
                current[0][0].int().expect("epoch"),
                rows[0][0].int().expect("count"),
            ))
        })
        .expect("epochs")
}

/// Everything a key change replaces on the server: the account's key
/// material, the epochs and the records of the personal vault.
fn server_state(server: &Server, vault_id: Uuid) -> Vec<Vec<Val>> {
    server
        .db
        .tx(TxKind::Read, |tx| -> Result<Vec<Vec<Val>>, DbError> {
            let mut rows = tx.query(
                "SELECT auth_verifier, recovery_verifier, kdf_salt, kek_wrap, recovery_wrap, \
                 credentials_changed_at FROM users",
                &[],
                &[
                    Ty::Text,
                    Ty::Text,
                    Ty::Bytes,
                    Ty::Bytes,
                    Ty::Bytes,
                    Ty::Time,
                ],
            )?;
            rows.extend(tx.query(
                "SELECT current_epoch FROM vaults WHERE vault_id = ?",
                &[Val::Uuid(vault_id)],
                &[Ty::Int],
            )?);
            rows.extend(tx.query(
                "SELECT epoch, prev_key_wrap FROM vault_key_epochs WHERE vault_id = ? \
                 ORDER BY epoch",
                &[Val::Uuid(vault_id)],
                &[Ty::Int, Ty::Bytes],
            )?);
            rows.extend(tx.query(
                "SELECT record_id, seq, revision, key_epoch, envelope FROM records \
                 WHERE vault_id = ? ORDER BY record_id",
                &[Val::Uuid(vault_id)],
                &[Ty::Uuid, Ty::Int, Ty::Int, Ty::Int, Ty::Bytes],
            )?);
            Ok(rows)
        })
        .expect("server state")
}

/// A device with "Remember on this device" on (the in-memory keychain).
fn remembering(client: &Client) {
    let status = client.core.remember_device_set(true).expect("remember on");
    assert!(status.remember_device_enabled);
}

fn sign_in_input(server: &Server, email: &str, password: &SecretString) -> SignInInput {
    SignInInput {
        origin: server.origin.clone(),
        email: email.to_owned(),
        password: password.clone(),
        registration_code: None,
    }
}

/// Run 8, agent C, finding 1 (REPORT-C, section 7): after "Lost or
/// stolen" the thief's copy of the recovery key must no longer pass
/// `recover/begin`, else the stolen device recovers the account.
#[test]
fn lost_or_stolen_retires_the_old_recovery_key() {
    let server = Server::start();
    let password = new_password();
    let email = email();
    let a = Client::empty("A");
    let old_key = a
        .core
        .first_run_setup(password.clone())
        .expect("first run")
        .recovery_key;
    a.host("ls.example.com");
    a.register(&server, &email, &password);
    let b = Client::empty("B");
    done(b.sign_in(&server, &email, &password));
    assert_eq!(recover_begin_status(&server, &email, &old_key), 200);

    // "Lost or stolen" on A for B: revoke, then the password change with
    // "My old password may be compromised" fixed on.
    let devices = a.core.sync_devices().expect("devices");
    let stolen = devices.iter().find(|device| !device.current).expect("B");
    a.core
        .sync_device_revoke(stolen.device_id)
        .expect("revoked");
    let new = new_password();
    a.core
        .password_change_rotating(password.clone(), new.clone())
        .expect("changed");

    assert_eq!(
        recover_begin_status(&server, &email, &old_key),
        401,
        "the old recovery key no longer recovers the account"
    );
}

/// The plan's "device stolen" (run 9, 3.2): A reports B lost or stolen.
/// One rotation; the old password and recovery key stop working and the
/// new key recovers; B gets nothing new and cannot push, and its
/// "Remember on this device" is never made for the new key; C signs in
/// with the new password and goes on with its own data, remembered again.
#[test]
fn a_stolen_device_is_locked_out_and_the_other_devices_go_on() {
    let server = Server::start();
    let password = new_password();
    let email = email();
    let a = Client::empty("A");
    let old_key = a
        .core
        .first_run_setup(password.clone())
        .expect("first run")
        .recovery_key;
    a.host("before.example.com");
    a.register(&server, &email, &password);
    remembering(&a);
    let vault_id = a.vault_id();
    let b = Client::empty("B");
    done(b.sign_in(&server, &email, &password));
    remembering(&b);
    let devices = a.core.sync_devices().expect("devices");
    let b_id = devices
        .iter()
        .find(|device| !device.current)
        .expect("B")
        .device_id;
    let c = Client::empty("C");
    done(c.sign_in(&server, &email, &password));
    remembering(&c);
    assert_eq!(server_epochs(&server, vault_id), (1, 1));

    // Devices ▸ Revoke ▸ "Lost or stolen" for B: revoked at once, then the
    // change with the option fixed on shows its new key first.
    a.core.sync_device_revoke(b_id).expect("revoked");
    let new = new_password();
    let prepared = a
        .core
        .password_change_prepare(password.clone(), new.clone())
        .expect("prepared")
        .expect("bound: a key to show");
    assert_eq!(
        server_epochs(&server, vault_id),
        (1, 1),
        "nothing is sent before \"I saved it\""
    );
    assert_eq!(recover_begin_status(&server, &email, &old_key), 200);
    // While the user saves the key, the thief signs in again with the old
    // password, which clears the revocation (SP 4.7).
    assert!(b.core.sync_now().is_err(), "B is revoked");
    done(b.sign_in(&server, &email, &password));
    b.sync();
    // "I saved it": B is revoked once more and the change sent right after.
    a.core
        .key_change_confirm_lost(prepared.id, b_id)
        .expect("revoked and committed");
    let shown = prepared.recovery_key;
    assert_eq!(server_epochs(&server, vault_id), (2, 2), "one rotation");
    let listed = a.core.sync_devices().expect("devices");
    assert!(
        listed
            .iter()
            .any(|device| device.device_id == b_id && device.revoked),
        "B is revoked"
    );
    // A's own "Remember on this device" follows the rotation (D102).
    assert!(a.core.lock_status().remember_device_enabled);

    // The old password and recovery key are dead; the new key recovers.
    let d = Client::empty("D");
    let refused = d
        .core
        .sync_sign_in(sign_in_input(&server, &email, &password));
    assert!(refused.is_err(), "the old password does not sign in");
    assert_eq!(recover_begin_status(&server, &email, &old_key), 401);
    assert_eq!(recover_begin_status(&server, &email, &shown), 200);

    // B: nothing new arrives, nothing leaves, the old password is refused.
    a.host("after.example.com");
    a.sync();
    b.host("thief.example.com");
    assert!(b.core.sync_now().is_err(), "B is revoked");
    assert!(!b.status().signed_in);
    let again = b
        .core
        .sync_sign_in(sign_in_input(&server, &email, &password));
    assert!(again.is_err(), "the thief's password no longer signs in");
    // Signed out, B cannot even make a key to show (nothing could be sent).
    let refused = b.core.recovery_key_reissue_prepare(password.clone());
    assert!(
        matches!(&refused, Err(CoreError::Sync(error)) if error.code() == "signed_out"),
        "a signed-out device is told to sign in first"
    );
    assert_eq!(
        b.addresses(),
        vec!["before.example.com", "thief.example.com"]
    );
    assert!(
        !b.core.lock_status().remember_device_lost,
        "B never learnt the new key, so it made no wrap for it"
    );
    let b = b.reopen("B");
    assert!(
        b.core.try_device_unlock().expect("device unlock"),
        "B's own copy still opens as before (nothing is wiped)"
    );
    assert!(
        !b.addresses().contains(&"after.example.com".to_owned()),
        "B never got the data written after the rotation"
    );

    // C: an offline edit, then the sign-in with the new password.
    c.host("c-offline.example.com");
    assert!(c.core.sync_now().is_err(), "C was signed out");
    done(c.sign_in_after_change(&server, &email, &new));
    c.sync();
    a.sync();
    assert_eq!(
        a.addresses(),
        vec![
            "after.example.com",
            "before.example.com",
            "c-offline.example.com"
        ],
        "C's own data reached A, the thief's did not"
    );
    assert_eq!(c.addresses(), a.addresses());
    let c_status = c.core.lock_status();
    assert!(
        c_status.remember_device_enabled && !c_status.remember_device_lost,
        "C's device key wraps the new epoch's key"
    );
    let c = c.reopen("C");
    assert!(
        c.core.try_device_unlock().expect("device unlock"),
        "C unlocks at launch with its device key"
    );
    assert_eq!(c.addresses().len(), 3);
    let a = a.reopen("A");
    assert!(
        a.core.try_device_unlock().expect("device unlock"),
        "A unlocks at launch with the new epoch's device wrap"
    );
}

/// "My old password may be compromised" on (D173): the old key fails
/// `recover/begin`, the key shown is the account's, the other device goes
/// on after signing in; its local data is kept.
#[test]
fn a_compromised_password_change_retires_the_old_recovery_key() {
    let server = Server::start();
    let password = new_password();
    let email = email();
    let a = Client::empty("A");
    let old_key = a
        .core
        .first_run_setup(password.clone())
        .expect("first run")
        .recovery_key;
    a.register(&server, &email, &password);
    let b = Client::empty("B");
    done(b.sign_in(&server, &email, &password));

    let new = new_password();
    let shown = a
        .core
        .password_change_rotating(password.clone(), new.clone())
        .expect("changed")
        .expect("bound: the new key")
        .recovery_key;
    assert_eq!(recover_begin_status(&server, &email, &old_key), 401);
    assert_eq!(recover_begin_status(&server, &email, &shown), 200);
    let held = a.core.recovery_key_show(new.clone()).expect("shown");
    assert_eq!(text(&held.recovery_key), text(&shown));

    b.host("b-local.example.com");
    assert!(b.core.sync_now().is_err(), "B was signed out");
    done(b.sign_in_after_change(&server, &email, &new));
    b.sync();
    a.sync();
    assert_eq!(a.addresses(), vec!["b-local.example.com"]);
    let on_b = b.core.recovery_key_show(new).expect("shown on B");
    assert_eq!(text(&on_b.recovery_key), text(&shown));
}

/// The option off: no rotation, the recovery key stays.
#[test]
fn a_password_change_without_the_option_keeps_the_epoch_and_the_key() {
    let server = Server::start();
    let password = new_password();
    let email = email();
    let a = Client::empty("A");
    let key = a
        .core
        .first_run_setup(password.clone())
        .expect("first run")
        .recovery_key;
    a.register(&server, &email, &password);
    let vault_id = a.vault_id();
    let new = new_password();
    a.core
        .password_change(password, new.clone())
        .expect("changed");
    assert_eq!(server_epochs(&server, vault_id), (1, 1), "no rotation");
    assert_eq!(recover_begin_status(&server, &email, &key), 200);
    let held = a.core.recovery_key_show(new).expect("shown");
    assert_eq!(text(&held.recovery_key), text(&key));
}

/// A combined request that fails at its last check (the `recovery_key`
/// record changed meanwhile) changes nothing on the server, and the device
/// keeps its password and key; the failure says the key was not accepted.
#[test]
fn a_failed_combined_request_changes_nothing_on_the_server() {
    let server = Server::start();
    let password = new_password();
    let email = email();
    let a = Client::empty("A");
    let key = a
        .core
        .first_run_setup(password.clone())
        .expect("first run")
        .recovery_key;
    a.register(&server, &email, &password);
    let vault_id = a.vault_id();
    let new = new_password();
    let prepared = a
        .core
        .password_change_prepare(password.clone(), new.clone())
        .expect("prepared")
        .expect("a key to show");
    // Every vault record moves on, so the record's `base_revision` is stale:
    // the server fails after it applied the rotation in its transaction.
    server
        .db
        .tx(TxKind::Write, |tx| -> Result<(), DbError> {
            tx.exec(
                "UPDATE records SET revision = revision + 1 WHERE vault_id = ? AND class = 1",
                &[Val::Uuid(vault_id)],
            )?;
            Ok(())
        })
        .expect("records moved on");
    let before = server_state(&server, vault_id);
    let failure = a.core.key_change_confirm(prepared.id).expect_err("refused");
    match &failure {
        KeyChangeFailure::NotAccepted(CoreError::Sync(error)) => {
            assert_eq!(error.code(), "revision_conflict")
        }
        other => panic!("expected a refusal, got {other:?}"),
    }
    assert_eq!(server_state(&server, vault_id), before, "rolled back");
    assert_eq!(recover_begin_status(&server, &email, &key), 200);
    assert!(
        a.core.key_change_confirm(prepared.id).is_err(),
        "the key shown was discarded"
    );
    // Still signed in, on the old epoch: a new host syncs and opens later.
    a.host("after-refusal.example.com");
    a.sync();
    let a = a.reopen("A");
    a.core.unlock(password.clone()).expect("the old password");
    assert_eq!(a.addresses(), vec!["after-refusal.example.com"]);
    let held = a.core.recovery_key_show(password).expect("shown");
    assert_eq!(text(&held.recovery_key), text(&key));
}

/// Two devices reissue at once (section 5.3, step 7): one request passes,
/// the other is refused as a whole (`401` or `409`) and its key never
/// works; after signing in again it shows the winner's key.
#[test]
fn two_devices_reissuing_at_once_keep_one_key() {
    let server = Server::start();
    let password = new_password();
    let email = email();
    let a = Client::local("A", &password);
    a.register(&server, &email, &password);
    let b = Client::empty("B");
    done(b.sign_in(&server, &email, &password));

    let on_a = a
        .core
        .recovery_key_reissue_prepare(password.clone())
        .expect("A prepared");
    let on_b = b
        .core
        .recovery_key_reissue_prepare(password.clone())
        .expect("B prepared");
    a.core
        .key_change_confirm(on_a.id)
        .expect("A's reissue passes");
    let failure = b
        .core
        .key_change_confirm(on_b.id)
        .expect_err("B's is refused");
    // A's reissue revoked B's tokens: B's refresh is refused (`401`).
    let KeyChangeFailure::SignedOut(CoreError::Sync(error)) = &failure else {
        panic!("expected B to be signed out, got {failure:?}");
    };
    assert_eq!(error.status(), Some(401), "SP 5.3, step 7");
    let (on_a, on_b) = (on_a.recovery_key, on_b.recovery_key);
    assert_eq!(recover_begin_status(&server, &email, &on_b), 401);
    assert_eq!(recover_begin_status(&server, &email, &on_a), 200);

    done(b.sign_in(&server, &email, &password));
    let shown = b.core.recovery_key_show(password).expect("shown on B");
    assert_eq!(text(&shown.recovery_key), text(&on_a));
}

/// Local mode (D173): the compromised password change rotates and puts a
/// new key in the `recovery_key` record without showing it; the old key
/// no longer opens the copy, and the key the later registration shows
/// (UI 13.5, step 4) is the new one, which the server then accepts.
#[test]
fn a_local_compromised_change_replaces_the_recovery_key() {
    let server = Server::start();
    let password = new_password();
    let a = Client::empty("A");
    let old_key = a
        .core
        .first_run_setup(password.clone())
        .expect("first run")
        .recovery_key;
    a.host("local.example.com");
    // The first run's grant to copy the key is still running here.
    let new = new_password();
    let shown = a
        .core
        .password_change_prepare(password, new.clone())
        .expect("changed locally");
    assert!(shown.is_none(), "local mode shows no recovery key");
    assert!(
        a.core
            .secret_copy(shellpp_core::service::dto::SecretCopyTarget::RecoveryKey)
            .is_err(),
        "and ends the grant to copy it"
    );
    assert!(a.core.recovery_key_granted().is_err());
    // Read back through the local path the app never offers (D217).
    shellpp_core::service::local_recovery_key::allow(true);
    let held = a.core.recovery_key_show(new.clone()).expect("held");
    assert_ne!(text(&held.recovery_key), text(&old_key));

    let a = a.reopen("A");
    assert!(
        a.core
            .recovery_unlock(old_key.clone(), new_password())
            .is_err(),
        "the old key no longer opens the copy"
    );
    a.core.unlock(new.clone()).expect("the new password");
    let email = email();
    a.register(&server, &email, &new);
    let registered = a.core.recovery_key_granted().expect("the wizard's key");
    assert_eq!(text(&registered.recovery_key), text(&held.recovery_key));
    assert_eq!(recover_begin_status(&server, &email, &old_key), 401);
    assert_eq!(
        recover_begin_status(&server, &email, &held.recovery_key),
        200
    );
    assert_eq!(a.addresses(), vec!["local.example.com"]);
}

/// A reissue prepared and cancelled sends nothing; the key stays.
#[test]
fn a_cancelled_reissue_sends_nothing() {
    let server = Server::start();
    let password = new_password();
    let email = email();
    let a = Client::empty("A");
    let key = a
        .core
        .first_run_setup(password.clone())
        .expect("first run")
        .recovery_key;
    a.register(&server, &email, &password);
    let vault_id = a.vault_id();
    let before = server_state(&server, vault_id);
    let prepared = a
        .core
        .recovery_key_reissue_prepare(password)
        .expect("prepared");
    assert_eq!(server_state(&server, vault_id), before);
    a.core.key_change_cancel(prepared.id);
    assert!(
        a.core.key_change_confirm(prepared.id).is_err(),
        "nothing to confirm"
    );
    assert_eq!(server_state(&server, vault_id), before);
    assert_eq!(recover_begin_status(&server, &email, &key), 200);
}

/// "Remember on this device" after a rotation made elsewhere, when the
/// device key is gone from the OS key store: the option goes off with a
/// note instead of keeping a wrap that opens nothing (D173).
#[test]
fn remember_goes_off_with_a_note_when_the_device_key_is_gone() {
    let server = Server::start();
    let password = new_password();
    let email = email();
    let a = Client::local("A", &password);
    a.register(&server, &email, &password);
    let b = Client::empty("B");
    done(b.sign_in(&server, &email, &password));
    remembering(&b);
    let c = Client::empty("C");
    done(c.sign_in(&server, &email, &password));
    remembering(&c);
    let new = new_password();
    a.core
        .password_change_rotating(password, new.clone())
        .expect("rotated");
    // B's keychain item is deleted; C's keychain cannot be read.
    for account in b.mem.secret_store.accounts() {
        shellpp_core::platform::SecretStore::delete(&*b.mem.secret_store, &account)
            .expect("deleted");
    }
    c.mem.secret_store.set_available(false);
    for device in [&b, &c] {
        assert!(device.core.sync_now().is_err(), "signed out");
        done(device.sign_in_after_change(&server, &email, &new));
        let status = device.core.lock_status();
        assert!(!status.remember_device_enabled, "no wrap without the key");
        assert!(status.remember_device_lost, "and the note says why");
    }
    c.mem.secret_store.set_available(true);
    for device in [&b, &c] {
        let status = device.core.remember_device_set(true).expect("on again");
        assert!(status.remember_device_enabled && !status.remember_device_lost);
    }
}

/// Two prepared changes (two windows): the newer replaces the older, whose
/// confirmation and copy are refused instead of committing the other key;
/// a lock drops the newer one. The server never sees either.
#[test]
fn a_second_prepare_or_a_lock_drops_the_first() {
    let server = Server::start();
    let password = new_password();
    let email = email();
    let a = Client::empty("A");
    let key = a
        .core
        .first_run_setup(password.clone())
        .expect("first run")
        .recovery_key;
    a.register(&server, &email, &password);
    let vault_id = a.vault_id();
    let before = server_state(&server, vault_id);
    let first = a
        .core
        .recovery_key_reissue_prepare(password.clone())
        .expect("first");
    let second = a
        .core
        .password_change_prepare(password.clone(), new_password())
        .expect("second")
        .expect("a key to show");
    assert_ne!(text(&first.recovery_key), text(&second.recovery_key));
    let copy = |id| {
        a.core
            .secret_copy(shellpp_core::service::dto::SecretCopyTarget::PendingRecoveryKey { id })
    };
    assert!(copy(first.id).is_err(), "the older key is not copied");
    copy(second.id).expect("the newer key is copied");
    assert!(matches!(
        a.core.key_change_confirm(first.id),
        Err(KeyChangeFailure::NotAccepted(CoreError::WrongState))
    ));
    a.core.lock().expect("locked");
    a.core.unlock(password.clone()).expect("unlocked");
    assert!(copy(second.id).is_err(), "a lock drops the prepared key");
    assert!(matches!(
        a.core.key_change_confirm(second.id),
        Err(KeyChangeFailure::NotAccepted(CoreError::WrongState))
    ));
    assert_eq!(server_state(&server, vault_id), before);
    assert_eq!(recover_begin_status(&server, &email, &key), 200);
}

/// Cross review, theme 1 (D195): after Log out with the local copy kept,
/// the vault keeps its account binding, so a change with "My old password
/// may be compromised" is refused (it would leave the server behind with
/// a recovery verifier no key opens); one without the option stays local.
#[test]
fn a_compromised_change_after_log_out_is_refused_while_the_binding_stays() {
    let server = Server::start();
    let password = new_password();
    let email = email();
    let a = Client::local("A", &password);
    a.register(&server, &email, &password);
    a.core.sync_sign_out(false).expect("signed out, copy kept");
    assert!(!a.status().bound);
    let refused = a
        .core
        .password_change_prepare(password.clone(), new_password());
    assert!(
        matches!(&refused, Err(CoreError::Sync(error)) if error.code() == "account_binding"),
        "refused while the vault keeps its account"
    );
    let reissue = a.core.recovery_key_reissue_prepare(password.clone());
    assert!(
        matches!(&reissue, Err(CoreError::Sync(error)) if error.code() == "account_binding"),
        "no new local recovery key either"
    );
    let new = new_password();
    a.core
        .password_change(password, new.clone())
        .expect("without the option the change stays local");
    a.core.lock().expect("locked");
    a.core.unlock(new).expect("the new password");
}

/// SP 5.1, re-post step 4 (D195): a server restored from a backup made
/// before a reissue gets this device's newer `recovery_key` record back
/// with the records, while its recovery verifier is the older key's; the
/// re-post offers a reissue, and after it the key Show gives recovers.
#[test]
fn a_repost_with_a_newer_recovery_key_offers_a_reissue() {
    if on_postgres() {
        return; // The backup here is a file copy of the SQLite database.
    }
    let mut server = Server::start();
    let password = new_password();
    let email = email();
    let a = Client::local("A", &password);
    a.register(&server, &email, &password);
    let db_path = server.dir.path().join("server.db");
    let backup = server.dir.path().join("backup.db");
    server
        .db
        .batch(&format!("VACUUM INTO '{}'", backup.display()))
        .expect("backup");
    a.core
        .recovery_key_reissue(password.clone())
        .expect("reissued after the backup");
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

    assert!(a.core.sync_now().is_err());
    assert!(
        a.core
            .sync_sign_in(sign_in_input(&server, &email, &password))
            .is_err()
    );
    a.core
        .sync_repost_epochs(password.clone())
        .expect("re-posted");
    assert!(a.status().recovery_reissue_needed, "a reissue is offered");
    let shown = a.core.recovery_key_show(password.clone()).expect("shown");
    assert_eq!(
        recover_begin_status(&server, &email, &shown.recovery_key),
        401,
        "the server's verifier is the older key's"
    );
    let reissued = a
        .core
        .recovery_key_reissue(password.clone())
        .expect("reissued");
    assert!(!a.status().recovery_reissue_needed, "the offer is gone");
    assert_eq!(
        recover_begin_status(&server, &email, &reissued.recovery_key),
        200
    );
    let shown = a.core.recovery_key_show(password.clone()).expect("shown");
    assert_eq!(text(&shown.recovery_key), text(&reissued.recovery_key));
    // Another device checks the server's recovery wrap against that key.
    let b = Client::empty("B");
    done(b.sign_in(&server, &email, &password));
    assert!(
        !b.status().recovery_mismatch,
        "its wrap opens to the vault key"
    );
}

/// Cross review, theme 1 (D195): a merge at sign-in with "Remember on this
/// device" makes the option again for the account's vault, with a new
/// device key that replaces the keychain item (the old one would still
/// open the pre-merge backup without the password).
#[test]
fn a_merge_at_sign_in_keeps_remember_with_a_new_device_key() {
    let server = Server::start();
    let password = new_password();
    let email = email();
    let a = Client::local("A", &password);
    a.register(&server, &email, &password);
    let b = Client::local("B", &password);
    b.host("b-local.example.com");
    remembering(&b);
    let before: Vec<Vec<u8>> = b
        .mem
        .secret_store
        .entries()
        .iter()
        .map(|(_, secret)| secret.expose_secret().to_vec())
        .collect();
    done(b.sign_in(&server, &email, &password));
    let status = b.core.lock_status();
    assert!(status.remember_device_enabled && !status.remember_device_lost);
    let after: Vec<Vec<u8>> = b
        .mem
        .secret_store
        .entries()
        .iter()
        .map(|(_, secret)| secret.expose_secret().to_vec())
        .collect();
    assert_eq!(after.len(), 1);
    assert!(!before.contains(&after[0]), "a new device key");
    let b = b.reopen("B");
    assert!(b.core.try_device_unlock().expect("device unlock"));
    assert!(b.addresses().contains(&"b-local.example.com".to_owned()));
}

/// Run 10 (ROADMAP 2.2, run 9's review): the server took the change but
/// its answer was lost. The device cannot tell; the user keeps the new key
/// (`Unknown`), and a sign-in with the new password tells which one holds.
#[test]
fn a_lost_answer_leaves_the_key_change_unknown_until_a_sign_in() {
    let server = Server::start();
    let email = email();
    let password = new_password();
    let a = Client::empty("A");
    let old_key = a
        .core
        .first_run_setup(password.clone())
        .expect("first run")
        .recovery_key;
    a.host("kept.example.com");
    a.register(&server, &email, &password);
    let new = new_password();
    let prepared = a
        .core
        .password_change_prepare(password.clone(), new.clone())
        .expect("prepared")
        .expect("a key to show");
    shellpp_core::sync::http::lose_answer::once("/v1/account/credentials");
    match a.core.key_change_confirm(prepared.id) {
        Err(KeyChangeFailure::Unknown(_)) => {}
        other => panic!("expected an unknown outcome, got {other:?}"),
    }
    assert!(!shellpp_core::sync::http::lose_answer::armed());
    // The server holds the change: the new recovery key, not the old one.
    assert_eq!(
        recover_begin_status(&server, &email, &prepared.recovery_key),
        200
    );
    assert_ne!(recover_begin_status(&server, &email, &old_key), 200);
    // This device learns it at its next sign-in, with the new password.
    let _ = a.core.sync_now();
    done(a.sign_in_after_change(&server, &email, &new));
    a.sync();
    assert_eq!(a.addresses(), vec!["kept.example.com"]);
    let a = a.reopen("A");
    a.core
        .unlock(new.clone())
        .expect("the local copy opens with the new password");
}

/// Run 10 (ROADMAP 2.2, run 9's review): the server took the change and
/// answered, but the device could not store it (`NotStoredHere`). The new
/// key and password are the account's; a sign-in repairs this device.
#[test]
fn a_change_the_device_could_not_store_is_repaired_by_a_sign_in() {
    let server = Server::start();
    let email = email();
    let password = new_password();
    let a = Client::empty("A");
    let old_key = a
        .core
        .first_run_setup(password.clone())
        .expect("first run")
        .recovery_key;
    a.host("kept.example.com");
    a.register(&server, &email, &password);
    let new = new_password();
    let prepared = a
        .core
        .password_change_prepare(password.clone(), new.clone())
        .expect("prepared")
        .expect("a key to show");
    shellpp_core::service::key_change_fail_store::once();
    match a.core.key_change_confirm(prepared.id) {
        Err(KeyChangeFailure::NotStoredHere(_)) => {}
        other => panic!("expected the change not stored here, got {other:?}"),
    }
    assert_eq!(
        recover_begin_status(&server, &email, &prepared.recovery_key),
        200
    );
    assert_ne!(recover_begin_status(&server, &email, &old_key), 200);
    let _ = a.core.sync_now();
    done(a.sign_in_after_change(&server, &email, &new));
    a.sync();
    assert_eq!(a.addresses(), vec!["kept.example.com"]);
    let a = a.reopen("A");
    a.core
        .unlock(new.clone())
        .expect("the local copy opens with the new password");
}

/// Run 10 (ROADMAP 2.2, run 9's review; D196): the recovery key was
/// reissued on another device while this one was out of the account. Its
/// rebind's first cycle fails, so the recovery row waits: it is built from
/// the pulled record by a later cycle, never from the stale one, and the
/// copy opens offline with the new key only.
#[test]
fn a_rebind_whose_first_cycle_fails_builds_the_recovery_row_later() {
    let server = Server::start();
    let email = email();
    let password = new_password();
    let a = Client::local("A", &password);
    a.host("kept.example.com");
    a.register(&server, &email, &password);
    let b = Client::empty("B");
    done(b.sign_in(&server, &email, &password));
    let old_key = b
        .core
        .recovery_key_show(password.clone())
        .expect("the key B holds")
        .recovery_key;
    b.core.sync_sign_out(false).expect("B out of the account");
    let new_key = a
        .core
        .recovery_key_reissue(password.clone())
        .expect("reissued on A")
        .recovery_key;
    // B's rebind: its first pull's answer is lost.
    let vault = a.vault_id();
    shellpp_core::sync::http::lose_answer::once(&format!("/v1/vaults/{vault}/changes"));
    done(b.sign_in(&server, &email, &password));
    assert!(
        !shellpp_core::sync::http::lose_answer::armed(),
        "the rebind's first pull lost its answer"
    );
    b.sync();
    b.core.lock().expect("locked");
    assert!(
        b.core.recovery_unlock(old_key, new_password()).is_err(),
        "the reissued key no longer opens B"
    );
    b.core
        .recovery_unlock(new_key, new_password())
        .expect("the new key opens B offline");
    assert_eq!(b.addresses(), vec!["kept.example.com"]);
}
