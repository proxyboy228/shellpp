//! Sign out with "Also delete the local copy" (UI 12.2; storage-format,
//! "Backup and restore"; D197, which revokes D190): the device leaves the
//! account and the local copy is deleted for real: the database, the copies
//! set aside earlier, the backups and the device keys of all of them in the
//! OS keychain. The app starts at first run with a new empty store, and the
//! data on the server stays: another device and this one again restore it
//! from sync.

mod support;

use shellpp_core::CoreError;
use shellpp_core::service::SignInInput;
use shellpp_core::service::dto::LockState;
use support::*;

#[test]
fn sign_out_deleting_the_local_copy_deletes_it_and_keeps_the_server_data() {
    let server = Server::start();
    let email = email();
    let password = new_password();
    let a = Client::local("A", &password);
    let data = a.dir.path().join("data");
    // A copy set aside earlier by "Start with a new vault", whose device key
    // stays in the keychain (it opens only that copy).
    a.host("set-aside.example.com");
    a.core
        .remember_device_set(true)
        .expect("remember on this device");
    a.core.lock().expect("locked");
    let archived = a.core.start_new_vault().expect("set aside");
    a.core
        .first_run_setup(password.clone())
        .expect("a new vault");
    assert!(data.join(&archived).is_file());
    assert_eq!(a.mem.secret_store.accounts().len(), 1, "the old copy's key");

    a.host("on-server.example.com");
    a.register(&server, &email, &password);
    a.sync();
    // Not on the server: deleted with the local copy, after the warning.
    a.host("only-here.example.com");
    a.core
        .remember_device_set(true)
        .expect("remember on this device");
    assert_eq!(a.mem.secret_store.accounts().len(), 2);
    std::fs::create_dir_all(data.join("backups")).expect("backups");
    std::fs::write(
        data.join("backups").join("shellpp-20261020T101500Z.db"),
        b"x",
    )
    .expect("a backup");
    std::fs::write(data.join("notes.txt"), b"not ours").expect("a file of the user");

    // What the confirmation lists: the set-aside copy and the backup.
    let listed: Vec<(String, bool)> = a
        .core
        .local_copy_files()
        .expect("listed")
        .into_iter()
        .map(|file| (file.name, file.set_aside))
        .collect();
    assert_eq!(
        listed,
        vec![
            ("backups/shellpp-20261020T101500Z.db".to_owned(), false),
            (archived.clone(), true),
        ]
    );

    a.core.sync_sign_out(true).expect("signed out");
    assert!(
        a.mem.secret_store.accounts().is_empty(),
        "no device key stays in the keychain"
    );
    assert_eq!(a.core.lock_status().state, LockState::FirstRun);
    assert!(!a.core.sync_status().expect("status").bound);
    let mut left: Vec<String> = std::fs::read_dir(&data)
        .expect("data dir")
        .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
        .filter(|name| !name.starts_with("shellpp.db") || name.ends_with(".old"))
        .collect();
    left.sort();
    assert!(
        !left.contains(&archived),
        "the set-aside copy went: {left:?}"
    );
    assert!(!left.contains(&"backups".to_owned()), "{left:?}");
    assert!(left.contains(&"notes.txt".to_owned()), "only our files go");
    assert!(a.core.local_copy_files().expect("listed").is_empty());

    // The server keeps the data: a new device restores it, and so does this
    // one after its new first run.
    let b = Client::empty("B");
    let outcome = b
        .core
        .sync_sign_in(SignInInput {
            origin: server.origin.clone(),
            email: email.clone(),
            password: password.clone(),
            registration_code: None,
        })
        .expect("B restores");
    assert!(matches!(
        outcome,
        shellpp_core::service::SignInOutcome::Done(_)
    ));
    assert_eq!(b.addresses(), vec!["on-server.example.com"]);
    let outcome = a
        .core
        .sync_sign_in(SignInInput {
            origin: server.origin.clone(),
            email,
            password: password.clone(),
            registration_code: None,
        })
        .expect("A restores");
    assert!(matches!(
        outcome,
        shellpp_core::service::SignInOutcome::Done(_)
    ));
    assert_eq!(a.addresses(), vec!["on-server.example.com"]);
}

/// When the OS keychain refuses to delete the device key, sign out with
/// "delete the local copy" stops before anything changes: still signed in,
/// the copy in place (D190, D197).
#[test]
fn a_keychain_that_refuses_stops_the_sign_out_before_anything_changes() {
    let server = Server::start();
    let password = new_password();
    let a = Client::local("A", &password);
    a.host("kept.example.com");
    a.register(&server, &email(), &password);
    a.core
        .remember_device_set(true)
        .expect("remember on this device");
    a.mem.secret_store.set_available(false);
    match a.core.sync_sign_out(true) {
        Err(CoreError::Unavailable) => {}
        other => panic!("expected the keychain refusal, got {other:?}"),
    }
    let status = a.status();
    assert!(status.bound && status.signed_in, "nothing changed");
    assert_eq!(a.core.lock_status().state, LockState::Unlocked);
    assert_eq!(a.addresses(), vec!["kept.example.com"]);
}
