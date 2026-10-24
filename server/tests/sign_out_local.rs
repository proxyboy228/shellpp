//! Sign out with "Also delete the local copy" (UI 12.2; storage-format,
//! "Backup and restore"; run 8 agent C finding 3; D190): the device leaves
//! the account and starts at first run with a new empty store, the local
//! copy of the vault is set aside (never deleted silently) and opens only
//! with the master password, and the data on the server stays: another
//! device and this one again restore it from sync.

mod support;

use shellpp_core::platform::OsKind;
use shellpp_core::service::SignInInput;
use shellpp_core::service::dto::LockState;
use shellpp_core::{Core, CoreConfig, CoreError};
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
    // Not on the server yet: it stays only in the set-aside copy.
    a.host("only-here.example.com");
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

    // The set-aside copy opens with the master password only, and holds what
    // was not on the server (D190).
    let copy = tempfile::tempdir().expect("dir");
    std::fs::create_dir_all(copy.path().join("data")).expect("data");
    for entry in std::fs::read_dir(&data).expect("data dir").flatten() {
        let name = entry.file_name().into_string().unwrap_or_default();
        if let Some(side) = name.strip_prefix(archived[0].as_str()) {
            std::fs::copy(
                entry.path(),
                copy.path().join("data").join(format!("shellpp.db{side}")),
            )
            .expect("copied");
        }
    }
    let old = Core::open(
        CoreConfig {
            data_dir: copy.path().join("data"),
            app_version: APP_VERSION.to_owned(),
        },
        a.mem.platform(OsKind::Windows, "A"),
    )
    .expect("the copy opens");
    assert!(
        !old.try_device_unlock().expect("device unlock"),
        "no device key"
    );
    old.unlock(password.clone())
        .expect("the master password opens it");
    let old = Client {
        dir: copy,
        mem: shellpp_core::platform::memory::MemoryPlatform::new(),
        core: old,
    };
    assert_eq!(
        old.addresses(),
        vec!["on-server.example.com", "only-here.example.com"]
    );

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
/// the copy in place (D190).
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
