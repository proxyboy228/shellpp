//! The server's limits at the API (docs/sync-protocol.md, section 10.1):
//! a burst of sign-ins does not hold the database, the growing TOTP delay
//! per account and client address with its soft limit per account, and the
//! client address behind a trusted proxy. On PostgreSQL with
//! `SHELLPP_TEST_POSTGRES=docker`. Keys and codes are generated at run time.

mod support;

use std::sync::{Arc, Barrier, Mutex};
use std::time::{Duration, Instant};

use serde_json::json;
use shellpp_server::config::Cidr;
use shellpp_server::ratelimit::RateLimits;
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

// ---------------------------------------------------------------------------
// TOTP delays (D175)
// ---------------------------------------------------------------------------

/// A server that believes `X-Forwarded-For` from 127.0.0.1, where the
/// tests connect from, so a request can name its client address.
fn proxied(limits: RateLimits) -> Server {
    Server::with(Options {
        trusted_proxies: vec![Cidr::parse("127.0.0.1/32").unwrap()],
        rate_limits: limits,
        ..Options::default()
    })
}

fn post(
    server: &Server,
    path: &str,
    headers: &[(&str, &str)],
    body: serde_json::Value,
) -> RawAnswer {
    let mut all = vec![("content-type", "application/json")];
    all.extend_from_slice(headers);
    raw_http(server, "POST", path, &all, body.to_string().as_bytes())
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

/// A new account with TOTP on: its email, auth key and TOTP secret.
fn with_totp(server: &Server) -> (String, [u8; 32], [u8; 20]) {
    let (email, auth_key, token) = register(server);
    let bearer = format!("Bearer {token}");
    let setup = post(
        server,
        "/v1/account/totp/setup",
        &[("authorization", &bearer)],
        json!({"current_auth_key": b64(&auth_key)}),
    );
    assert_eq!(setup.status, 200, "{}", setup.body);
    let secret: [u8; 20] = data_encoding::BASE32_NOPAD
        .decode(setup.body["secret"].as_str().unwrap().as_bytes())
        .unwrap()
        .try_into()
        .unwrap();
    let enable = post(
        server,
        "/v1/account/totp/enable",
        &[("authorization", &bearer)],
        json!({"code": shellpp_server::secrets::totp_code(&secret, unix_now())}),
    );
    assert_eq!(enable.status, 204, "{}", enable.body);
    (email, auth_key, secret)
}

fn login_ticket(server: &Server, email: &str, auth_key: &[u8]) -> String {
    let answer = post(
        server,
        "/v1/auth/login",
        &[],
        json!({"email": email, "auth_key": b64(auth_key), "device_id": uuid::Uuid::now_v7()}),
    );
    assert_eq!(answer.body["status"], "totp_required", "{}", answer.body);
    answer.body["login_ticket"].as_str().unwrap().to_owned()
}

/// A TOTP code for `ticket` from the client at `ip`.
fn totp(server: &Server, ticket: &str, code: &str, ip: &str) -> RawAnswer {
    post(
        server,
        "/v1/auth/login/totp",
        &[("x-forwarded-for", ip)],
        json!({"login_ticket": ticket, "code": code}),
    )
}

/// The code of the next step: the step of the enable is used up.
fn right(secret: &[u8; 20]) -> String {
    shellpp_server::secrets::totp_code(secret, unix_now() + 30)
}

/// A code accepted at no step around now.
fn wrong(secret: &[u8; 20]) -> String {
    let now = unix_now();
    let near: Vec<String> = [now - 30, now, now + 30, now + 60]
        .iter()
        .map(|t| shellpp_server::secrets::totp_code(secret, *t))
        .collect();
    (0..1_000_000)
        .map(|n| format!("{n:06}"))
        .find(|code| !near.contains(code))
        .unwrap()
}

fn assert_wrong(answer: &RawAnswer) {
    assert_eq!(
        (answer.status, answer.code()),
        (401, Some("invalid_totp")),
        "{}",
        answer.body
    );
}

/// `429 rate_limited` with a `Retry-After` in `seconds`.
fn assert_delayed(answer: &RawAnswer, seconds: std::ops::RangeInclusive<u64>) -> u64 {
    assert_eq!(
        (answer.status, answer.code()),
        (429, Some("rate_limited")),
        "{}",
        answer.body
    );
    let wait = answer.retry_after.expect("a Retry-After");
    assert!(
        seconds.contains(&wait),
        "Retry-After {wait}, expected {seconds:?}"
    );
    wait
}

/// Waits out a `Retry-After`, with a margin.
fn wait_out(seconds: u64) {
    std::thread::sleep(Duration::from_secs(seconds) + Duration::from_millis(200));
}

const ATTACKER: &str = "198.51.100.7";
const USER: &str = "192.0.2.1";

/// D175: wrong codes from one client address make it wait 2 s, then 4 s
/// (here after one free failure), answered as `429` with `Retry-After`, a
/// right code too while the delay runs; another address is not delayed;
/// a right code starts that address over.
#[test]
fn totp_delays_grow_per_client_and_a_right_code_resets_them() {
    let server = proxied(RateLimits {
        totp_free_failures: 1,
        totp_soft_failures: 1_000,
        ..RateLimits::generous()
    });
    let (email, auth_key, secret) = with_totp(&server);
    let ticket = login_ticket(&server, &email, &auth_key);
    assert_wrong(&totp(&server, &ticket, &wrong(&secret), ATTACKER));
    let wait = assert_delayed(&totp(&server, &ticket, &wrong(&secret), ATTACKER), 1..=2);
    // Another address: its own count.
    assert_wrong(&totp(&server, &ticket, &wrong(&secret), USER));
    wait_out(wait);
    assert_wrong(&totp(&server, &ticket, &wrong(&secret), ATTACKER));
    let wait = assert_delayed(&totp(&server, &ticket, &wrong(&secret), ATTACKER), 3..=4);
    // The right code waits too: the delay is not a hint about the code.
    assert_delayed(&totp(&server, &ticket, &right(&secret), ATTACKER), 1..=4);
    wait_out(wait);
    let answer = totp(&server, &ticket, &right(&secret), ATTACKER);
    assert_eq!(answer.status, 200, "{}", answer.body);
    // After the right code the address counts from one again (else: 8 s).
    let ticket = login_ticket(&server, &email, &auth_key);
    assert_wrong(&totp(&server, &ticket, &wrong(&secret), ATTACKER));
    assert_delayed(&totp(&server, &ticket, &wrong(&secret), ATTACKER), 1..=2);
}

/// D175: the failures of one address put the soft delay of the account on
/// every other address, but never more than it: here the attacker waits
/// 8 s while the user waits at most 2 s and then signs in.
#[test]
fn another_client_waits_at_most_the_soft_delay_of_the_account() {
    let server = proxied(RateLimits {
        totp_free_failures: 1,
        totp_soft_failures: 3,
        ..RateLimits::generous()
    });
    let (email, auth_key, secret) = with_totp(&server);
    let ticket = login_ticket(&server, &email, &auth_key);
    // Three failures from the attacker, each as soon as it may try.
    assert_wrong(&totp(&server, &ticket, &wrong(&secret), ATTACKER));
    wait_out(assert_delayed(
        &totp(&server, &ticket, &wrong(&secret), ATTACKER),
        1..=2,
    ));
    assert_wrong(&totp(&server, &ticket, &wrong(&secret), ATTACKER));
    wait_out(assert_delayed(
        &totp(&server, &ticket, &wrong(&secret), ATTACKER),
        3..=4,
    ));
    assert_wrong(&totp(&server, &ticket, &wrong(&secret), ATTACKER));
    // The attacker: 8 s. The user, who never failed: the soft delay, 2 s.
    assert_delayed(&totp(&server, &ticket, &wrong(&secret), ATTACKER), 7..=8);
    let wait = assert_delayed(&totp(&server, &ticket, &right(&secret), USER), 1..=2);
    wait_out(wait);
    let answer = totp(&server, &ticket, &right(&secret), USER);
    assert_eq!(answer.status, 200, "the user signs in: {}", answer.body);
    let bearer = format!(
        "Bearer {}",
        answer.body["tokens"]["access_token"].as_str().unwrap()
    );
    let ticket = login_ticket(&server, &email, &auth_key);
    assert_delayed(&totp(&server, &ticket, &wrong(&secret), ATTACKER), 1..=6);
    // Signed in from there, the user's network is known: the soft limit
    // no longer holds it, however the attacker takes its turns.
    assert_wrong(&totp(&server, &ticket, &wrong(&secret), "203.0.113.5"));
    assert_delayed(
        &totp(&server, &ticket, &wrong(&secret), "203.0.113.6"),
        1..=4,
    );
    assert_wrong(&totp(&server, &ticket, &wrong(&secret), USER));
    // Review finding: a code in a signed-in request (here: turning TOTP
    // off) is held by the soft limit on every network, known or not; a
    // stolen token gets no network out of it.
    let disable = post(
        &server,
        "/v1/account/totp/disable",
        &[("authorization", &bearer), ("x-forwarded-for", USER)],
        json!({"current_auth_key": b64(&auth_key), "code": wrong(&secret)}),
    );
    assert_delayed(&disable, 1..=8);
}

// ---------------------------------------------------------------------------
// Client addresses behind a proxy (D103, D176)
// ---------------------------------------------------------------------------

/// `X-Forwarded-For` names the client only when the peer is a trusted
/// proxy. From any other peer the header is ignored, so a client cannot
/// dodge its TOTP delay by naming another address in it.
#[test]
fn a_forged_forwarded_address_does_not_dodge_the_totp_delay() {
    let limits = RateLimits {
        totp_free_failures: 1,
        totp_soft_failures: 1_000,
        ..RateLimits::generous()
    };
    // The tests connect from 127.0.0.1; only 10.0.0.0/8 is trusted here.
    let server = Server::with(Options {
        trusted_proxies: vec![Cidr::parse("10.0.0.0/8").unwrap()],
        rate_limits: limits,
        ..Options::default()
    });
    let (email, auth_key, secret) = with_totp(&server);
    let ticket = login_ticket(&server, &email, &auth_key);
    assert_wrong(&totp(&server, &ticket, &wrong(&secret), ATTACKER));
    // Another address in the header: the same client, still delayed.
    for forged in [USER, "203.0.113.9, 192.0.2.77", "10.0.0.1"] {
        assert_delayed(&totp(&server, &ticket, &wrong(&secret), forged), 1..=2);
    }
    // With the peer trusted, the header names the client (and the same
    // forged header would now be believed, which is why only the proxy in
    // front of the server may be trusted).
    let trusting = proxied(limits);
    let (email, auth_key, secret) = with_totp(&trusting);
    let ticket = login_ticket(&trusting, &email, &auth_key);
    assert_wrong(&totp(&trusting, &ticket, &wrong(&secret), ATTACKER));
    assert_wrong(&totp(&trusting, &ticket, &wrong(&secret), USER));
}

/// The per-address limit of the auth endpoints (20 a minute, burst 10,
/// here a burst of 3) keys an untrusted peer by its own address whatever
/// `X-Forwarded-For` says, and a trusted proxy's clients each by theirs.
#[test]
fn the_auth_limit_keys_clients_by_the_trusted_forwarded_address_only() {
    let limits = RateLimits {
        auth_burst: 3,
        auth_per_minute: 1,
        ..RateLimits::generous()
    };
    let prelogin = |server: &Server, forwarded: &str| {
        post(
            server,
            "/v1/auth/prelogin",
            &[("x-forwarded-for", forwarded)],
            json!({"email": "someone@example.com"}),
        )
        .status
    };
    let untrusted = Server::with(Options {
        rate_limits: limits,
        ..Options::default()
    });
    let statuses: Vec<u16> = (0..4)
        .map(|i| prelogin(&untrusted, &format!("198.51.100.{i}")))
        .collect();
    assert_eq!(statuses, [200, 200, 200, 429], "one bucket: the peer");
    let trusting = proxied(limits);
    let statuses: Vec<u16> = (0..4)
        .map(|i| prelogin(&trusting, &format!("198.51.100.{i}")))
        .collect();
    assert_eq!(statuses, [200, 200, 200, 200], "a bucket per client");
}

// ---------------------------------------------------------------------------
// Failed sign-ins per email and network (D178)
// ---------------------------------------------------------------------------

fn login_from(server: &Server, email: &str, key: &[u8], ip: &str) -> RawAnswer {
    raw_http(
        server,
        "POST",
        "/v1/auth/login",
        &[
            ("content-type", "application/json"),
            ("x-forwarded-for", ip),
        ],
        &login_body(email, key),
    )
}

/// Cross review (D178): someone who knows only the email cannot lock the
/// owner out. Ten wrong keys from one network close that network's window
/// (15 minutes); the network the owner signed in from before still signs
/// in at once, and a network never seen waits at most the soft delay of
/// the email (here 2 s), then signs in.
#[test]
fn wrong_keys_from_one_network_do_not_lock_the_owner_out() {
    let server = proxied(RateLimits::generous());
    let (email, auth_key, _) = register(&server);
    let ok = login_from(&server, &email, &auth_key, USER);
    assert_eq!(ok.status, 200, "{}", ok.body);
    for _ in 0..10 {
        let answer = login_from(&server, &email, &[7; 32], ATTACKER);
        assert_eq!(
            answer.code(),
            Some("invalid_credentials"),
            "{}",
            answer.body
        );
    }
    let closed = login_from(&server, &email, &auth_key, ATTACKER);
    assert_delayed(&closed, 800..=900);
    // The owner's network: no wait at all.
    let ok = login_from(&server, &email, &auth_key, USER);
    assert_eq!(ok.status, 200, "{}", ok.body);
    // A new network: the soft delay of the email, never more.
    let fresh = "203.0.113.50";
    let wait = assert_delayed(&login_from(&server, &email, &auth_key, fresh), 1..=2);
    wait_out(wait);
    let ok = login_from(&server, &email, &auth_key, fresh);
    assert_eq!(ok.status, 200, "{}", ok.body);
}

/// Review finding (D178): an attacker who takes every turn of the soft
/// limit holds new networks, but never the account's own devices (a
/// sign-in with the id of one of its devices) nor its signed-in requests.
#[test]
fn the_soft_signin_limit_never_holds_the_accounts_devices_or_sessions() {
    let server = proxied(RateLimits::generous());
    let (email, auth_key, token) = register(&server);
    // The owner's device signs in once (its id is the account's now).
    let device = uuid::Uuid::now_v7();
    let body = |key: &[u8]| {
        json!({"email": email, "auth_key": b64(key), "device_id": device})
            .to_string()
            .into_bytes()
    };
    let headers = |ip: &'static str| {
        [
            ("content-type", "application/json"),
            ("x-forwarded-for", ip),
        ]
    };
    let ok = raw_http(
        &server,
        "POST",
        "/v1/auth/login",
        &headers(USER),
        &body(&auth_key),
    );
    assert_eq!(ok.status, 200, "{}", ok.body);
    for _ in 0..10 {
        login_from(&server, &email, &[7; 32], ATTACKER);
    }
    // A new network: held by the soft delay...
    assert_delayed(
        &login_from(&server, &email, &auth_key, "203.0.113.60"),
        1..=2,
    );
    // ...but not the owner's device there, nor a signed-in request.
    let ok = raw_http(
        &server,
        "POST",
        "/v1/auth/login",
        &headers("203.0.113.61"),
        &body(&auth_key),
    );
    assert_eq!(ok.status, 200, "the account's device: {}", ok.body);
    let setup = post(
        &server,
        "/v1/account/totp/setup",
        &[
            ("authorization", &format!("Bearer {token}")),
            ("x-forwarded-for", "203.0.113.62"),
        ],
        json!({"current_auth_key": b64(&auth_key)}),
    );
    assert_eq!(setup.status, 200, "a signed-in request: {}", setup.body);
}

/// Review finding (D174): the account endpoints take at most the 1 MiB
/// body of the auth endpoints, so the requests waiting for an Argon2id
/// slot hold at most that much each.
#[test]
fn account_endpoints_refuse_bodies_over_1_mib() {
    let server = Server::start();
    let (_, _, token) = register(&server);
    let big = json!({"current_auth_key": "A".repeat(2 << 20)}).to_string();
    for path in [
        "/v1/account/totp/setup",
        "/v1/account/email",
        "/v1/account/password",
    ] {
        let answer = raw_http(
            &server,
            "POST",
            path,
            &[
                ("content-type", "application/json"),
                ("authorization", &format!("Bearer {token}")),
            ],
            big.as_bytes(),
        );
        assert_eq!(answer.status, 413, "{path}: {}", answer.body);
    }
}
