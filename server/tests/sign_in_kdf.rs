//! Pre-login on a bound, signed-out device (docs/sync-protocol.md, section
//! 4.2; D173 item 1.5; D189): KDF parameters weaker than this device's (yet
//! above the floor) are refused before anything is derived or sent, as a
//! possible downgrade attack; the sign-in goes on only when the user
//! confirms these very parameters. The legitimate case is a password
//! changed on a device of the mobile preset. Settings or a salt that differ
//! otherwise mean the password or the email changed on another device: the
//! device says so and asks for the current password before deriving
//! anything (SP 4.2, stage 0 decision; run 10).

mod support;

use secrecy::SecretString;
use shellpp_core::crypto::{KdfParams, PlatformClass};
use shellpp_core::platform::OsKind;
use shellpp_core::platform::memory::MemoryPlatform;
use shellpp_core::service::{SignInInput, SignInOutcome};
use shellpp_core::sync::HttpError;
use shellpp_core::{Core, CoreConfig, CoreError};
use shellpp_server::db::{DbError, TxKind, Ty, Val};
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

    // The confirmation of these parameters goes on to SP 4.2: the password
    // changed elsewhere, the user types the current one again.
    match laptop
        .core
        .sync_sign_in_accepting_kdf(input(&server, &email, &second), offer.server.clone())
    {
        Err(CoreError::Sync(HttpError::Refused("kdf_changed"))) => {}
        other => panic!("expected the changed settings, got {other:?}"),
    }
    let outcome = laptop
        .core
        .sync_sign_in_after_kdf_change(input(&server, &email, &second), Some(offer.server.clone()))
        .expect("signed in after the confirmation");
    assert!(matches!(outcome, SignInOutcome::Done(_)));
    let status = laptop.status();
    assert!(status.bound && status.signed_in);
    assert_eq!(laptop.addresses(), vec!["kept.example.com"]);
    laptop.sync();
    assert!(laptop.core.sync_weaker_kdf().is_none());
    // The confirmed parameters are the local copy's too now (D189): it opens
    // with the account's new password.
    laptop.core.lock().expect("locked");
    laptop
        .core
        .unlock(second.clone())
        .expect("the local copy opens with the new password");
}

/// The same parameters and salt ask nothing: a device whose account kept
/// its KDF signs in again as before.
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

/// SP 4.2: the password changed on another device of the same preset. The
/// signed-out device sees another salt, says so before deriving anything,
/// and signs in once the user typed the current password.
#[test]
fn changed_kdf_settings_ask_for_the_current_password_first() {
    let server = Server::start();
    let email = email();
    let first = new_password();
    let laptop = Client::local("laptop", &first);
    laptop.host("kept.example.com");
    laptop.register(&server, &email, &first);
    let desktop = Client::empty("desktop");
    let outcome = desktop
        .core
        .sync_sign_in(input(&server, &email, &first))
        .expect("signed in");
    assert!(matches!(outcome, SignInOutcome::Done(_)));
    let second = new_password();
    desktop
        .core
        .password_change(first.clone(), second.clone())
        .expect("password changed on the desktop");
    let _ = laptop.core.sync_now();
    assert!(!laptop.status().signed_in, "the laptop is signed out");

    // The old password, as the user would type it: refused before anything
    // is derived, and so is the new one until the user was told.
    for password in [&first, &second] {
        match laptop.core.sync_sign_in(input(&server, &email, password)) {
            Err(CoreError::Sync(HttpError::Refused("kdf_changed"))) => {}
            other => panic!("expected the changed settings, got {other:?}"),
        }
    }
    // A mistyped current password is refused by the server, and the next
    // try does not ask again (review of run 10).
    match laptop
        .core
        .sync_sign_in_after_kdf_change(input(&server, &email, &first), None)
    {
        Err(CoreError::Sync(HttpError::Api { status: 401, .. })) => {}
        other => panic!("expected the server's refusal, got {other:?}"),
    }
    let outcome = laptop
        .core
        .sync_sign_in_after_kdf_change(input(&server, &email, &second), None)
        .expect("signed in with the current password");
    assert!(matches!(outcome, SignInOutcome::Done(_)));
    assert_eq!(laptop.addresses(), vec!["kept.example.com"]);
    laptop.core.lock().expect("locked");
    laptop
        .core
        .unlock(second.clone())
        .expect("the local copy opens with the current password");
}

/// SP 4.2 at a server that only answers pre-login, with the desktop preset
/// and a salt this device has not seen: no login request is sent, also not
/// with the acknowledgement for other settings.
#[test]
fn changed_kdf_settings_send_nothing_before_the_user_is_told() {
    let server = Server::start();
    let email = email();
    let password = new_password();
    let laptop = Client::local("laptop", &password);
    laptop.register(&server, &email, &password);
    laptop.core.sync_sign_out(false).expect("logged out");
    let (other, seen) = prelogin_server(DESKTOP_PRELOGIN);
    let input = |origin: &str| SignInInput {
        origin: origin.to_owned(),
        email: email.clone(),
        password: password.clone(),
        registration_code: None,
    };
    match laptop.core.sync_sign_in(input(&other)) {
        Err(CoreError::Sync(HttpError::Refused("kdf_changed"))) => {}
        other => panic!("expected the changed settings, got {other:?}"),
    }
    assert_eq!(
        seen.lock().unwrap().clone(),
        vec!["/v1/auth/prelogin"],
        "no auth key was sent"
    );
    // Told, the next sign-in derives and sends; this server refuses it.
    match laptop
        .core
        .sync_sign_in_after_kdf_change(input(&other), None)
    {
        Err(CoreError::Sync(HttpError::Api { status: 401, .. })) => {}
        other => panic!("expected the server's refusal, got {other:?}"),
    }
    assert_eq!(seen.lock().unwrap().len(), 3, "pre-login, pre-login, login");
}

/// Review of run 10: the acknowledgement covers only the settings and salt
/// the user was told about. Another salt at the next try asks again, and
/// nothing is sent to that server either.
#[test]
fn the_acknowledgement_covers_only_the_settings_seen() {
    let server = Server::start();
    let email = email();
    let password = new_password();
    let laptop = Client::local("laptop", &password);
    laptop.register(&server, &email, &password);
    laptop.core.sync_sign_out(false).expect("logged out");
    let (first, _) = prelogin_server(DESKTOP_PRELOGIN);
    let (second, seen) = prelogin_server(OTHER_SALT_PRELOGIN);
    let input = |origin: &str| SignInInput {
        origin: origin.to_owned(),
        email: email.clone(),
        password: password.clone(),
        registration_code: None,
    };
    match laptop.core.sync_sign_in(input(&first)) {
        Err(CoreError::Sync(HttpError::Refused("kdf_changed"))) => {}
        other => panic!("expected the changed settings, got {other:?}"),
    }
    match laptop
        .core
        .sync_sign_in_after_kdf_change(input(&second), None)
    {
        Err(CoreError::Sync(HttpError::Refused("kdf_changed"))) => {}
        other => panic!("expected the changed settings again, got {other:?}"),
    }
    assert_eq!(
        seen.lock().unwrap().clone(),
        vec!["/v1/auth/prelogin"],
        "no auth key was sent"
    );
}

const OTHER_SALT_PRELOGIN: &str = r#"{"kdf":{"alg":"argon2id","v":1,"m_kib":65536,"t":3,"p":4},"kdf_salt":"BBBBBBBBBBBBBBBBBBBBBA"}"#;
const MOBILE_PRELOGIN: &str = r#"{"kdf":{"alg":"argon2id","v":1,"m_kib":32768,"t":4,"p":2},"kdf_salt":"AAAAAAAAAAAAAAAAAAAAAA"}"#;
const DESKTOP_PRELOGIN: &str = r#"{"kdf":{"alg":"argon2id","v":1,"m_kib":65536,"t":3,"p":4},"kdf_salt":"AAAAAAAAAAAAAAAAAAAAAA"}"#;

/// A plain-HTTP server that answers pre-login with `prelogin` and records
/// the paths it is asked for (anything else `401`).
fn prelogin_server(
    prelogin: &'static str,
) -> (String, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("addr").port();
    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let log = seen.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let log = log.clone();
            std::thread::spawn(move || {
                let _ = stream.set_read_timeout(Some(std::time::Duration::from_millis(300)));
                let mut buffer = Vec::new();
                let mut chunk = [0u8; 4096];
                loop {
                    match stream.read(&mut chunk) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => buffer.extend_from_slice(&chunk[..n]),
                    }
                }
                let text = String::from_utf8_lossy(&buffer).into_owned();
                let path = text
                    .split_whitespace()
                    .nth(1)
                    .unwrap_or_default()
                    .to_owned();
                log.lock().unwrap().push(path.clone());
                let (status, body) = if path == "/v1/auth/prelogin" {
                    ("200 OK", prelogin)
                } else {
                    (
                        "401 Unauthorized",
                        r#"{"error":{"code":"invalid_credentials","message":"x"}}"#,
                    )
                };
                let answer = format!(
                    "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(answer.as_bytes());
            });
        }
    });
    (format!("http://127.0.0.1:{port}"), seen)
}

/// A device that is no longer bound but was (Log out, or an
/// `account_deleted` answer of a hostile server: the vault keeps its
/// binding) still compares with its own parameters, at any address: a
/// server that asks for weaker ones gets nothing, not even a login request
/// (review of D189).
#[test]
fn a_device_bound_before_refuses_weaker_parameters_at_any_address() {
    let server = Server::start();
    let email = email();
    let password = new_password();
    let laptop = Client::local("laptop", &password);
    laptop.register(&server, &email, &password);
    laptop.core.sync_sign_out(false).expect("logged out");
    assert!(!laptop.status().bound);

    let (hostile, seen) = prelogin_server(MOBILE_PRELOGIN);
    let input = SignInInput {
        origin: hostile,
        email,
        password: password.clone(),
        registration_code: None,
    };
    match laptop.core.sync_sign_in(input) {
        Err(CoreError::Sync(HttpError::Refused("weaker_kdf"))) => {}
        other => panic!("expected the downgrade refusal, got {other:?}"),
    }
    let paths = seen.lock().unwrap().clone();
    assert_eq!(paths, vec!["/v1/auth/prelogin"], "no auth key was sent");
}

/// A device bound to one account that signs in to another account on another
/// server (D200, D209; on the same server the device id is already another
/// user's and the login is refused, SP 4.7): the other salt reads as changed
/// settings (the email may have changed, SP 5.4), the login goes through,
/// the merge refuses `previous_account`, and the session that login opened
/// is revoked: no refresh token of it stays live. The device's own account
/// still signs in, its email in other letter case.
#[test]
fn another_accounts_login_on_a_bound_device_is_refused_and_revoked() {
    let server = Server::start();
    let email = email();
    let password = new_password();
    let laptop = Client::local("laptop", &password);
    laptop.register(&server, &email, &password);
    laptop.core.sync_sign_out(false).expect("logged out");

    let other = Server::start();
    let other_email = support::email();
    let other_password = new_password();
    let desk = Client::local("desk", &other_password);
    desk.register(&other, &other_email, &other_password);
    assert_eq!(live_refresh_tokens(&other, &other_email), 1);

    match laptop
        .core
        .sync_sign_in(input(&other, &other_email, &other_password))
    {
        Err(CoreError::Sync(HttpError::Refused("kdf_changed"))) => {}
        other => panic!("expected the changed settings, got {other:?}"),
    }
    match laptop
        .core
        .sync_sign_in_after_kdf_change(input(&other, &other_email, &other_password), None)
    {
        Err(CoreError::Sync(HttpError::Refused("previous_account"))) => {}
        other => panic!("expected the refusal, got {other:?}"),
    }
    assert_eq!(
        live_refresh_tokens(&other, &other_email),
        1,
        "only the desk's session stays"
    );
    assert!(!laptop.status().signed_in);

    let outcome = laptop
        .core
        .sync_sign_in(input(&server, &email.to_uppercase(), &password))
        .expect("the own account signs in");
    assert!(matches!(outcome, SignInOutcome::Done(_)));
}

/// The refresh tokens of the account with `email` that are not revoked.
fn live_refresh_tokens(server: &Server, email: &str) -> i64 {
    server
        .db
        .tx(TxKind::Read, |tx| {
            let rows = tx.query(
                "SELECT COUNT(*) FROM refresh_tokens t JOIN users u ON u.user_id = t.user_id \
                 WHERE u.email = ? AND t.revoked_at IS NULL",
                &[Val::from(email)],
                &[Ty::Int],
            )?;
            Ok::<_, DbError>(rows[0][0].int().unwrap_or(0))
        })
        .expect("count")
}
