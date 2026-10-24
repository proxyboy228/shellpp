//! The server's limits at the API (docs/sync-protocol.md, section 10.1):
//! a burst of sign-ins does not hold the database, the growing TOTP delay
//! per account and client address with its soft limit per account, and the
//! client address behind a trusted proxy. On PostgreSQL with
//! `SHELLPP_TEST_POSTGRES=docker`. Keys and codes are generated at run time.

mod support;

use std::sync::{Arc, Barrier, Mutex};
use std::time::{Duration, Instant};

use serde_json::json;
use support::*;

fn b64(bytes: &[u8]) -> String {
    data_encoding::BASE64URL_NOPAD.encode(bytes)
}

/// Registers an account at the API: its email, auth key and access token.
fn register(server: &Server) -> (String, [u8; 32], String) {
    let email = email();
    let auth_key = shellpp_server::secrets::random_bytes::<32>();
    let body = json!({
        "email": email,
        "kdf": {"alg": "argon2id", "v": 1, "m_kib": 65536, "t": 3, "p": 4},
        "kdf_salt": b64(&[1; 16]),
        "auth_key": b64(&auth_key),
        "recovery_auth_key": b64(&shellpp_server::secrets::random_bytes::<32>()),
        "personal_vault": {
            "vault_id": uuid::Uuid::now_v7(),
            "epoch": 1,
            "kek_wrap": b64(&[4; 73]),
            "recovery_wrap": b64(&[5; 73]),
            "epoch_chain": [],
        },
        "x25519_public_key": b64(&[6; 32]),
        "ssh_id_public_key": "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIK0 shellpp:x",
        "device_id": uuid::Uuid::now_v7(),
    });
    let answer = raw_http(
        server,
        "POST",
        "/v1/auth/register",
        &[("content-type", "application/json")],
        body.to_string().as_bytes(),
    );
    assert_eq!(answer.status, 201, "{}", answer.body);
    let token = answer.body["tokens"]["access_token"]
        .as_str()
        .expect("an access token")
        .to_owned();
    (email, auth_key, token)
}

fn login_body(email: &str, key: &[u8]) -> Vec<u8> {
    json!({"email": email, "auth_key": b64(key), "device_id": uuid::Uuid::now_v7()})
        .to_string()
        .into_bytes()
}

/// Run 7 finding 1 (D174): a burst of failing sign-ins verifies its keys
/// outside the database transaction, so other requests that need the
/// database are answered while the burst is still being verified. Before,
/// every sign-in held the database (the one SQLite connection, or a pool
/// connection) during its Argon2id check, and a request behind a burst
/// waited for most of it.
#[test]
fn a_burst_of_sign_ins_does_not_hold_the_database() {
    const BURST: usize = 64;
    let server = Arc::new(Server::start());
    let (_, _, token) = register(&server);
    let bearer = format!("Bearer {token}");
    // Unknown emails, each its own: no failure limit stops a check early.
    let answered = Arc::new(Mutex::new(0usize));
    let start = Arc::new(Barrier::new(BURST + 1));
    let threads: Vec<_> = (0..BURST)
        .map(|_| {
            let server = server.clone();
            let answered = answered.clone();
            let start = start.clone();
            std::thread::spawn(move || {
                let body = login_body(&email(), &shellpp_server::secrets::random_bytes::<32>());
                start.wait();
                let answer = raw_http(
                    &server,
                    "POST",
                    "/v1/auth/login",
                    &[("content-type", "application/json")],
                    &body,
                );
                *answered.lock().unwrap() += 1;
                (answer.status, answer.code().map(str::to_owned))
            })
        })
        .collect();
    start.wait();
    // The burst is being verified once its first answer is out.
    let deadline = Instant::now() + Duration::from_secs(120);
    while *answered.lock().unwrap() == 0 {
        assert!(Instant::now() < deadline, "no sign-in was answered");
        std::thread::sleep(Duration::from_millis(1));
    }
    // Three requests that each need the database, one after the other.
    for _ in 0..3 {
        let answer = raw_http(
            &server,
            "GET",
            "/v1/account",
            &[("authorization", &bearer)],
            b"",
        );
        assert_eq!(answer.status, 200, "{}", answer.body);
    }
    let done = *answered.lock().unwrap();
    for thread in threads {
        let (status, code) = thread.join().expect("a sign-in");
        assert_eq!(
            (status, code.as_deref()),
            (401, Some("invalid_credentials"))
        );
    }
    // With a margin: at most three quarters of the burst was answered
    // before the three requests were (before the change, nearly all of it).
    assert!(
        done <= BURST * 3 / 4,
        "the requests waited for {done} of {BURST} sign-ins"
    );
}

/// Review finding (D174): checks of one email that run at once never pass
/// the failure limit together (10 per 15 minutes, section 10.1). Each
/// attempt counts before its Argon2id check; a passing check takes it back.
#[test]
fn concurrent_failures_of_one_email_stop_at_the_limit() {
    const TRIES: usize = 30;
    let server = Arc::new(Server::start());
    let (email, auth_key, _) = register(&server);
    let start = Arc::new(Barrier::new(TRIES));
    let threads: Vec<_> = (0..TRIES)
        .map(|_| {
            let server = server.clone();
            let start = start.clone();
            let body = login_body(&email, &shellpp_server::secrets::random_bytes::<32>());
            std::thread::spawn(move || {
                start.wait();
                let answer = raw_http(
                    &server,
                    "POST",
                    "/v1/auth/login",
                    &[("content-type", "application/json")],
                    &body,
                );
                (answer.status, answer.code().map(str::to_owned))
            })
        })
        .collect();
    let mut wrong = 0;
    let mut limited = 0;
    for thread in threads {
        match thread.join().expect("a sign-in") {
            (401, Some(code)) if code == "invalid_credentials" => wrong += 1,
            (429, Some(code)) if code == "rate_limited" => limited += 1,
            other => panic!("unexpected answer {other:?}"),
        }
    }
    assert_eq!((wrong, limited), (10, TRIES - 10));
    // The right key is limited too until the window ends.
    let answer = raw_http(
        &server,
        "POST",
        "/v1/auth/login",
        &[("content-type", "application/json")],
        &login_body(&email, &auth_key),
    );
    assert_eq!(answer.status, 429);
}

/// Review finding (D174): a personal key epoch for a vault that is not the
/// account's personal vault answers `403 not_member` before the key is
/// checked, as before the check moved out of the transaction: no Argon2id,
/// no failure counted.
#[test]
fn a_key_epoch_for_another_vault_is_refused_before_the_key_is_checked() {
    let server = Server::start();
    let (email, auth_key, token) = register(&server);
    let bearer = format!("Bearer {token}");
    let body = json!({
        "current_auth_key": b64(&shellpp_server::secrets::random_bytes::<32>()),
        "epoch": 2,
        "prev_key_wrap": b64(&[1; 73]),
        "account_wraps": {"kek_wrap": b64(&[1; 73]), "recovery_wrap": b64(&[1; 73])},
    })
    .to_string();
    for _ in 0..12 {
        let answer = raw_http(
            &server,
            "POST",
            &format!("/v1/vaults/{}/key-epochs", uuid::Uuid::now_v7()),
            &[
                ("authorization", &bearer),
                ("content-type", "application/json"),
            ],
            body.as_bytes(),
        );
        assert_eq!(
            (answer.status, answer.code()),
            (403, Some("not_member")),
            "{}",
            answer.body
        );
    }
    // Twelve refusals counted no failure: the right key still signs in.
    let answer = raw_http(
        &server,
        "POST",
        "/v1/auth/login",
        &[("content-type", "application/json")],
        &login_body(&email, &auth_key),
    );
    assert_eq!(answer.status, 200, "{}", answer.body);
}
