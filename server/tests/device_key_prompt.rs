//! The device key behind Android's biometric prompt (D218) when another
//! device rotates the vault key: the phone's sign-in again (a rebind to the
//! new epoch) runs while its key store answers "authentication required"
//! (the prompt not passed). The item stays, "Remember on this device" is not
//! marked lost, the new epoch has no `device` row, and once the prompt
//! passes the option is turned on again and unlocks.

mod support;

use shellpp_core::platform::OsKind;
use shellpp_core::service::SignInOutcome;
use shellpp_core::{Core, CoreConfig};
use support::*;

#[test]
fn a_rebind_while_the_device_key_needs_the_prompt_keeps_it() {
    let server = Server::start();
    let password = new_password();
    let email = email();
    let desktop = Client::local("Desktop", &password);
    desktop.host("before.example.com");
    desktop.register(&server, &email, &password);

    // The phone: an Android core with fingerprint unlock on.
    let phone = Client::empty("Phone");
    let phone = {
        let Client { dir, mem, core } = phone;
        drop(core);
        let core = Core::open(
            CoreConfig {
                data_dir: dir.path().join("data"),
                app_version: APP_VERSION.to_owned(),
            },
            mem.platform(OsKind::Android, "Phone"),
        )
        .expect("core opens");
        Client { dir, mem, core }
    };
    assert!(matches!(
        phone.sign_in(&server, &email, &password),
        SignInOutcome::Done(_)
    ));
    let status = phone.core.remember_device_set(true).expect("remember on");
    assert!(status.remember_device_enabled);
    let items = phone.mem.secret_store.accounts();
    assert_eq!(items.len(), 1);

    // The desktop changes the password with a rotation (the default).
    let new = new_password();
    let prepared = desktop
        .core
        .password_change_prepare(password.clone(), new.clone())
        .expect("prepared")
        .expect("bound: a key to show");
    desktop
        .core
        .key_change_confirm(prepared.id)
        .expect("committed");

    // The phone finds itself signed out, and signs in again while its key
    // store wants the prompt.
    assert!(phone.core.sync_now().is_err(), "signed out by the change");
    phone.mem.secret_store.set_requires_authentication(true);
    assert!(matches!(
        phone.sign_in_after_change(&server, &email, &new),
        SignInOutcome::Done(_)
    ));
    let status = phone.core.lock_status();
    assert!(!status.remember_device_lost, "not marked lost");
    assert!(!status.remember_device_enabled, "no wrap for the new epoch");
    assert_eq!(phone.mem.secret_store.accounts(), items, "the item stays");
    phone.sync();
    assert!(phone.addresses().contains(&"before.example.com".to_owned()));

    // The prompt passes: the option is turned on again and opens the vault after a lock.
    phone.mem.secret_store.set_requires_authentication(false);
    let status = phone.core.remember_device_set(true).expect("remember on");
    assert!(status.remember_device_enabled && !status.remember_device_lost);
    phone.core.lock().expect("lock");
    assert!(phone.core.try_device_unlock().expect("device unlock"));
}
