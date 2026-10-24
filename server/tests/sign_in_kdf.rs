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
    // The confirmed parameters are the local copy's too now (D189): it opens
    // with the account's new password.
    laptop.core.lock().expect("locked");
    laptop
        .core
        .unlock(second.clone())
        .expect("the local copy opens with the new password");
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

/// A plain-HTTP server that answers pre-login with the mobile preset and
/// records the paths it is asked for (anything else `401`).
fn weak_prelogin_server() -> (String, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
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
                    (
                        "200 OK",
                        r#"{"kdf":{"alg":"argon2id","v":1,"m_kib":32768,"t":4,"p":2},"kdf_salt":"AAAAAAAAAAAAAAAAAAAAAA"}"#,
                    )
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

    let (hostile, seen) = weak_prelogin_server();
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
