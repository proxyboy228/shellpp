//! The server's checks of what a client presents (docs/sync-protocol.md,
//! sections 4 and 5; docs/security.md, section 5.5): access and refresh
//! tokens, login and recovery tickets, TOTP codes and the TOTP secret at
//! rest, auth and recovery keys against their verifiers, registration codes
//! and the fake pre-login salt. Each is accepted as issued and refused,
//! with its precise error, when changed, expired, used up or presented
//! elsewhere. The operations run directly on the database with a chosen
//! `now`, committed as the API commits them; on PostgreSQL with
//! `SHELLPP_TEST_POSTGRES=docker`. Keys, tokens and secrets are generated at
//! run time.

mod support;

use std::sync::Arc;

use shellpp_server::accounts::{self, CheckedKey, KeyKind, Principal};
use shellpp_server::config::Config;
use shellpp_server::db::{DbError, Tx, TxKind, Ty, Val};
use shellpp_server::error::ApiResult;
use shellpp_server::ratelimit::RateLimits;
use shellpp_server::state::Shared;
use shellpp_server::{admin, model, secrets};
use shellpp_sync_api::api::{self as wire, B64, Registration, Secret, code, token_prefix};
use support::*;
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

/// The client address of the operations run directly (the TOTP delays of
/// D175 are kept per account and client network).
const IP: std::net::IpAddr = std::net::IpAddr::V4(std::net::Ipv4Addr::new(192, 0, 2, 1));

fn ms(n: i64) -> Duration {
    Duration::milliseconds(n)
}

fn s(n: i64) -> Duration {
    Duration::seconds(n)
}

/// The failure limits out of the way: the checks answer their own errors.
fn limits() -> RateLimits {
    RateLimits {
        failures_per_window: 1_000,
        totp_free_failures: 1_000,
        totp_soft_failures: 1_000,
        ..RateLimits::generous()
    }
}

struct Fixture {
    server: Server,
    shared: Arc<Shared>,
}

struct Account {
    email: String,
    user_id: Uuid,
    device_id: Uuid,
    auth_key: [u8; 32],
    recovery_auth_key: [u8; 32],
    kdf_salt: Vec<u8>,
    tokens: wire::Tokens,
}

fn register_request(
    email: &str,
    auth_key: &[u8],
    recovery_auth_key: &[u8],
    kdf_salt: &[u8],
    device_id: Uuid,
    registration_code: Option<&str>,
) -> wire::RegisterRequest {
    wire::RegisterRequest {
        email: email.to_owned(),
        registration_code: registration_code.map(|code| Secret(code.to_owned())),
        kdf: wire::Kdf::desktop(),
        kdf_salt: B64(kdf_salt.to_vec()),
        auth_key: B64(auth_key.to_vec()),
        recovery_auth_key: B64(recovery_auth_key.to_vec()),
        personal_vault: wire::PersonalVaultRegistration {
            vault_id: Uuid::now_v7(),
            epoch: 1,
            kek_wrap: B64(vec![4; 73]),
            recovery_wrap: B64(vec![5; 73]),
            epoch_chain: Vec::new(),
        },
        x25519_public_key: B64(vec![6; 32]),
        ssh_id_public_key: "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIK0 shellpp:x".into(),
        device_id,
    }
}

fn complete_request(
    email: &str,
    ticket: &str,
    device_id: Uuid,
    totp_code: Option<&str>,
) -> wire::RecoverCompleteRequest {
    wire::RecoverCompleteRequest {
        recovery_ticket: Secret(ticket.to_owned()),
        totp_code: totp_code.map(|code| Secret(code.to_owned())),
        email: email.to_owned(),
        kdf: wire::Kdf::desktop(),
        kdf_salt: B64(secrets::random_bytes::<16>().to_vec()),
        auth_key: B64(secrets::random_bytes::<32>().to_vec()),
        kek_wrap: B64(vec![4; 73]),
        rotation: wire::Rotation {
            epoch: 2,
            prev_key_wrap: B64(vec![9; 73]),
            recovery_wrap: B64(vec![10; 73]),
        },
        device_id,
    }
}

impl Fixture {
    fn new(registration: Registration) -> Fixture {
        let server = Server::with(Options {
            registration,
            rate_limits: limits(),
            ..Options::default()
        });
        let shared = Shared::new(server.db.clone(), server.config.clone());
        Fixture { server, shared }
    }

    /// The same database under another server secret.
    fn other_server(&self) -> Arc<Shared> {
        let config = Config {
            server_secret: secrets::random_bytes::<32>().to_vec().into(),
            ..self.server.config.clone()
        };
        Shared::new(self.server.db.clone(), config)
    }

    /// `f` in a write transaction, committed as the API commits it: on
    /// success and on the failures whose writes must stick (`api::sticky`).
    fn write_with<T>(
        &self,
        shared: &Shared,
        f: impl FnOnce(&mut dyn Tx, &Shared) -> ApiResult<T>,
    ) -> ApiResult<T> {
        self.server
            .db
            .tx(TxKind::Write, |tx| -> ApiResult<ApiResult<T>> {
                match f(tx, shared) {
                    Err(error)
                        if [
                            code::INVALID_TOTP,
                            code::TICKET_EXPIRED,
                            code::REFRESH_TOKEN_REUSED,
                        ]
                        .contains(&error.code) =>
                    {
                        Ok(Err(error))
                    }
                    other => other.map(Ok),
                }
            })
            .and_then(|result| result)
    }

    fn write<T>(&self, f: impl FnOnce(&mut dyn Tx, &Shared) -> ApiResult<T>) -> ApiResult<T> {
        self.write_with(&self.shared, f)
    }

    /// A registration as the API runs it: the verifiers outside, then the
    /// write transaction (D174).
    fn register_with(
        &self,
        request: wire::RegisterRequest,
        now: OffsetDateTime,
    ) -> ApiResult<wire::RegisterResponse> {
        let verifiers = accounts::register_verifiers(&self.shared, &request, now)?;
        self.write(|tx, shared| accounts::register(tx, shared, request, verifiers, now))
    }

    /// The key of a login or `recover/begin`, checked outside any
    /// transaction (D174).
    fn sign_in_key(&self, email: &str, kind: KeyKind, key: &[u8]) -> ApiResult<CheckedKey> {
        accounts::check_sign_in(&self.shared, &secrets::normalize_email(email), kind, key)
    }

    /// `f` with `key` checked as the `current_auth_key` of `principal`
    /// first, outside the transaction, as the API runs it (D174).
    fn signed_in<T>(
        &self,
        principal: &Principal,
        key: &[u8],
        f: impl FnOnce(&mut dyn Tx, &Shared, &CheckedKey) -> ApiResult<T>,
    ) -> ApiResult<T> {
        let checked = accounts::check_current_auth_key(&self.shared, principal, key)?;
        self.write(|tx, shared| f(tx, shared, &checked))
    }

    fn exec(&self, sql: &str, params: &[Val]) {
        self.server
            .db
            .tx(TxKind::Write, |tx| tx.exec(sql, params))
            .map_err(|error: DbError| error.to_string())
            .expect("statement");
    }

    fn register(&self, now: OffsetDateTime) -> Account {
        let email = email();
        let auth_key = secrets::random_bytes::<32>();
        let recovery_auth_key = secrets::random_bytes::<32>();
        let kdf_salt = secrets::random_bytes::<16>().to_vec();
        let device_id = Uuid::now_v7();
        let request = register_request(
            &email,
            &auth_key,
            &recovery_auth_key,
            &kdf_salt,
            device_id,
            None,
        );
        let response = self.register_with(request, now).expect("registered");
        Account {
            email,
            user_id: response.user_id,
            device_id,
            auth_key,
            recovery_auth_key,
            kdf_salt,
            tokens: response.tokens,
        }
    }

    fn authenticate(&self, token: &str, now: OffsetDateTime) -> ApiResult<Principal> {
        self.write(|tx, _| accounts::authenticate(tx, token, now))
    }

    fn refresh(&self, token: &str, now: OffsetDateTime) -> ApiResult<wire::Tokens> {
        self.write(|tx, _| accounts::refresh(tx, &Secret(token.to_owned()), now))
    }

    fn login(&self, email: &str, key: &[u8], device_id: Uuid) -> ApiResult<wire::LoginResponse> {
        let request = wire::LoginRequest {
            email: email.to_owned(),
            auth_key: B64(key.to_vec()),
            device_id,
        };
        let checked = self.sign_in_key(email, KeyKind::Auth, key)?;
        self.write(|tx, shared| accounts::login(tx, shared, &checked, request, model::now()))
    }

    fn login_ticket(&self, account: &Account) -> String {
        let response = self
            .login(&account.email, &account.auth_key, account.device_id)
            .expect("signed in");
        assert_eq!(response.status.as_deref(), Some("totp_required"));
        response.login_ticket.expect("a login ticket").0
    }

    fn login_totp(
        &self,
        ticket: &str,
        code: &str,
        now: OffsetDateTime,
    ) -> ApiResult<wire::LoginResponse> {
        let request = wire::LoginTotpRequest {
            login_ticket: Secret(ticket.to_owned()),
            code: Secret(code.to_owned()),
        };
        self.write(|tx, shared| accounts::login_totp(tx, shared, request, IP, now))
    }

    /// Sets TOTP up (not enabled yet): the principal and the secret.
    fn setup_totp(&self, account: &Account, now: OffsetDateTime) -> (Principal, [u8; 20]) {
        let principal = self
            .authenticate(&account.tokens.access_token.0, now)
            .expect("signed in");
        let setup = self
            .signed_in(&principal, &account.auth_key, |tx, shared, checked| {
                accounts::totp_setup(tx, shared, checked)
            })
            .expect("set up");
        let secret = data_encoding::BASE32_NOPAD
            .decode(setup.secret.0.as_bytes())
            .expect("base32")
            .try_into()
            .expect("20 bytes");
        (principal, secret)
    }

    fn enable_totp(
        &self,
        shared: &Shared,
        principal: &Principal,
        code: &str,
        now: OffsetDateTime,
    ) -> ApiResult<()> {
        let request = wire::TotpCode {
            code: Secret(code.to_owned()),
        };
        self.write_with(shared, |tx, shared| {
            accounts::totp_enable(tx, shared, principal, request, IP, now)
        })
    }

    /// TOTP set up and enabled with the code of `now`: the secret.
    fn with_totp(&self, account: &Account, now: OffsetDateTime) -> [u8; 20] {
        let (principal, secret) = self.setup_totp(account, now);
        self.enable_totp(&self.shared, &principal, &code_at(&secret, now), now)
            .expect("enabled");
        secret
    }

    fn totp_blob(&self, user_id: Uuid) -> Vec<u8> {
        self.server
            .db
            .tx(TxKind::Read, |tx| {
                tx.query_one(
                    "SELECT totp_secret_enc FROM users WHERE user_id = ?",
                    &[user_id.into()],
                    &[Ty::Bytes],
                )
            })
            .map_err(|error: DbError| error.to_string())
            .expect("query")
            .and_then(|row| row[0].bytes().map(<[u8]>::to_vec))
            .expect("a sealed secret")
    }

    fn set_totp_blob(&self, user_id: Uuid, blob: &[u8]) {
        self.exec(
            "UPDATE users SET totp_secret_enc = ? WHERE user_id = ?",
            &[blob.into(), user_id.into()],
        );
    }
}

/// The error of `result` is `status` with `code`.
#[track_caller]
fn refused<T: std::fmt::Debug>(result: ApiResult<T>, status: u16, code: &str) {
    match result {
        Err(error) => assert_eq!(
            (error.status.as_u16(), error.code),
            (status, code),
            "{}",
            error.message
        ),
        Ok(value) => panic!("expected {status} {code}, got {value:?}"),
    }
}

/// Forms of a token (ticket, code) that must all be refused: the first
/// character after the prefix and the last one changed, one character
/// less, one more, nothing.
fn tampered(token: &str, prefix: &str) -> Vec<String> {
    let change = |i: usize| {
        let mut bytes = token.as_bytes().to_vec();
        bytes[i] = if bytes[i] == b'A' { b'B' } else { b'A' };
        String::from_utf8(bytes).expect("ascii")
    };
    vec![
        change(prefix.len()),
        change(token.len() - 1),
        token[..token.len() - 1].to_owned(),
        format!("{token}A"),
        String::new(),
    ]
}

fn flipped(key: &[u8], i: usize) -> Vec<u8> {
    let mut key = key.to_vec();
    key[i] ^= 0x80;
    key
}

fn code_at(secret: &[u8; 20], t: OffsetDateTime) -> String {
    secrets::totp_code(secret, t.unix_timestamp() as u64)
}

/// A code that is none of the codes accepted at `now`.
fn wrong_code(secret: &[u8; 20], now: OffsetDateTime) -> String {
    let window: Vec<String> = [now - s(30), now, now + s(30)]
        .iter()
        .map(|t| code_at(secret, *t))
        .collect();
    (0..10)
        .map(|d| format!("{d}{d}{d}{d}{d}{d}"))
        .find(|code| !window.contains(code))
        .expect("a wrong code")
}

// ---------------------------------------------------------------------------
// Access tokens
// ---------------------------------------------------------------------------

/// `authenticate`: the token as issued (its SHA-256 is the lookup key),
/// behind its own prefix, until `expires_at` (excluded).
#[test]
fn access_tokens_are_found_by_their_exact_hash_until_they_expire() {
    let fx = Fixture::new(Registration::Open);
    let t0 = model::now();
    let a = fx.register(t0);
    let access = a.tokens.access_token.0.clone();
    let principal = fx.authenticate(&access, t0).expect("as issued");
    assert_eq!(
        (principal.user_id, principal.device_id),
        (a.user_id, a.device_id)
    );
    for wrong in tampered(&access, token_prefix::ACCESS) {
        refused(fx.authenticate(&wrong, t0), 401, code::UNAUTHENTICATED);
    }
    // Its random part behind another prefix or none; the refresh token, and
    // the refresh token's random part behind the access prefix.
    let body = &access[token_prefix::ACCESS.len()..];
    let refresh_body = &a.tokens.refresh_token.0[token_prefix::REFRESH.len()..];
    for wrong in [
        format!("{}{body}", token_prefix::REFRESH),
        format!("{}{body}", token_prefix::LOGIN_TICKET),
        body.to_owned(),
        a.tokens.refresh_token.0.clone(),
        format!("{}{refresh_body}", token_prefix::ACCESS),
    ] {
        refused(fx.authenticate(&wrong, t0), 401, code::UNAUTHENTICATED);
    }
    let expires = t0 + model::ACCESS_LIFETIME;
    fx.authenticate(&access, expires - ms(1))
        .expect("valid to its last millisecond");
    refused(
        fx.authenticate(&access, expires),
        401,
        code::UNAUTHENTICATED,
    );
    refused(
        fx.authenticate(&access, expires + Duration::days(1)),
        401,
        code::UNAUTHENTICATED,
    );
}

/// A revoked device or a disabled account stops an access token even while
/// its row is still there.
#[test]
fn access_tokens_of_a_revoked_device_or_a_disabled_account_are_refused() {
    let fx = Fixture::new(Registration::Open);
    let t0 = model::now();
    let a = fx.register(t0);
    let access = &a.tokens.access_token.0;
    fx.authenticate(access, t0).expect("valid");
    fx.exec(
        "UPDATE devices SET revoked_at = ? WHERE device_id = ?",
        &[t0.into(), a.device_id.into()],
    );
    refused(fx.authenticate(access, t0), 401, code::UNAUTHENTICATED);
    fx.exec(
        "UPDATE devices SET revoked_at = NULL WHERE device_id = ?",
        &[a.device_id.into()],
    );
    fx.authenticate(access, t0).expect("valid again");
    fx.exec(
        "UPDATE users SET disabled = TRUE WHERE user_id = ?",
        &[a.user_id.into()],
    );
    refused(fx.authenticate(access, t0), 401, code::UNAUTHENTICATED);
}

// ---------------------------------------------------------------------------
// Refresh tokens (section 4.6, D93)
// ---------------------------------------------------------------------------

/// `refresh` and `revoke` find a token by its exact hash only: a changed
/// token is `401 unauthenticated`, never a reuse, and revokes nothing.
#[test]
fn a_wrong_refresh_token_is_refused_and_revokes_nothing() {
    let fx = Fixture::new(Registration::Open);
    let t0 = model::now();
    let a = fx.register(t0);
    let refresh = a.tokens.refresh_token.0.clone();
    let body = &refresh[token_prefix::REFRESH.len()..];
    let mut wrong = tampered(&refresh, token_prefix::REFRESH);
    wrong.push(format!("{}{body}", token_prefix::ACCESS));
    wrong.push(a.tokens.access_token.0.clone());
    for token in &wrong {
        refused(fx.refresh(token, t0), 401, code::UNAUTHENTICATED);
        refused(
            fx.write(|tx, _| accounts::revoke(tx, &Secret(token.clone()), t0)),
            401,
            code::UNAUTHENTICATED,
        );
    }
    // The family is whole: its access token works and the token rotates.
    fx.authenticate(&a.tokens.access_token.0, t0)
        .expect("not revoked");
    let next = fx.refresh(&refresh, t0).expect("rotates");
    fx.authenticate(&next.access_token.0, t0)
        .expect("the new access token");
}

/// The one retry of a used refresh token lasts 60 seconds, the bound
/// included; a millisecond later the old token is a reuse that revokes the
/// whole family, access tokens too.
#[test]
fn the_retry_of_a_used_refresh_token_lasts_60_seconds() {
    let fx = Fixture::new(Registration::Open);
    let t0 = model::now();
    let retry = model::REFRESH_RETRY;

    let a = fx.register(t0);
    let first = a.tokens.refresh_token.0.clone();
    fx.refresh(&first, t0).expect("rotated");
    let retried = fx.refresh(&first, t0 + retry).expect("the retry at 60 s");
    fx.authenticate(&retried.access_token.0, t0 + retry)
        .expect("its access token");

    let b = fx.register(t0);
    let first = b.tokens.refresh_token.0.clone();
    let second = fx.refresh(&first, t0).expect("rotated");
    let late = t0 + retry + ms(1);
    refused(fx.refresh(&first, late), 401, code::REFRESH_TOKEN_REUSED);
    refused(
        fx.refresh(&second.refresh_token.0, late),
        401,
        code::UNAUTHENTICATED,
    );
    for access in [&b.tokens.access_token.0, &second.access_token.0] {
        refused(fx.authenticate(access, late), 401, code::UNAUTHENTICATED);
    }
}

/// The successor presented after the retry is a reuse (the retry marked it
/// used); so is a retry whose successor was already used.
#[test]
fn a_successor_after_the_retry_and_a_retry_after_the_successor_are_reuses() {
    let fx = Fixture::new(Registration::Open);
    let t0 = model::now();

    let a = fx.register(t0);
    let first = a.tokens.refresh_token.0.clone();
    let second = fx.refresh(&first, t0).expect("rotated");
    let retried = fx.refresh(&first, t0 + s(1)).expect("the retry");
    refused(
        fx.refresh(&second.refresh_token.0, t0 + s(2)),
        401,
        code::REFRESH_TOKEN_REUSED,
    );
    refused(
        fx.refresh(&retried.refresh_token.0, t0 + s(2)),
        401,
        code::UNAUTHENTICATED,
    );
    refused(
        fx.authenticate(&retried.access_token.0, t0 + s(2)),
        401,
        code::UNAUTHENTICATED,
    );

    let b = fx.register(t0);
    let first = b.tokens.refresh_token.0.clone();
    let second = fx.refresh(&first, t0).expect("rotated");
    let third = fx
        .refresh(&second.refresh_token.0, t0 + s(1))
        .expect("rotated again");
    refused(
        fx.refresh(&first, t0 + s(2)),
        401,
        code::REFRESH_TOKEN_REUSED,
    );
    refused(
        fx.refresh(&third.refresh_token.0, t0 + s(2)),
        401,
        code::UNAUTHENTICATED,
    );
}

/// An expired refresh token is `401 unauthenticated` and revokes nothing.
#[test]
fn an_expired_refresh_token_is_refused_without_revoking_its_family() {
    let fx = Fixture::new(Registration::Open);
    let t0 = model::now();
    let a = fx.register(t0);
    let refresh = &a.tokens.refresh_token.0;
    let expires = t0 + model::REFRESH_LIFETIME;
    refused(fx.refresh(refresh, expires), 401, code::UNAUTHENTICATED);
    refused(
        fx.refresh(refresh, expires + Duration::days(1)),
        401,
        code::UNAUTHENTICATED,
    );
    fx.refresh(refresh, expires - ms(1))
        .expect("valid to its last millisecond");
}

/// A deleted account's refresh tokens answer `account_deleted` with the
/// day for 30 days, then `unauthenticated`; only the exact token does.
#[test]
fn a_deleted_accounts_refresh_token_answers_account_deleted_for_30_days() {
    let fx = Fixture::new(Registration::Open);
    let t0 = model::now();
    let a = fx.register(t0);
    let principal = fx.authenticate(&a.tokens.access_token.0, t0).unwrap();
    let delete = |key: &[u8]| {
        let request = wire::AccountDeleteRequest {
            current_auth_key: B64(key.to_vec()),
            totp_code: None,
        };
        fx.signed_in(&principal, key, |tx, shared, checked| {
            accounts::account_delete(tx, shared, checked, request, IP, t0)
        })
    };
    refused(
        delete(&flipped(&a.auth_key, 0)),
        401,
        code::INVALID_CREDENTIALS,
    );
    fx.authenticate(&a.tokens.access_token.0, t0)
        .expect("nothing deleted");
    delete(&a.auth_key).expect("deleted");

    let refresh = &a.tokens.refresh_token.0;
    match fx.refresh(refresh, t0 + s(1)) {
        Err(error) => {
            assert_eq!(
                (error.status.as_u16(), error.code),
                (401, code::ACCOUNT_DELETED)
            );
            assert_eq!(
                error.details.get("deleted_at").and_then(|v| v.as_str()),
                Some(t0.date().to_string().as_str())
            );
        }
        Ok(_) => panic!("a deleted account's refresh token was accepted"),
    }
    for wrong in tampered(refresh, token_prefix::REFRESH) {
        refused(fx.refresh(&wrong, t0 + s(1)), 401, code::UNAUTHENTICATED);
    }
    let kept = t0 + Duration::days(model::DELETED_ACCOUNT_DAYS);
    fx.write(|tx, _| accounts::purge(tx, 90, kept))
        .expect("purged");
    refused(fx.refresh(refresh, kept), 401, code::ACCOUNT_DELETED);
    fx.write(|tx, _| accounts::purge(tx, 90, kept + ms(1)))
        .expect("purged");
    refused(
        fx.refresh(refresh, kept + ms(1)),
        401,
        code::UNAUTHENTICATED,
    );
}

// ---------------------------------------------------------------------------
// Auth keys, recovery keys and recovery tickets
// ---------------------------------------------------------------------------

/// Login and the `current_auth_key` checks accept only the 32-byte key of
/// the verifier; a password change replaces the verifier.
#[test]
fn auth_keys_are_checked_against_their_verifiers() {
    let fx = Fixture::new(Registration::Open);
    let t0 = model::now();
    let a = fx.register(t0);
    let b = fx.register(t0);
    let ok = fx
        .login(&a.email, &a.auth_key, a.device_id)
        .expect("the right key");
    assert_eq!(ok.status.as_deref(), Some("ok"));
    fx.login(
        &format!(" {} ", a.email.to_uppercase()),
        &a.auth_key,
        a.device_id,
    )
    .expect("the email is normalized");
    let wrong = [
        flipped(&a.auth_key, 0),
        flipped(&a.auth_key, 31),
        a.auth_key[..31].to_vec(),
        [&a.auth_key[..], &[0]].concat(),
        Vec::new(),
        a.recovery_auth_key.to_vec(),
        b.auth_key.to_vec(),
    ];
    for key in &wrong {
        refused(
            fx.login(&a.email, key, a.device_id),
            401,
            code::INVALID_CREDENTIALS,
        );
    }
    refused(
        fx.login(&format!("x{}", a.email), &a.auth_key, a.device_id),
        401,
        code::INVALID_CREDENTIALS,
    );

    let principal = fx.authenticate(&a.tokens.access_token.0, t0).unwrap();
    let new_key = secrets::random_bytes::<32>();
    let change = |current: &[u8]| {
        let request = wire::PasswordChangeRequest {
            current_auth_key: B64(current.to_vec()),
            kdf: wire::Kdf::desktop(),
            kdf_salt: B64(secrets::random_bytes::<16>().to_vec()),
            auth_key: B64(new_key.to_vec()),
            kek_wrap: B64(vec![4; 73]),
            rotation: None,
        };
        fx.write(|tx, shared| accounts::password_change(tx, shared, &principal, request, t0))
    };
    refused(change(&wrong[0]), 401, code::INVALID_CREDENTIALS);
    fx.login(&a.email, &a.auth_key, a.device_id)
        .expect("unchanged");
    change(&a.auth_key).expect("changed");
    refused(
        fx.login(&a.email, &a.auth_key, a.device_id),
        401,
        code::INVALID_CREDENTIALS,
    );
    fx.login(&a.email, &new_key, a.device_id)
        .expect("the new key");
}

/// D174: a key is checked outside the transaction that acts on it, so a
/// credential change can commit in between. The write then takes the user
/// only while the verifier is still the one the key matched; a change of
/// the other verifier does not matter.
#[test]
fn a_key_checked_before_a_credential_change_is_refused_after_it() {
    let fx = Fixture::new(Registration::Open);
    let t0 = model::now();
    let a = fx.register(t0);
    let principal = fx.authenticate(&a.tokens.access_token.0, t0).unwrap();
    let login = fx
        .sign_in_key(&a.email, KeyKind::Auth, &a.auth_key)
        .expect("the auth key");
    let recovery = fx
        .sign_in_key(&a.email, KeyKind::Recovery, &a.recovery_auth_key)
        .expect("the recovery key");
    let current = accounts::check_current_auth_key(&fx.shared, &principal, &a.auth_key)
        .expect("the current auth key");
    let set = |column: &str| {
        fx.exec(
            &format!("UPDATE users SET {column} = ? WHERE user_id = ?"),
            &[
                secrets::make_verifier(&secrets::random_bytes::<32>()).into(),
                a.user_id.into(),
            ],
        );
    };
    let login_with = |checked: &CheckedKey| {
        let request = wire::LoginRequest {
            email: a.email.clone(),
            auth_key: B64(a.auth_key.to_vec()),
            device_id: a.device_id,
        };
        fx.write(|tx, shared| accounts::login(tx, shared, checked, request, t0))
    };
    let begin_with = |checked: &CheckedKey| {
        let request = wire::RecoverBeginRequest {
            email: a.email.clone(),
            recovery_auth_key: B64(a.recovery_auth_key.to_vec()),
            device_id: a.device_id,
        };
        fx.write(|tx, _| accounts::recover_begin(tx, checked, request, t0))
    };
    // A reissue in between replaces the recovery verifier only.
    set("recovery_verifier");
    login_with(&login).expect("the auth verifier did not change");
    refused(begin_with(&recovery), 401, code::INVALID_CREDENTIALS);
    // A password change in between replaces the auth verifier.
    set("auth_verifier");
    refused(login_with(&login), 401, code::INVALID_CREDENTIALS);
    refused(
        fx.write(|tx, shared| accounts::totp_setup(tx, shared, &current)),
        401,
        code::INVALID_CREDENTIALS,
    );
    // The keys were right: no failure counts for the email.
    assert!(
        fx.shared
            .limiters
            .failures
            .check(&a.email, 1, std::time::Instant::now())
            .is_ok(),
        "no failure counted"
    );
    // A registration that will be refused is refused before its hashes.
    let taken = register_request(
        &a.email,
        &secrets::random_bytes::<32>(),
        &secrets::random_bytes::<32>(),
        &secrets::random_bytes::<16>(),
        Uuid::now_v7(),
        None,
    );
    refused(
        accounts::register_verifiers(&fx.shared, &taken, t0).map(|_| ()),
        409,
        code::EMAIL_TAKEN,
    );
    // The account deleted in between: a signed-in check is `unauthenticated`.
    let b = fx.register(t0);
    let principal = fx.authenticate(&b.tokens.access_token.0, t0).unwrap();
    let current = accounts::check_current_auth_key(&fx.shared, &principal, &b.auth_key)
        .expect("the current auth key");
    fx.write(|tx, _| accounts::delete_user(tx, b.user_id))
        .unwrap();
    refused(
        fx.write(|tx, shared| accounts::totp_setup(tx, shared, &current)),
        401,
        code::UNAUTHENTICATED,
    );
}

/// Recovery: only the recovery key opens a ticket; the ticket is found by
/// its exact hash, binds its device, is used up by the recovery and
/// expires after 10 minutes (then it is deleted).
#[test]
fn recovery_checks_the_key_the_ticket_and_its_device() {
    let fx = Fixture::new(Registration::Open);
    let t0 = model::now();
    let a = fx.register(t0);
    let device = Uuid::now_v7();
    let begin = |email: &str, key: &[u8], now: OffsetDateTime| {
        let request = wire::RecoverBeginRequest {
            email: email.to_owned(),
            recovery_auth_key: B64(key.to_vec()),
            device_id: device,
        };
        let checked = fx.sign_in_key(email, KeyKind::Recovery, key)?;
        fx.write(|tx, _| accounts::recover_begin(tx, &checked, request, now))
    };
    let complete = |ticket: &str, device_id: Uuid, now: OffsetDateTime| {
        let request = complete_request(&a.email, ticket, device_id, None);
        fx.write(|tx, shared| accounts::recover_complete(tx, shared, request, IP, now))
    };
    for key in [
        flipped(&a.recovery_auth_key, 0),
        flipped(&a.recovery_auth_key, 31),
        a.recovery_auth_key[..31].to_vec(),
        Vec::new(),
        a.auth_key.to_vec(),
    ] {
        refused(begin(&a.email, &key, t0), 401, code::INVALID_CREDENTIALS);
    }
    refused(
        begin(&format!("x{}", a.email), &a.recovery_auth_key, t0),
        401,
        code::INVALID_CREDENTIALS,
    );

    let ticket = begin(&a.email, &a.recovery_auth_key, t0)
        .expect("begun")
        .recovery_ticket
        .0;
    for wrong in tampered(&ticket, token_prefix::RECOVERY_TICKET) {
        refused(complete(&wrong, device, t0), 401, code::TICKET_EXPIRED);
    }
    refused(
        complete(&ticket, Uuid::now_v7(), t0),
        401,
        code::TICKET_EXPIRED,
    );
    let tokens = complete(&ticket, device, t0 + s(1)).expect("recovered");
    fx.authenticate(&tokens.access_token.0, t0 + s(1))
        .expect("signed in");
    refused(
        complete(&ticket, device, t0 + s(2)),
        401,
        code::TICKET_EXPIRED,
    );
    refused(
        fx.refresh(&a.tokens.refresh_token.0, t0 + s(2)),
        401,
        code::UNAUTHENTICATED,
    );

    let t1 = t0 + s(3);
    let ticket = begin(&a.email, &a.recovery_auth_key, t1)
        .expect("begun again")
        .recovery_ticket
        .0;
    refused(
        complete(&ticket, device, t1 + model::RECOVERY_TICKET_LIFETIME),
        401,
        code::TICKET_EXPIRED,
    );
    refused(complete(&ticket, device, t1), 401, code::TICKET_EXPIRED);
}

// ---------------------------------------------------------------------------
// Login tickets and TOTP (section 5.5)
// ---------------------------------------------------------------------------

/// A login ticket is found by its exact hash and kind; the step the enable
/// used and wrong codes count failures, four of them keep the ticket; the
/// right code finishes and uses the ticket and its step up.
#[test]
fn login_tickets_are_found_by_hash_and_kind_and_survive_four_wrong_codes() {
    let fx = Fixture::new(Registration::Open);
    let t0 = model::now();
    let a = fx.register(t0);
    let secret = fx.with_totp(&a, t0);
    let ticket = fx.login_ticket(&a);
    let next = code_at(&secret, t0 + s(30));
    for wrong in tampered(&ticket, token_prefix::LOGIN_TICKET) {
        refused(fx.login_totp(&wrong, &next, t0), 401, code::TICKET_EXPIRED);
    }
    let request = complete_request(&a.email, &ticket, a.device_id, Some(&next));
    refused(
        fx.write(|tx, shared| accounts::recover_complete(tx, shared, request, IP, t0)),
        401,
        code::TICKET_EXPIRED,
    );
    refused(
        fx.login_totp(&ticket, &code_at(&secret, t0), t0),
        401,
        code::INVALID_TOTP,
    );
    for _ in 0..3 {
        refused(
            fx.login_totp(&ticket, &wrong_code(&secret, t0), t0),
            401,
            code::INVALID_TOTP,
        );
    }
    let tokens = fx
        .login_totp(&ticket, &next, t0)
        .expect("the right code")
        .tokens
        .expect("tokens");
    fx.authenticate(&tokens.access_token.0, t0)
        .expect("signed in");
    refused(fx.login_totp(&ticket, &next, t0), 401, code::TICKET_EXPIRED);
    let again = fx.login_ticket(&a);
    refused(fx.login_totp(&again, &next, t0), 401, code::INVALID_TOTP);
}

/// The fifth wrong code deletes the ticket: the right code then finds none.
#[test]
fn the_fifth_wrong_code_deletes_the_login_ticket() {
    let fx = Fixture::new(Registration::Open);
    let t0 = model::now();
    let a = fx.register(t0);
    let secret = fx.with_totp(&a, t0);
    let next = code_at(&secret, t0 + s(30));
    let ticket = fx.login_ticket(&a);
    for _ in 0..5 {
        refused(
            fx.login_totp(&ticket, &wrong_code(&secret, t0), t0),
            401,
            code::INVALID_TOTP,
        );
    }
    refused(fx.login_totp(&ticket, &next, t0), 401, code::TICKET_EXPIRED);
    let ticket = fx.login_ticket(&a);
    fx.login_totp(&ticket, &next, t0)
        .expect("a new ticket works");
}

/// A login ticket lives 5 minutes, the bound excluded; an expired one is
/// deleted.
#[test]
fn login_tickets_expire_after_five_minutes() {
    let fx = Fixture::new(Registration::Open);
    let t0 = model::now();
    let a = fx.register(t0);
    let secret = fx.with_totp(&a, t0);
    let issued = model::now();
    let ticket = fx.login_ticket(&a);
    let expires = issued + model::LOGIN_TICKET_LIFETIME;
    refused(
        fx.login_totp(&ticket, &code_at(&secret, expires + s(1)), expires + s(1)),
        401,
        code::TICKET_EXPIRED,
    );
    refused(
        fx.login_totp(&ticket, &code_at(&secret, t0 + s(30)), t0),
        401,
        code::TICKET_EXPIRED,
    );
    let issued = model::now();
    let ticket = fx.login_ticket(&a);
    let before = issued + model::LOGIN_TICKET_LIFETIME - s(1);
    fx.login_totp(&ticket, &code_at(&secret, before), before)
        .expect("before the bound");
}

/// The codes of a signed-in user (enable, disable, account deletion): a
/// wrong code, a code out of the window, the step already used and no code
/// are `401 invalid_totp`; a wrong auth key is `invalid_credentials` first.
#[test]
fn signed_in_totp_checks_refuse_wrong_used_and_missing_codes() {
    let fx = Fixture::new(Registration::Open);
    let t0 = model::now();
    let a = fx.register(t0);
    let (principal, secret) = fx.setup_totp(&a, t0);
    for wrong in [wrong_code(&secret, t0), code_at(&secret, t0 - s(60))] {
        refused(
            fx.enable_totp(&fx.shared, &principal, &wrong, t0),
            401,
            code::INVALID_TOTP,
        );
    }
    fx.enable_totp(&fx.shared, &principal, &code_at(&secret, t0), t0)
        .expect("enabled");

    let disable = |key: &[u8], totp: &str| {
        let request = wire::TotpDisableRequest {
            current_auth_key: B64(key.to_vec()),
            code: Secret(totp.to_owned()),
        };
        fx.signed_in(&principal, key, |tx, shared, checked| {
            accounts::totp_disable(tx, shared, checked, request, IP, t0)
        })
    };
    let delete = |totp: Option<&str>| {
        let request = wire::AccountDeleteRequest {
            current_auth_key: B64(a.auth_key.to_vec()),
            totp_code: totp.map(|totp| Secret(totp.to_owned())),
        };
        fx.signed_in(&principal, &a.auth_key, |tx, shared, checked| {
            accounts::account_delete(tx, shared, checked, request, IP, t0)
        })
    };
    let next = code_at(&secret, t0 + s(30));
    refused(
        disable(&flipped(&a.auth_key, 0), &next),
        401,
        code::INVALID_CREDENTIALS,
    );
    for wrong in [
        wrong_code(&secret, t0),
        code_at(&secret, t0),
        code_at(&secret, t0 + s(60)),
    ] {
        refused(disable(&a.auth_key, &wrong), 401, code::INVALID_TOTP);
    }
    refused(delete(None), 401, code::INVALID_TOTP);
    refused(
        delete(Some(&wrong_code(&secret, t0))),
        401,
        code::INVALID_TOTP,
    );
    fx.authenticate(&a.tokens.access_token.0, t0)
        .expect("nothing deleted");
    disable(&a.auth_key, &next).expect("disabled with the right code");
    let login = fx
        .login(&a.email, &a.auth_key, a.device_id)
        .expect("signed in");
    assert_eq!(login.status.as_deref(), Some("ok"), "no code asked");
}

/// The TOTP secret at rest opens only for its user and under this server's
/// key: a flipped byte, another user's blob or another server secret make
/// the right code fail (`422` at enable, `500` at sign-in), never pass.
#[test]
fn a_tampered_or_transplanted_totp_secret_at_rest_is_refused() {
    let fx = Fixture::new(Registration::Open);
    let t0 = model::now();
    let a = fx.register(t0);
    let b = fx.register(t0);
    let (principal, secret) = fx.setup_totp(&a, t0);
    let (_, secret_b) = fx.setup_totp(&b, t0);
    let blob = fx.totp_blob(a.user_id);
    let blob_b = fx.totp_blob(b.user_id);
    assert_eq!((blob.len(), blob[0]), (1 + 24 + 20 + 16, 0x01));
    assert!(
        !blob.windows(20).any(|w| w == secret.as_slice()),
        "sealed, not plain"
    );
    let right = code_at(&secret, t0);

    fx.set_totp_blob(a.user_id, &blob_b);
    for candidate in [&right, &code_at(&secret_b, t0)] {
        refused(
            fx.enable_totp(&fx.shared, &principal, candidate, t0),
            422,
            code::VALIDATION_FAILED,
        );
    }
    for i in [0, 1, 24, 25, 44, 45, 60] {
        let mut tampered = blob.clone();
        tampered[i] ^= 0x01;
        fx.set_totp_blob(a.user_id, &tampered);
        refused(
            fx.enable_totp(&fx.shared, &principal, &right, t0),
            422,
            code::VALIDATION_FAILED,
        );
    }
    fx.set_totp_blob(a.user_id, &blob);
    refused(
        fx.enable_totp(&fx.other_server(), &principal, &right, t0),
        422,
        code::VALIDATION_FAILED,
    );
    fx.enable_totp(&fx.shared, &principal, &right, t0)
        .expect("enabled with the intact row");

    let ticket = fx.login_ticket(&a);
    let next = code_at(&secret, t0 + s(30));
    let mut tampered = blob.clone();
    tampered[40] ^= 0x01;
    fx.set_totp_blob(a.user_id, &tampered);
    refused(fx.login_totp(&ticket, &next, t0), 500, code::INTERNAL);
    fx.set_totp_blob(a.user_id, &blob_b);
    refused(
        fx.login_totp(&ticket, &code_at(&secret_b, t0 + s(30)), t0),
        500,
        code::INTERNAL,
    );
    fx.set_totp_blob(a.user_id, &blob);
    fx.login_totp(&ticket, &next, t0)
        .expect("the right code once the row is intact");
}

// ---------------------------------------------------------------------------
// Registration codes and the pre-login salt
// ---------------------------------------------------------------------------

/// The typed form of a code: lowercase, in groups, `l` and `o` for `1`, `0`.
fn typed_form(code: &str) -> String {
    let mut out = String::new();
    for (i, c) in code.chars().enumerate() {
        if i > 0 && i % 5 == 0 {
            out.push(if i % 10 == 0 { ' ' } else { '-' });
        }
        out.push(match c {
            '1' => 'l',
            '0' => 'o',
            c => c.to_ascii_lowercase(),
        });
    }
    out
}

/// Invite-only registration takes the exact code (in its typed forms),
/// before its expiry and while it has uses left; `403 registration_closed`
/// otherwise.
#[test]
fn registration_codes_are_checked_by_hash_expiry_and_uses() {
    let fx = Fixture::new(Registration::Invite);
    let invite = admin::invite(&fx.server.db, 2, 1, None).expect("invite");
    let now = model::now();
    let register = |given: Option<&str>, now: OffsetDateTime| {
        let request = register_request(
            &email(),
            &secrets::random_bytes::<32>(),
            &secrets::random_bytes::<32>(),
            &secrets::random_bytes::<16>(),
            Uuid::now_v7(),
            given,
        );
        fx.register_with(request, now)
    };
    refused(register(None, now), 403, code::REGISTRATION_CLOSED);
    for wrong in tampered(&invite, "") {
        refused(register(Some(&wrong), now), 403, code::REGISTRATION_CLOSED);
    }
    refused(
        register(Some(&invite), now + Duration::days(1) + s(1)),
        403,
        code::REGISTRATION_CLOSED,
    );
    let typed = typed_form(&invite);
    assert_ne!(typed, invite);
    register(Some(&typed), now).expect("the typed form");
    register(Some(&invite), now).expect("the second use");
    refused(register(Some(&invite), now), 403, code::REGISTRATION_CLOSED);
}

/// An unknown email gets the fake salt of its normalized form, stable and
/// per server secret (D11); a known email its own salt.
#[test]
fn unknown_emails_get_the_fake_salt_of_their_normalized_email() {
    let fx = Fixture::new(Registration::Open);
    let a = fx.register(model::now());
    let prelogin = |shared: &Shared, email: &str| {
        let request = wire::PreloginRequest {
            email: email.to_owned(),
        };
        fx.write_with(shared, |tx, shared| accounts::prelogin(tx, shared, request))
            .expect("prelogin")
            .kdf_salt
            .0
    };
    let unknown = email();
    let salt = prelogin(&fx.shared, &unknown);
    assert_eq!(salt, fx.shared.keys.fake_salt(&unknown).to_vec());
    assert_eq!(salt.len(), 16);
    assert_eq!(
        prelogin(&fx.shared, &format!("  {}  ", unknown.to_uppercase())),
        salt
    );
    assert_ne!(prelogin(&fx.shared, &email()), salt);
    assert_ne!(prelogin(&fx.other_server(), &unknown), salt);
    assert_eq!(prelogin(&fx.shared, &a.email), a.kdf_salt);
}
