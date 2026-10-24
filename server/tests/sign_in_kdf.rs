//! Pre-login on a bound, signed-out device (docs/sync-protocol.md, section
//! 4.2; D173 item 1.5; D189): KDF parameters weaker than this device's (yet
//! above the floor) are refused before anything is derived or sent, as a
//! possible downgrade attack; the sign-in goes on only when the user
//! confirms these very parameters. The legitimate case is a password
//! changed on a device of the mobile preset.

mod support;

use secrecy::SecretString;
use shellpp_core::crypto::{KdfParams, PlatformClass};
use shellpp_core::platform::OsKind;
use shellpp_core::platform::memory::MemoryPlatform;
use shellpp_core::service::{SignInInput, SignInOutcome};
use shellpp_core::sync::HttpError;
use shellpp_core::{Core, CoreConfig, CoreError};
use support::*;

/// A phone at first run: the mobile class writes the mobile preset.
fn phone() -> (tempfile::TempDir, Core) {
    let dir = tempfile::tempdir().expect("dir");
    let mem = MemoryPlatform::new();
    let mut platform = mem.platform(OsKind::Android, "phone");
    platform.class = PlatformClass::Mobile;
    let core = Core::open(
        CoreConfig {
            data_dir: dir.path().join("data"),
            app_version: APP_VERSION.to_owned(),
        },
        platform,
    )
    .expect("core opens");
    (dir, core)
}

fn input(server: &Server, email: &str, password: &SecretString) -> SignInInput {
    SignInInput {
        origin: server.origin.clone(),
        email: email.to_owned(),
        password: password.clone(),
        registration_code: None,
    }
}

#[test]
fn weaker_kdf_parameters_are_refused_until_the_user_confirms_them() {
    let server = Server::start();
    let email = email();
    let first = new_password();
    let laptop = Client::local("laptop", &first);
    laptop.host("kept.example.com");
    laptop.register(&server, &email, &first);

    // The phone joins, then changes the password: the account now has the
    // mobile preset, and the laptop is signed out.
    let (_dir, phone) = phone();
    let outcome = phone
        .sync_sign_in(input(&server, &email, &first))
        .expect("phone signs in");
    assert!(matches!(outcome, SignInOutcome::Done(_)));
    let second = new_password();
    phone
        .password_change(first.clone(), second.clone())
        .expect("password changed on the phone");
    let _ = laptop.core.sync_now();
    let status = laptop.status();
    assert!(
        status.bound && !status.signed_in,
        "the laptop is signed out"
    );

    // Signing in again: refused before anything is derived or sent.
    match laptop.core.sync_sign_in(input(&server, &email, &second)) {
        Err(CoreError::Sync(HttpError::Refused("weaker_kdf"))) => {}
        other => panic!("expected the downgrade refusal, got {other:?}"),
    }
    let offer = laptop.core.sync_weaker_kdf().expect("the parameters shown");
    assert_eq!(offer.local, KdfParams::preset(PlatformClass::Desktop));
    assert_eq!(offer.server, KdfParams::preset(PlatformClass::Mobile));
    assert!(!laptop.status().signed_in);

    // Confirming other parameters than the server's goes nowhere.
    let mut other = offer.server.clone();
    other.m_kib += 1024;
    match laptop
        .core
        .sync_sign_in_accepting_kdf(input(&server, &email, &second), other)
    {
        Err(CoreError::Sync(HttpError::Refused("weaker_kdf"))) => {}
        other => panic!("expected the refusal again, got {other:?}"),
    }

    // The explicit confirmation of these parameters signs in.
    let outcome = laptop
        .core
        .sync_sign_in_accepting_kdf(input(&server, &email, &second), offer.server.clone())
        .expect("signed in after the confirmation");
    assert!(matches!(outcome, SignInOutcome::Done(_)));
    let status = laptop.status();
    assert!(status.bound && status.signed_in);
    assert_eq!(laptop.addresses(), vec!["kept.example.com"]);
    laptop.sync();
    assert!(laptop.core.sync_weaker_kdf().is_none());
}

/// The same or stronger parameters ask nothing: a device whose account
/// kept its KDF signs in again as before.
#[test]
fn the_same_kdf_parameters_sign_in_without_a_question() {
    let server = Server::start();
    let email = email();
    let password = new_password();
    let laptop = Client::local("laptop", &password);
    laptop.register(&server, &email, &password);
    let other = Client::empty("desktop");
    let outcome = other
        .core
        .sync_sign_in(input(&server, &email, &password))
        .expect("signed in");
    assert!(matches!(outcome, SignInOutcome::Done(_)));
    let id = other
        .core
        .sync_devices()
        .expect("devices")
        .into_iter()
        .find(|device| device.current)
        .expect("this device")
        .device_id;
    // Revoked from the laptop: signed out, the same parameters.
    laptop.core.sync_device_revoke(id).expect("revoked");
    let _ = other.core.sync_now();
    assert!(!other.status().signed_in);
    let outcome = other
        .core
        .sync_sign_in(input(&server, &email, &password))
        .expect("signed in again");
    assert!(matches!(outcome, SignInOutcome::Done(_)));
}
