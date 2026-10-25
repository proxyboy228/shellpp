//! The device key on Android (D218): fingerprint unlock opens the vault
//! after a lock too, and a device key behind the biometric prompt that was
//! not passed is never taken for a lost one: a rotation keeps its item and
//! does not mark the option lost.

mod common;

use secrecy::{ExposeSecret, SecretString};
use shellpp_core::Core;
use shellpp_core::platform::OsKind;
use shellpp_core::service::dto::LockState;

use common::{Env, new_password};

struct Phone {
    env: Env,
    core: Core,
    password: SecretString,
    recovery: String,
}

/// A first run on `os` with "remember on this device" turned on.
fn remembered(os: OsKind) -> Phone {
    let env = Env::new();
    let core = Core::open(env.config(), env.mem.platform(os, "TEST-PHONE")).expect("core opens");
    let password = new_password();
    let recovery = core
        .first_run_setup(password.clone())
        .expect("first run")
        .recovery_key
        .expose_secret()
        .to_owned();
    let status = core.remember_device_set(true).expect("remember on");
    assert!(status.remember_device_enabled);
    Phone {
        env,
        core,
        password,
        recovery,
    }
}

#[test]
fn android_unlocks_with_the_device_key_after_a_lock_and_the_desktop_does_not() {
    let phone = remembered(OsKind::Android);
    phone.core.lock().expect("lock");
    assert!(phone.core.try_device_unlock().expect("device unlock"));
    assert!(phone.core.is_unlocked());
    // Again after a second lock, and after an unlock with the password.
    phone.core.lock().expect("lock");
    phone.core.unlock(phone.password.clone()).expect("password");
    phone.core.lock().expect("lock");
    assert!(phone.core.try_device_unlock().expect("device unlock"));

    // The desktop keeps RR8: after a lock the password is asked.
    let desktop = remembered(OsKind::Windows);
    desktop.core.lock().expect("lock");
    assert!(!desktop.core.try_device_unlock().expect("device unlock"));
    assert_eq!(desktop.core.lock_status().state, LockState::Locked);
}

#[test]
fn a_device_key_behind_the_prompt_is_not_used_and_not_lost() {
    let phone = remembered(OsKind::Android);
    let accounts = phone.env.mem.secret_store.accounts();
    assert_eq!(accounts.len(), 1);
    phone.core.lock().expect("lock");

    // The prompt was cancelled: no unlock, and nothing changes.
    phone.env.mem.secret_store.set_requires_authentication(true);
    assert!(!phone.core.try_device_unlock().expect("device unlock"));
    assert_eq!(phone.core.lock_status().state, LockState::Locked);
    assert_eq!(phone.env.mem.secret_store.accounts(), accounts);

    // A rotation (local recovery) while the key needs the prompt: it goes
    // on without a `device` row for the new epoch, but the item stays and
    // the option is not marked lost.
    let status = phone
        .core
        .recovery_unlock(SecretString::from(phone.recovery.clone()), new_password())
        .expect("recovery");
    assert_eq!(status.state, LockState::Unlocked);
    assert!(!status.remember_device_enabled);
    assert!(!status.remember_device_lost);
    assert_eq!(phone.env.mem.secret_store.accounts(), accounts);

    // After the prompt passes the option is turned on again and works.
    phone
        .env
        .mem
        .secret_store
        .set_requires_authentication(false);
    let status = phone.core.remember_device_set(true).expect("remember on");
    assert!(status.remember_device_enabled);
    assert!(!status.remember_device_lost);
    phone.core.lock().expect("lock");
    assert!(phone.core.try_device_unlock().expect("device unlock"));
}

#[test]
fn the_master_password_is_checked_again_while_unlocked() {
    let phone = remembered(OsKind::Android);
    phone
        .core
        .password_check(phone.password.clone())
        .expect("the right password");
    assert!(matches!(
        phone.core.password_check(new_password()),
        Err(shellpp_core::CoreError::InvalidPassword)
    ));
    assert_eq!(phone.core.unlock_failures(), 1, "counted as an unlock is");
    phone
        .core
        .password_check(phone.password.clone())
        .expect("the right password");
    assert_eq!(phone.core.unlock_failures(), 0);
    phone.core.lock().expect("lock");
    assert!(matches!(
        phone.core.password_check(phone.password.clone()),
        Err(shellpp_core::CoreError::Locked)
    ));
}
