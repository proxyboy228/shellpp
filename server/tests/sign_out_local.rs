//! Sign out with "Also delete the local copy" (UI 12.2; storage-format,
//! "Backup and restore"; run 8 agent C finding 3; D190): the device leaves
//! the account and starts at first run with a new empty store, the local
//! copy of the vault is set aside (never deleted silently) and opens only
//! with the master password, and the data on the server stays: another
//! device and this one again restore it from sync.

mod support;

use shellpp_core::service::SignInInput;
use shellpp_core::service::dto::LockState;
use support::*;

#[test]
fn sign_out_deleting_the_local_copy_starts_an_empty_store_and_keeps_the_server_data() {
    let server = Server::start();
    let email = email();
    let password = new_password();
    let a = Client::local("A", &password);
    a.host("on-server.example.com");
    a.register(&server, &email, &password);
    a.sync();
    let data = a.dir.path().join("data");
    let files_before: Vec<String> = std::fs::read_dir(&data)
        .expect("data dir")
        .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
        .collect();

    a.core
        .remember_device_set(true)
        .expect("remember on this device");
    assert_eq!(a.mem.secret_store.accounts().len(), 1, "the device key");

    a.core.sync_sign_out(true).expect("signed out");
    assert!(
        a.mem.secret_store.accounts().is_empty(),
        "nothing opens the set-aside copy without the master password"
    );

    // A new empty store at first run.
    assert_eq!(a.core.lock_status().state, LockState::FirstRun);
    assert!(!a.core.sync_status().expect("status").bound);
    // The old file is set aside next to the new one, not deleted.
    let archived: Vec<String> = std::fs::read_dir(&data)
        .expect("data dir")
        .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
        .filter(|name| {
            !files_before.contains(name)
                && name.starts_with("shellpp.db.")
                && name.ends_with(".old")
        })
        .collect();
    assert_eq!(archived.len(), 1, "one archive: {archived:?}");

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
