//! Authentication and account operations (docs/sync-protocol.md, sections
//! 4 and 5). Every function runs inside one database transaction.

use axum::http::StatusCode;
use shellpp_sync_api::api::{
    AccountDeleteRequest, B64, CredentialsChangeRequest, CurrentAuth, Device, DeviceList,
    EmailChangeRequest, EmailChangeResponse, LoginRequest, LoginResponse, LoginTotpRequest,
    PasswordChangeRequest, PersonalKeyEpochRequest, PreloginRequest, PreloginResponse,
    RecoverBeginRequest, RecoverBeginResponse, RecoverCompleteRequest, RecoveryKeyReissueRequest,
    RecoveryKeyReissueResponse, RegisterRequest, RegisterResponse, Registration, Rotation, Secret,
    SshIdRequest, Tokens, TotpCode, TotpDisableRequest, TotpSetupResponse, code, token_prefix,
};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::db::{DbError, Tx, Ty, Val};
use crate::error::{ApiError, ApiResult};
use crate::model::{
    self, DELETED_ACCOUNT_DAYS, LOGIN_TICKET_LIFETIME, RECOVERY_TICKET_LIFETIME, REFRESH_LIFETIME,
    REFRESH_RETRY, TICKET_ATTEMPTS, User, WRAP_LEN, exact_len, issue_family, revoke_all,
    upsert_device, user_by_email, user_by_id,
};
use crate::records;
use crate::secrets;
use crate::state::Shared;

/// The identity of an authenticated request.
#[derive(Clone, Copy, Debug)]
pub struct Principal {
    pub user_id: Uuid,
    pub device_id: Uuid,
    pub family_id: Uuid,
}

fn quota_defaults(shared: &Shared) -> (Option<u64>, Option<u64>) {
    (
        shared.config.quota_max_records,
        shared.config.quota_max_bytes,
    )
}

fn rate_limited(seconds: u64) -> ApiError {
    ApiError::rate_limited(seconds)
}

/// Checks `current_auth_key` (or a login's `auth_key`) of a known user,
/// counting a failure for the email (section 10.1).
fn check_auth_key(shared: &Shared, user: &User, key: &[u8]) -> ApiResult<()> {
    shared
        .limiters
        .signin_allowed(&user.email)
        .map_err(rate_limited)?;
    if key.len() == 32 && secrets::check_verifier(&user.auth_verifier, key) {
        Ok(())
    } else {
        shared.limiters.signin_failed(&user.email);
        Err(ApiError::invalid_credentials())
    }
}

/// The auth key check for an email that may be unknown: a dummy Argon2id
/// verification keeps the timing alike (section 4.4).
fn check_login(shared: &Shared, user: Option<&User>, email: &str, key: &[u8]) -> ApiResult<()> {
    shared
        .limiters
        .signin_allowed(email)
        .map_err(rate_limited)?;
    let ok = match user {
        Some(user) if key.len() == 32 => secrets::check_verifier(&user.auth_verifier, key),
        // A wrong length runs the dummy verification too: the timing must
        // not tell a known email from an unknown one (section 4.4).
        Some(_) => {
            let _ = secrets::check_verifier(&shared.dummy_verifier, key);
            false
        }
        None => {
            let _ = secrets::check_verifier(&shared.dummy_verifier, key);
            false
        }
    };
    if ok {
        Ok(())
    } else {
        shared.limiters.signin_failed(email);
        Err(ApiError::invalid_credentials())
    }
}

fn ticket_expired() -> ApiError {
    ApiError::new(
        StatusCode::UNAUTHORIZED,
        code::TICKET_EXPIRED,
        "ticket expired",
    )
}

fn invalid_totp() -> ApiError {
    ApiError::new(
        StatusCode::UNAUTHORIZED,
        code::INVALID_TOTP,
        "invalid TOTP code",
    )
}

/// `POST /v1/auth/prelogin`.
pub fn prelogin(
    tx: &mut dyn Tx,
    shared: &Shared,
    request: PreloginRequest,
) -> ApiResult<PreloginResponse> {
    let email = secrets::normalize_email(&request.email);
    match user_by_email(tx, &email)? {
        Some(user) => Ok(PreloginResponse {
            kdf: user.kdf,
            kdf_salt: B64(user.kdf_salt),
        }),
        None => Ok(PreloginResponse {
            kdf: shellpp_sync_api::api::Kdf::desktop(),
            kdf_salt: B64(shared.keys.fake_salt(&email).to_vec()),
        }),
    }
}

/// `POST /v1/auth/register` (section 4.3), checks in their order.
pub fn register(
    tx: &mut dyn Tx,
    shared: &Shared,
    request: RegisterRequest,
    now: OffsetDateTime,
) -> ApiResult<RegisterResponse> {
    // 1. The registration code, before anything about emails.
    let mut code_hash = None;
    if shared.config.registration == Registration::Invite {
        let closed = || {
            ApiError::new(
                StatusCode::FORBIDDEN,
                code::REGISTRATION_CLOSED,
                "registration needs an invitation code",
            )
        };
        let code = request.registration_code.as_ref().ok_or_else(closed)?;
        let hash = secrets::hash_token(&secrets::canonical_code(&code.0));
        let sql = format!(
            "SELECT uses_left, expires_at FROM registration_codes WHERE code_hash = ?{}",
            tx.for_update()
        );
        let row = tx.query_one(&sql, &[hash.clone().into()], &[Ty::Int, Ty::Time])?;
        match row {
            Some(row)
                if row[0].int().unwrap_or(0) > 0 && row[1].time().is_some_and(|t| t > now) => {}
            _ => return Err(closed()),
        }
        code_hash = Some(hash);
    }
    // 2. KDF bounds and binary sizes.
    if !request.kdf.server_acceptable() {
        return Err(ApiError::weak_kdf());
    }
    exact_len(request.kdf_salt.as_bytes(), 16)?;
    exact_len(request.auth_key.as_bytes(), 32)?;
    exact_len(request.recovery_auth_key.as_bytes(), 32)?;
    exact_len(request.personal_vault.kek_wrap.as_bytes(), WRAP_LEN)?;
    exact_len(request.personal_vault.recovery_wrap.as_bytes(), WRAP_LEN)?;
    exact_len(request.x25519_public_key.as_bytes(), 32)?;
    for link in &request.personal_vault.epoch_chain {
        exact_len(link.prev_key_wrap.as_bytes(), WRAP_LEN)?;
    }
    if request.ssh_id_public_key.len() > 512 {
        return Err(ApiError::bad_request("ssh_id_public_key too long"));
    }
    // 3. The email.
    let email = secrets::normalize_email(&request.email);
    if email.is_empty() || email.len() > 320 || !email.contains('@') {
        return Err(ApiError::validation("email"));
    }
    if user_by_email(tx, &email)?.is_some() {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            code::EMAIL_TAKEN,
            "email already registered",
        ));
    }
    // 4. The vault, its epochs and the device.
    let vault = &request.personal_vault;
    if vault.epoch < 1 {
        return Err(ApiError::validation("personal_vault.epoch"));
    }
    if model::vault_epoch(tx, vault.vault_id)?.is_some() {
        return Err(ApiError::validation("personal_vault.vault_id"));
    }
    // The length first: `epoch` comes from the request (no allocation
    // follows its value).
    if vault.epoch_chain.len() as u64 != u64::from(vault.epoch) - 1 {
        return Err(ApiError::validation("personal_vault.epoch_chain"));
    }
    let mut chain: Vec<u32> = vault.epoch_chain.iter().map(|link| link.epoch).collect();
    chain.sort_unstable();
    let expected: Vec<u32> = (2..=vault.epoch).collect();
    if chain != expected {
        return Err(ApiError::validation("personal_vault.epoch_chain"));
    }
    model::device_free_for(tx, request.device_id, None)?;
    // 5. Create everything.
    let user_id = Uuid::now_v7();
    tx.exec(
        "INSERT INTO users (user_id, email, email_verified, auth_verifier, recovery_verifier, \
         kdf, kdf_salt, personal_vault_id, kek_wrap, recovery_wrap, x25519_public_key, \
         ssh_id_public_key, created_at, credentials_changed_at) \
         VALUES (?, ?, FALSE, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        &[
            user_id.into(),
            email.clone().into(),
            secrets::make_verifier(request.auth_key.as_bytes()).into(),
            secrets::make_verifier(request.recovery_auth_key.as_bytes()).into(),
            serde_json::to_value(&request.kdf)
                .map_err(|_| ApiError::internal())?
                .into(),
            request.kdf_salt.0.clone().into(),
            vault.vault_id.into(),
            vault.kek_wrap.0.clone().into(),
            vault.recovery_wrap.0.clone().into(),
            request.x25519_public_key.0.clone().into(),
            request.ssh_id_public_key.clone().into(),
            now.into(),
            now.into(),
        ],
    )
    // Two registrations of one email at once: the check of step 3 saw
    // neither, the unique email of the second insert answers.
    .map_err(|error| match error {
        DbError::Unique => ApiError::new(
            StatusCode::CONFLICT,
            code::EMAIL_TAKEN,
            "email already registered",
        ),
        other => other.into(),
    })?;
    tx.exec(
        "INSERT INTO vaults (vault_id, kind, current_epoch, created_by, created_at) \
         VALUES (?, 'personal', ?, ?, ?)",
        &[
            vault.vault_id.into(),
            i64::from(vault.epoch).into(),
            user_id.into(),
            now.into(),
        ],
    )?;
    tx.exec(
        "INSERT INTO vault_members (vault_id, user_id, role, added_by, added_at) \
         VALUES (?, ?, 'owner', ?, ?)",
        &[
            vault.vault_id.into(),
            user_id.into(),
            user_id.into(),
            now.into(),
        ],
    )?;
    tx.exec(
        "INSERT INTO vault_key_epochs (vault_id, epoch, prev_key_wrap, created_by, created_at) \
         VALUES (?, 1, NULL, ?, ?)",
        &[vault.vault_id.into(), user_id.into(), now.into()],
    )?;
    for link in &vault.epoch_chain {
        tx.exec(
            "INSERT INTO vault_key_epochs (vault_id, epoch, prev_key_wrap, created_by, created_at) \
             VALUES (?, ?, ?, ?, ?)",
            &[
                vault.vault_id.into(),
                i64::from(link.epoch).into(),
                link.prev_key_wrap.0.clone().into(),
                user_id.into(),
                now.into(),
            ],
        )?;
    }
    upsert_device(tx, request.device_id, user_id, now)?;
    let tokens = issue_family(tx, user_id, request.device_id, now)?;
    if let Some(hash) = code_hash {
        tx.exec(
            "UPDATE registration_codes SET uses_left = uses_left - 1 WHERE code_hash = ?",
            &[hash.into()],
        )?;
    }
    tracing::info!(%user_id, "account registered");
    Ok(RegisterResponse { user_id, tokens })
}

/// `POST /v1/auth/login` (section 4.4).
pub fn login(
    tx: &mut dyn Tx,
    shared: &Shared,
    request: LoginRequest,
    now: OffsetDateTime,
) -> ApiResult<LoginResponse> {
    let email = secrets::normalize_email(&request.email);
    let user = user_by_email(tx, &email)?;
    check_login(shared, user.as_ref(), &email, request.auth_key.as_bytes())?;
    let user = user.ok_or_else(ApiError::invalid_credentials)?;
    if user.disabled {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            code::ACCOUNT_DISABLED,
            "account disabled",
        ));
    }
    model::device_free_for(tx, request.device_id, Some(user.user_id))?;
    if user.totp_enabled {
        let ticket = secrets::new_token(token_prefix::LOGIN_TICKET);
        tx.exec(
            "INSERT INTO auth_tickets (ticket_hash, kind, user_id, device_id, expires_at) \
             VALUES (?, 'login', ?, ?, ?)",
            &[
                secrets::hash_token(&ticket).into(),
                user.user_id.into(),
                request.device_id.into(),
                (now + LOGIN_TICKET_LIFETIME).into(),
            ],
        )?;
        return Ok(LoginResponse {
            status: Some("totp_required".to_owned()),
            login_ticket: Some(Secret(ticket)),
            tokens: None,
            account: None,
        });
    }
    finish_login(tx, shared, &user, request.device_id, now, true)
}

fn finish_login(
    tx: &mut dyn Tx,
    shared: &Shared,
    user: &User,
    device_id: Uuid,
    now: OffsetDateTime,
    with_status: bool,
) -> ApiResult<LoginResponse> {
    upsert_device(tx, device_id, user.user_id, now)?;
    let tokens = issue_family(tx, user.user_id, device_id, now)?;
    let account = model::bundle(tx, user, quota_defaults(shared))?;
    Ok(LoginResponse {
        status: with_status.then(|| "ok".to_owned()),
        login_ticket: None,
        tokens: Some(tokens),
        account: Some(account),
    })
}

/// A ticket of `kind`: its user and device; `401 ticket_expired` when it is
/// unknown, expired or out of attempts.
fn ticket(
    tx: &mut dyn Tx,
    token: &str,
    kind: &str,
    now: OffsetDateTime,
) -> ApiResult<(Vec<u8>, Uuid, Uuid, i64)> {
    let hash = secrets::hash_token(token);
    let sql = format!(
        "SELECT user_id, device_id, attempts, expires_at FROM auth_tickets \
         WHERE ticket_hash = ? AND kind = ?{}",
        tx.for_update()
    );
    let row = tx
        .query_one(
            &sql,
            &[hash.clone().into(), kind.into()],
            &[Ty::Uuid, Ty::Uuid, Ty::Int, Ty::Time],
        )?
        .ok_or_else(ticket_expired)?;
    let attempts = row[2].int().unwrap_or(0);
    if row[3].time().is_none_or(|t| t <= now) || attempts >= TICKET_ATTEMPTS {
        tx.exec(
            "DELETE FROM auth_tickets WHERE ticket_hash = ?",
            &[hash.into()],
        )?;
        return Err(ticket_expired());
    }
    Ok((
        hash,
        row[0].uuid().ok_or_else(ApiError::internal)?,
        row[1].uuid().ok_or_else(ApiError::internal)?,
        attempts,
    ))
}

/// Checks a TOTP code for a user with a ticket: a wrong code counts an
/// attempt (the 5th deletes the ticket) and an account failure.
fn check_ticket_totp(
    tx: &mut dyn Tx,
    shared: &Shared,
    user: &User,
    ticket_hash: &[u8],
    attempts: i64,
    code: &str,
    now: OffsetDateTime,
) -> ApiResult<u64> {
    shared
        .limiters
        .totp_allowed(user.user_id)
        .map_err(rate_limited)?;
    let secret = user
        .totp_secret_enc
        .as_ref()
        .and_then(|blob| shared.keys.open_totp(user.user_id, blob))
        .ok_or_else(ApiError::internal)?;
    match secrets::check_totp(
        &secret,
        code,
        now.unix_timestamp().max(0) as u64,
        user.totp_last_step,
    ) {
        Some(step) => Ok(step),
        None => {
            shared.limiters.totp_failed(user.user_id);
            if attempts + 1 >= TICKET_ATTEMPTS {
                tx.exec(
                    "DELETE FROM auth_tickets WHERE ticket_hash = ?",
                    &[ticket_hash.into()],
                )?;
            } else {
                tx.exec(
                    "UPDATE auth_tickets SET attempts = attempts + 1 WHERE ticket_hash = ?",
                    &[ticket_hash.into()],
                )?;
            }
            Err(invalid_totp())
        }
    }
}

/// `POST /v1/auth/login/totp`.
pub fn login_totp(
    tx: &mut dyn Tx,
    shared: &Shared,
    request: LoginTotpRequest,
    now: OffsetDateTime,
) -> ApiResult<LoginResponse> {
    let (hash, user_id, device_id, attempts) = ticket(tx, &request.login_ticket.0, "login", now)?;
    let user = model::user_by_id_locked(tx, user_id)?.ok_or_else(ticket_expired)?;
    // The account may have changed during the ticket's 5 minutes.
    if user.disabled {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            code::ACCOUNT_DISABLED,
            "account disabled",
        ));
    }
    if user.totp_secret_enc.is_none() {
        tx.exec(
            "DELETE FROM auth_tickets WHERE ticket_hash = ?",
            &[hash.into()],
        )?;
        return Err(ticket_expired());
    }
    let step = check_ticket_totp(tx, shared, &user, &hash, attempts, &request.code.0, now)?;
    tx.exec(
        "UPDATE users SET totp_last_step = ? WHERE user_id = ?",
        &[(step as i64).into(), user_id.into()],
    )?;
    tx.exec(
        "DELETE FROM auth_tickets WHERE ticket_hash = ?",
        &[hash.into()],
    )?;
    finish_login(tx, shared, &user, device_id, now, false)
}

fn refresh_reused() -> ApiError {
    ApiError::new(
        StatusCode::UNAUTHORIZED,
        code::REFRESH_TOKEN_REUSED,
        "refresh token reused; the family is revoked",
    )
}

/// `POST /v1/auth/token/refresh` (section 4.6, with the retry of D93).
pub fn refresh(tx: &mut dyn Tx, token: &Secret, now: OffsetDateTime) -> ApiResult<Tokens> {
    let hash = secrets::hash_token(&token.0);
    // The user row, then the token row, are locked (PostgreSQL): two
    // refreshes with one token, or a refresh racing a password change or a
    // revocation (which lock the user row first too), take turns.
    let owner = tx.query_one(
        "SELECT user_id FROM refresh_tokens WHERE token_hash = ?",
        &[hash.clone().into()],
        &[Ty::Uuid],
    )?;
    if let Some(user_id) = owner.and_then(|row| row[0].uuid()) {
        model::user_by_id_locked(tx, user_id)?;
    }
    let sql = format!(
        "SELECT family_id, device_id, user_id, expires_at, used_at, revoked_at, \
         successor_hash, retried_at FROM refresh_tokens WHERE token_hash = ?{}",
        tx.for_update()
    );
    let row = tx.query_one(
        &sql,
        &[hash.clone().into()],
        &[
            Ty::Uuid,
            Ty::Uuid,
            Ty::Uuid,
            Ty::Time,
            Ty::Time,
            Ty::Time,
            Ty::Bytes,
            Ty::Time,
        ],
    )?;
    let Some(row) = row else {
        // A deleted account's token (D93).
        let deleted = tx.query_one(
            "SELECT deleted_at FROM deleted_account_tokens WHERE token_hash = ?",
            &[hash.into()],
            &[Ty::Time],
        )?;
        return Err(match deleted.and_then(|row| row[0].time()) {
            Some(at) => ApiError::new(
                StatusCode::UNAUTHORIZED,
                code::ACCOUNT_DELETED,
                "account deleted",
            )
            .detail("deleted_at", at.date().to_string()),
            None => ApiError::unauthenticated(),
        });
    };
    let family = row[0].uuid().ok_or_else(ApiError::internal)?;
    let device = row[1].uuid().ok_or_else(ApiError::internal)?;
    let user = row[2].uuid().ok_or_else(ApiError::internal)?;
    if row[3].time().is_none_or(|t| t <= now) || !row[5].is_null() {
        return Err(ApiError::unauthenticated());
    }
    let device_ok = tx
        .query_one(
            "SELECT d.revoked_at, u.disabled FROM devices d JOIN users u ON u.user_id = d.user_id \
             WHERE d.device_id = ?",
            &[device.into()],
            &[Ty::Time, Ty::Bool],
        )?
        .is_some_and(|row| row[0].is_null() && row[1].bool() == Some(false));
    if !device_ok {
        return Err(ApiError::unauthenticated());
    }
    if let Some(used_at) = row[4].time() {
        // The one retry of a lost response: used at most 60 s ago, its
        // successor unused, no retry before.
        let successor = row[6].bytes().map(<[u8]>::to_vec);
        let retried = !row[7].is_null();
        let successor_unused = match &successor {
            Some(successor) => tx
                .query_one(
                    "SELECT used_at, revoked_at FROM refresh_tokens WHERE token_hash = ?",
                    &[successor.clone().into()],
                    &[Ty::Time, Ty::Time],
                )?
                .is_some_and(|row| row[0].is_null() && row[1].is_null()),
            None => false,
        };
        if now - used_at <= REFRESH_RETRY && !retried && successor_unused {
            let successor = successor.expect("checked");
            // Presenting the unused successor later is then a reuse.
            tx.exec(
                "UPDATE refresh_tokens SET used_at = ? WHERE token_hash = ?",
                &[now.into(), successor.into()],
            )?;
            let tokens = model::issue_in_family(tx, family, user, device, now)?;
            tx.exec(
                "UPDATE refresh_tokens SET retried_at = ?, successor_hash = ? WHERE token_hash = ?",
                &[
                    now.into(),
                    secrets::hash_token(&tokens.refresh_token.0).into(),
                    hash.into(),
                ],
            )?;
            return Ok(tokens);
        }
        model::revoke_family(tx, family, now)?;
        tracing::info!(%user, %device, "refresh token reused; family revoked");
        return Err(refresh_reused());
    }
    let tokens = model::issue_in_family(tx, family, user, device, now)?;
    tx.exec(
        "UPDATE refresh_tokens SET used_at = ?, successor_hash = ? WHERE token_hash = ?",
        &[
            now.into(),
            secrets::hash_token(&tokens.refresh_token.0).into(),
            hash.into(),
        ],
    )?;
    Ok(tokens)
}

/// `POST /v1/auth/token/revoke`: logs this device out (its family).
pub fn revoke(tx: &mut dyn Tx, token: &Secret, now: OffsetDateTime) -> ApiResult<()> {
    let hash = secrets::hash_token(&token.0);
    let family = tx
        .query_one(
            "SELECT family_id FROM refresh_tokens WHERE token_hash = ?",
            &[hash.into()],
            &[Ty::Uuid],
        )?
        .and_then(|row| row[0].uuid())
        .ok_or_else(ApiError::unauthenticated)?;
    model::revoke_family(tx, family, now)?;
    Ok(())
}

/// Resolves a bearer access token (section 4.6).
/// Why a bearer token did not authenticate: one the server never issued
/// (counted against the client IP, section 10.1), or one it knows that is
/// expired, of a revoked device or of a disabled account (an ordinary `401`
/// that every client meets when its access token runs out).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TokenRefusal {
    Unknown,
    Refused,
}

pub fn authenticate(tx: &mut dyn Tx, token: &str, now: OffsetDateTime) -> ApiResult<Principal> {
    authenticate_token(tx, token, now)?.map_err(|_| ApiError::unauthenticated())
}

/// [`authenticate`], telling an unknown token from a refused one.
pub fn authenticate_token(
    tx: &mut dyn Tx,
    token: &str,
    now: OffsetDateTime,
) -> ApiResult<Result<Principal, TokenRefusal>> {
    if !token.starts_with(token_prefix::ACCESS) {
        return Ok(Err(TokenRefusal::Unknown));
    }
    let row = tx.query_one(
        "SELECT a.user_id, a.device_id, a.family_id, a.expires_at, d.revoked_at, \
             u.disabled, d.last_seen_at FROM access_tokens a \
             JOIN devices d ON d.device_id = a.device_id \
             JOIN users u ON u.user_id = a.user_id WHERE a.token_hash = ?",
        &[secrets::hash_token(token).into()],
        &[
            Ty::Uuid,
            Ty::Uuid,
            Ty::Uuid,
            Ty::Time,
            Ty::Time,
            Ty::Bool,
            Ty::Time,
        ],
    )?;
    let Some(row) = row else {
        return Ok(Err(TokenRefusal::Unknown));
    };
    if row[3].time().is_none_or(|t| t <= now) || !row[4].is_null() || row[5].bool() != Some(false) {
        return Ok(Err(TokenRefusal::Refused));
    }
    let principal = Principal {
        user_id: row[0].uuid().ok_or_else(ApiError::internal)?,
        device_id: row[1].uuid().ok_or_else(ApiError::internal)?,
        family_id: row[2].uuid().ok_or_else(ApiError::internal)?,
    };
    // Last seen: at most once a day, day precision.
    let today = model::day_of(now);
    if row[6].time().is_none_or(|seen| seen < today) {
        tx.exec(
            "UPDATE devices SET last_seen_at = ? WHERE device_id = ?",
            &[today.into(), principal.device_id.into()],
        )?;
    }
    Ok(Ok(principal))
}

/// The signed-in user (or `401` when it is gone).
pub fn me(tx: &mut dyn Tx, principal: &Principal) -> ApiResult<User> {
    model::user_by_id_locked(tx, principal.user_id)?.ok_or_else(ApiError::unauthenticated)
}

/// `GET /v1/account`.
pub fn account(
    tx: &mut dyn Tx,
    shared: &Shared,
    principal: &Principal,
) -> ApiResult<shellpp_sync_api::api::AccountBundle> {
    let user = me(tx, principal)?;
    model::bundle(tx, &user, quota_defaults(shared))
}

/// Checks a rotation of the personal vault and applies it (sections 5.1,
/// 7.7): `epoch = current + 1`, wrap sizes; the epoch row, the recovery
/// wrap and the current epoch. `kek_wrap` is replaced by the caller.
fn apply_rotation(
    tx: &mut dyn Tx,
    user: &User,
    rotation: &Rotation,
    now: OffsetDateTime,
) -> ApiResult<()> {
    tx.lock_vault(user.personal_vault_id)?;
    let current = model::vault_epoch(tx, user.personal_vault_id)?.ok_or_else(ApiError::internal)?;
    if rotation.epoch != current + 1 {
        return Err(epoch_conflict(current));
    }
    exact_len(rotation.prev_key_wrap.as_bytes(), WRAP_LEN)?;
    exact_len(rotation.recovery_wrap.as_bytes(), WRAP_LEN)?;
    tx.exec(
        "INSERT INTO vault_key_epochs (vault_id, epoch, prev_key_wrap, created_by, created_at) \
         VALUES (?, ?, ?, ?, ?)",
        &[
            user.personal_vault_id.into(),
            i64::from(rotation.epoch).into(),
            rotation.prev_key_wrap.0.clone().into(),
            user.user_id.into(),
            now.into(),
        ],
    )?;
    tx.exec(
        "UPDATE vaults SET current_epoch = ?, rotation_pending = FALSE WHERE vault_id = ?",
        &[
            i64::from(rotation.epoch).into(),
            user.personal_vault_id.into(),
        ],
    )?;
    tx.exec(
        "UPDATE users SET recovery_wrap = ? WHERE user_id = ?",
        &[rotation.recovery_wrap.0.clone().into(), user.user_id.into()],
    )?;
    Ok(())
}

fn epoch_conflict(current: u32) -> ApiError {
    ApiError::new(
        StatusCode::CONFLICT,
        code::EPOCH_CONFLICT,
        "the epoch is not the current one plus one",
    )
    .detail("current_epoch", current)
}

/// `POST /v1/account/password` (section 5.1).
pub fn password_change(
    tx: &mut dyn Tx,
    shared: &Shared,
    principal: &Principal,
    request: PasswordChangeRequest,
    now: OffsetDateTime,
) -> ApiResult<Tokens> {
    let user = me(tx, principal)?;
    check_auth_key(shared, &user, request.current_auth_key.as_bytes())?;
    if !request.kdf.server_acceptable() {
        return Err(ApiError::weak_kdf());
    }
    exact_len(request.kdf_salt.as_bytes(), 16)?;
    exact_len(request.auth_key.as_bytes(), 32)?;
    exact_len(request.kek_wrap.as_bytes(), WRAP_LEN)?;
    if let Some(rotation) = &request.rotation {
        apply_rotation(tx, &user, rotation, now)?;
    }
    tx.exec(
        "UPDATE users SET auth_verifier = ?, kdf = ?, kdf_salt = ?, kek_wrap = ?, \
         credentials_changed_at = ? WHERE user_id = ?",
        &[
            secrets::make_verifier(request.auth_key.as_bytes()).into(),
            serde_json::to_value(&request.kdf)
                .map_err(|_| ApiError::internal())?
                .into(),
            request.kdf_salt.0.into(),
            request.kek_wrap.0.into(),
            now.into(),
            user.user_id.into(),
        ],
    )?;
    revoke_all(tx, user.user_id, now)?;
    let tokens = issue_family(tx, user.user_id, principal.device_id, now)?;
    tracing::info!(user_id = %user.user_id, "password changed");
    Ok(tokens)
}

/// `POST /v1/auth/recover/begin` (section 5.2).
pub fn recover_begin(
    tx: &mut dyn Tx,
    shared: &Shared,
    request: RecoverBeginRequest,
    now: OffsetDateTime,
) -> ApiResult<RecoverBeginResponse> {
    let email = secrets::normalize_email(&request.email);
    shared
        .limiters
        .signin_allowed(&email)
        .map_err(rate_limited)?;
    let user = user_by_email(tx, &email)?;
    let ok = match &user {
        Some(user) if request.recovery_auth_key.as_bytes().len() == 32 => secrets::check_verifier(
            &user.recovery_verifier,
            request.recovery_auth_key.as_bytes(),
        ),
        Some(_) | None => {
            let _ = secrets::check_verifier(
                &shared.dummy_verifier,
                request.recovery_auth_key.as_bytes(),
            );
            false
        }
    };
    let Some(user) = user.filter(|_| ok) else {
        shared.limiters.signin_failed(&email);
        return Err(ApiError::invalid_credentials());
    };
    // As at login: `account_disabled` only after a correct key.
    if user.disabled {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            code::ACCOUNT_DISABLED,
            "account disabled",
        ));
    }
    model::device_free_for(tx, request.device_id, Some(user.user_id))?;
    let ticket = secrets::new_token(token_prefix::RECOVERY_TICKET);
    tx.exec(
        "INSERT INTO auth_tickets (ticket_hash, kind, user_id, device_id, expires_at) \
         VALUES (?, 'recovery', ?, ?, ?)",
        &[
            secrets::hash_token(&ticket).into(),
            user.user_id.into(),
            request.device_id.into(),
            (now + RECOVERY_TICKET_LIFETIME).into(),
        ],
    )?;
    let epoch = model::vault_epoch(tx, user.personal_vault_id)?.ok_or_else(ApiError::internal)?;
    Ok(RecoverBeginResponse {
        recovery_ticket: Secret(ticket),
        email: user.email.clone(),
        recovery_wrap: B64(user.recovery_wrap.clone()),
        personal_vault_id: user.personal_vault_id,
        epoch,
        totp_required: user.totp_enabled,
    })
}

/// `POST /v1/auth/recover/complete` (section 5.2).
pub fn recover_complete(
    tx: &mut dyn Tx,
    shared: &Shared,
    request: RecoverCompleteRequest,
    now: OffsetDateTime,
) -> ApiResult<Tokens> {
    let (hash, user_id, device_id, attempts) =
        ticket(tx, &request.recovery_ticket.0, "recovery", now)?;
    if device_id != request.device_id {
        return Err(ticket_expired());
    }
    let user = user_by_id(tx, user_id)?.ok_or_else(ticket_expired)?;
    if user.totp_enabled {
        let code = request
            .totp_code
            .as_ref()
            .map(|code| code.0.as_str())
            .unwrap_or("");
        let step = check_ticket_totp(tx, shared, &user, &hash, attempts, code, now)?;
        tx.exec(
            "UPDATE users SET totp_last_step = ? WHERE user_id = ?",
            &[(step as i64).into(), user_id.into()],
        )?;
    }
    if secrets::normalize_email(&request.email) != user.email {
        return Err(ApiError::validation("email"));
    }
    if !request.kdf.server_acceptable() {
        return Err(ApiError::weak_kdf());
    }
    exact_len(request.kdf_salt.as_bytes(), 16)?;
    exact_len(request.auth_key.as_bytes(), 32)?;
    exact_len(request.kek_wrap.as_bytes(), WRAP_LEN)?;
    apply_rotation(tx, &user, &request.rotation, now)?;
    tx.exec(
        "UPDATE users SET auth_verifier = ?, kdf = ?, kdf_salt = ?, kek_wrap = ?, \
         credentials_changed_at = ? WHERE user_id = ?",
        &[
            secrets::make_verifier(request.auth_key.as_bytes()).into(),
            serde_json::to_value(&request.kdf)
                .map_err(|_| ApiError::internal())?
                .into(),
            request.kdf_salt.0.into(),
            request.kek_wrap.0.into(),
            now.into(),
            user.user_id.into(),
        ],
    )?;
    revoke_all(tx, user.user_id, now)?;
    upsert_device(tx, device_id, user.user_id, now)?;
    let tokens = issue_family(tx, user.user_id, device_id, now)?;
    tracing::info!(user_id = %user.user_id, "account recovered");
    Ok(tokens)
}

/// `POST /v1/account/recovery-key` (section 5.3): one transaction with the
/// rotation and the new `recovery_key` record.
pub fn recovery_key_reissue(
    tx: &mut dyn Tx,
    shared: &Shared,
    principal: &Principal,
    request: RecoveryKeyReissueRequest,
    now: OffsetDateTime,
) -> ApiResult<RecoveryKeyReissueResponse> {
    let user = me(tx, principal)?;
    check_auth_key(shared, &user, request.current_auth_key.as_bytes())?;
    exact_len(request.recovery_auth_key.as_bytes(), 32)?;
    exact_len(request.kek_wrap.as_bytes(), WRAP_LEN)?;
    apply_rotation(tx, &user, &request.rotation, now)?;
    let seq = records::write_reissue_record(
        tx,
        &user,
        principal,
        &request.record,
        request.rotation.epoch,
        now,
    )?;
    tx.exec(
        "UPDATE users SET recovery_verifier = ?, kek_wrap = ?, credentials_changed_at = ? \
         WHERE user_id = ?",
        &[
            secrets::make_verifier(request.recovery_auth_key.as_bytes()).into(),
            request.kek_wrap.0.into(),
            now.into(),
            user.user_id.into(),
        ],
    )?;
    revoke_all(tx, user.user_id, now)?;
    let tokens = issue_family(tx, user.user_id, principal.device_id, now)?;
    tracing::info!(user_id = %user.user_id, "recovery key reissued");
    Ok(RecoveryKeyReissueResponse { tokens, seq })
}

/// `POST /v1/account/credentials` (section 5.9): the password change of
/// section 5.1 with its rotation and the recovery key reissue of section
/// 5.3 in this one transaction. Every check runs before the first write
/// that a later check could leave behind, and any failure rolls back the
/// whole request: the old password and recovery key keep working.
pub fn credentials_change(
    tx: &mut dyn Tx,
    shared: &Shared,
    principal: &Principal,
    request: CredentialsChangeRequest,
    now: OffsetDateTime,
) -> ApiResult<RecoveryKeyReissueResponse> {
    let user = me(tx, principal)?;
    check_auth_key(shared, &user, request.current_auth_key.as_bytes())?;
    if !request.kdf.server_acceptable() {
        return Err(ApiError::weak_kdf());
    }
    exact_len(request.kdf_salt.as_bytes(), 16)?;
    exact_len(request.auth_key.as_bytes(), 32)?;
    exact_len(request.kek_wrap.as_bytes(), WRAP_LEN)?;
    exact_len(request.recovery_auth_key.as_bytes(), 32)?;
    apply_rotation(tx, &user, &request.rotation, now)?;
    let seq = records::write_reissue_record(
        tx,
        &user,
        principal,
        &request.record,
        request.rotation.epoch,
        now,
    )?;
    tx.exec(
        "UPDATE users SET auth_verifier = ?, kdf = ?, kdf_salt = ?, kek_wrap = ?, \
         recovery_verifier = ?, credentials_changed_at = ? WHERE user_id = ?",
        &[
            secrets::make_verifier(request.auth_key.as_bytes()).into(),
            serde_json::to_value(&request.kdf)
                .map_err(|_| ApiError::internal())?
                .into(),
            request.kdf_salt.0.into(),
            request.kek_wrap.0.into(),
            secrets::make_verifier(request.recovery_auth_key.as_bytes()).into(),
            now.into(),
            user.user_id.into(),
        ],
    )?;
    revoke_all(tx, user.user_id, now)?;
    let tokens = issue_family(tx, user.user_id, principal.device_id, now)?;
    tracing::info!(
        user_id = %user.user_id,
        "password changed and recovery key reissued"
    );
    Ok(RecoveryKeyReissueResponse { tokens, seq })
}

/// `POST /v1/account/email` without SMTP: the change takes effect at once
/// with the new key material (section 5.4).
pub fn email_change(
    tx: &mut dyn Tx,
    shared: &Shared,
    principal: &Principal,
    request: EmailChangeRequest,
    now: OffsetDateTime,
) -> ApiResult<EmailChangeResponse> {
    let user = me(tx, principal)?;
    check_auth_key(shared, &user, request.current_auth_key.as_bytes())?;
    let email = secrets::normalize_email(&request.new_email);
    if email.is_empty() || email.len() > 320 || !email.contains('@') {
        return Err(ApiError::validation("new_email"));
    }
    let (Some(kdf), Some(kdf_salt), Some(auth_key), Some(kek_wrap)) = (
        request.kdf,
        request.kdf_salt,
        request.auth_key,
        request.kek_wrap,
    ) else {
        // No SMTP: the material for the new email is required.
        return Err(ApiError::validation("auth_key"));
    };
    if email != user.email
        && let Some(other) = user_by_email(tx, &email)?
        && other.user_id != user.user_id
    {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            code::EMAIL_TAKEN,
            "email already registered",
        ));
    }
    if !kdf.server_acceptable() {
        return Err(ApiError::weak_kdf());
    }
    exact_len(kdf_salt.as_bytes(), 16)?;
    exact_len(auth_key.as_bytes(), 32)?;
    exact_len(kek_wrap.as_bytes(), WRAP_LEN)?;
    tx.exec(
        "UPDATE users SET email = ?, email_verified = FALSE, auth_verifier = ?, kdf = ?, \
         kdf_salt = ?, kek_wrap = ?, credentials_changed_at = ? WHERE user_id = ?",
        &[
            email.clone().into(),
            secrets::make_verifier(auth_key.as_bytes()).into(),
            serde_json::to_value(&kdf)
                .map_err(|_| ApiError::internal())?
                .into(),
            kdf_salt.0.into(),
            kek_wrap.0.into(),
            now.into(),
            user.user_id.into(),
        ],
    )?;
    tx.exec(
        "DELETE FROM email_codes WHERE user_id = ?",
        &[user.user_id.into()],
    )?;
    revoke_all(tx, user.user_id, now)?;
    let tokens = issue_family(tx, user.user_id, principal.device_id, now)?;
    Ok(EmailChangeResponse {
        email: Some(email),
        email_verified: Some(false),
        tokens: Some(tokens),
        verification: None,
    })
}

/// `POST /v1/account/totp/setup` (section 5.5).
pub fn totp_setup(
    tx: &mut dyn Tx,
    shared: &Shared,
    principal: &Principal,
    request: CurrentAuth,
) -> ApiResult<TotpSetupResponse> {
    let user = me(tx, principal)?;
    check_auth_key(shared, &user, request.current_auth_key.as_bytes())?;
    if user.totp_enabled {
        return Err(ApiError::validation("totp"));
    }
    let secret = secrets::new_totp_secret();
    tx.exec(
        "UPDATE users SET totp_secret_enc = ?, totp_enabled = FALSE WHERE user_id = ?",
        &[
            shared.keys.seal_totp(user.user_id, &secret).into(),
            user.user_id.into(),
        ],
    )?;
    Ok(TotpSetupResponse {
        secret: Secret(secrets::totp_secret_text(&secret)),
        otpauth_uri: Secret(secrets::otpauth_uri(&user.email, &secret)),
    })
}

/// `POST /v1/account/totp/enable`.
pub fn totp_enable(
    tx: &mut dyn Tx,
    shared: &Shared,
    principal: &Principal,
    request: TotpCode,
    now: OffsetDateTime,
) -> ApiResult<()> {
    let user = me(tx, principal)?;
    if user.totp_enabled {
        return Err(ApiError::validation("totp"));
    }
    shared
        .limiters
        .totp_allowed(user.user_id)
        .map_err(rate_limited)?;
    let secret = user
        .totp_secret_enc
        .as_ref()
        .and_then(|blob| shared.keys.open_totp(user.user_id, blob))
        .ok_or_else(|| ApiError::validation("totp"))?;
    let step = secrets::check_totp(
        &secret,
        &request.code.0,
        now.unix_timestamp().max(0) as u64,
        None,
    )
    .ok_or_else(|| {
        shared.limiters.totp_failed(user.user_id);
        invalid_totp()
    })?;
    tx.exec(
        "UPDATE users SET totp_enabled = TRUE, totp_last_step = ? WHERE user_id = ?",
        &[(step as i64).into(), user.user_id.into()],
    )?;
    Ok(())
}

/// `POST /v1/account/totp/disable`.
pub fn totp_disable(
    tx: &mut dyn Tx,
    shared: &Shared,
    principal: &Principal,
    request: TotpDisableRequest,
    now: OffsetDateTime,
) -> ApiResult<()> {
    let user = me(tx, principal)?;
    check_auth_key(shared, &user, request.current_auth_key.as_bytes())?;
    if user.totp_enabled {
        check_user_totp(shared, &user, &request.code.0, now)?;
    }
    tx.exec(
        "UPDATE users SET totp_secret_enc = NULL, totp_enabled = FALSE, totp_last_step = NULL \
         WHERE user_id = ?",
        &[user.user_id.into()],
    )?;
    Ok(())
}

/// A TOTP code of a signed-in user (no ticket): disable, account deletion.
fn check_user_totp(
    shared: &Shared,
    user: &User,
    code: &str,
    now: OffsetDateTime,
) -> ApiResult<u64> {
    shared
        .limiters
        .totp_allowed(user.user_id)
        .map_err(rate_limited)?;
    let secret = user
        .totp_secret_enc
        .as_ref()
        .and_then(|blob| shared.keys.open_totp(user.user_id, blob))
        .ok_or_else(ApiError::internal)?;
    secrets::check_totp(
        &secret,
        code,
        now.unix_timestamp().max(0) as u64,
        user.totp_last_step,
    )
    .ok_or_else(|| {
        shared.limiters.totp_failed(user.user_id);
        invalid_totp()
    })
}

/// `PUT /v1/account/ssh-id`.
pub fn ssh_id(tx: &mut dyn Tx, principal: &Principal, request: SshIdRequest) -> ApiResult<()> {
    if request.ssh_id_public_key.is_empty() || request.ssh_id_public_key.len() > 512 {
        return Err(ApiError::validation("ssh_id_public_key"));
    }
    tx.exec(
        "UPDATE users SET ssh_id_public_key = ? WHERE user_id = ?",
        &[request.ssh_id_public_key.into(), principal.user_id.into()],
    )?;
    Ok(())
}

/// `POST /v1/account/delete` (section 5.8): immediate; the refresh token
/// hashes stay 30 days so the other devices learn why (D93).
pub fn account_delete(
    tx: &mut dyn Tx,
    shared: &Shared,
    principal: &Principal,
    request: AccountDeleteRequest,
    now: OffsetDateTime,
) -> ApiResult<()> {
    let user = me(tx, principal)?;
    check_auth_key(shared, &user, request.current_auth_key.as_bytes())?;
    if user.totp_enabled {
        let code = request
            .totp_code
            .as_ref()
            .map(|code| code.0.as_str())
            .unwrap_or("");
        check_user_totp(shared, &user, code, now)?;
    }
    // Sole owner of a team vault with other members (D14).
    let sole: Vec<Val> = tx
        .query(
            "SELECT m.vault_id FROM vault_members m JOIN vaults v ON v.vault_id = m.vault_id \
             WHERE m.user_id = ? AND m.role = 'owner' AND v.kind = 'team' \
             AND NOT EXISTS (SELECT 1 FROM vault_members o WHERE o.vault_id = m.vault_id \
               AND o.user_id <> m.user_id AND o.role = 'owner') \
             AND EXISTS (SELECT 1 FROM vault_members o WHERE o.vault_id = m.vault_id \
               AND o.user_id <> m.user_id)",
            &[user.user_id.into()],
            &[Ty::Uuid],
        )?
        .into_iter()
        .map(|row| row[0].clone())
        .collect();
    if !sole.is_empty() {
        let ids: Vec<String> = sole
            .iter()
            .filter_map(Val::uuid)
            .map(|id| id.to_string())
            .collect();
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            code::SOLE_OWNER,
            "the account is the only owner of a shared vault",
        )
        .detail("vault_ids", ids));
    }
    tx.exec(
        "INSERT INTO deleted_account_tokens (token_hash, deleted_at) \
         SELECT token_hash, ? FROM refresh_tokens \
         WHERE user_id = ? AND revoked_at IS NULL AND expires_at > ?",
        &[now.into(), user.user_id.into(), now.into()],
    )?;
    delete_user(tx, user.user_id)?;
    tracing::info!(user_id = %user.user_id, "account deleted");
    Ok(())
}

/// Deletes a user, its personal vault and every row that belongs to it
/// (account deletion and `admin user delete`). Team vaults where it is the
/// only member go too; in the others its membership and wraps go (their
/// rotation is a stage 8 matter).
pub fn delete_user(tx: &mut dyn Tx, user_id: Uuid) -> ApiResult<()> {
    let personal = tx
        .query_one(
            "SELECT personal_vault_id FROM users WHERE user_id = ?",
            &[user_id.into()],
            &[Ty::Uuid],
        )?
        .and_then(|row| row[0].uuid());
    tx.exec(
        "DELETE FROM vaults WHERE kind = 'team' AND vault_id IN \
         (SELECT vault_id FROM vault_members WHERE user_id = ?) AND NOT EXISTS \
         (SELECT 1 FROM vault_members o WHERE o.vault_id = vaults.vault_id AND o.user_id <> ?)",
        &[user_id.into(), user_id.into()],
    )?;
    tx.exec(
        "UPDATE vaults SET rotation_pending = TRUE WHERE kind = 'team' AND vault_id IN \
         (SELECT vault_id FROM vault_members WHERE user_id = ?)",
        &[user_id.into()],
    )?;
    if let Some(vault_id) = personal {
        tx.exec("DELETE FROM vaults WHERE vault_id = ?", &[vault_id.into()])?;
    }
    tx.exec("DELETE FROM users WHERE user_id = ?", &[user_id.into()])?;
    Ok(())
}

/// `GET /v1/devices`.
pub fn devices(tx: &mut dyn Tx, principal: &Principal) -> ApiResult<DeviceList> {
    let rows = tx.query(
        "SELECT device_id, created_at, last_seen_at, revoked_at FROM devices \
         WHERE user_id = ? ORDER BY created_at",
        &[principal.user_id.into()],
        &[Ty::Uuid, Ty::Time, Ty::Time, Ty::Time],
    )?;
    let devices = rows
        .iter()
        .filter_map(|row| {
            let device_id = row[0].uuid()?;
            Some(Device {
                device_id,
                created_at: model::wire_time(row[1].time()?),
                last_seen_at: model::wire_time(row[2].time()?),
                revoked: !row[3].is_null(),
                current: device_id == principal.device_id,
            })
        })
        .collect();
    Ok(DeviceList { devices })
}

/// `DELETE /v1/devices/{device_id}`: revokes the device's tokens.
pub fn device_revoke(
    tx: &mut dyn Tx,
    principal: &Principal,
    device_id: Uuid,
    now: OffsetDateTime,
) -> ApiResult<()> {
    // The user row first, as a refresh does (no lost revocation).
    me(tx, principal)?;
    let changed = tx.exec(
        "UPDATE devices SET revoked_at = ? WHERE device_id = ? AND user_id = ?",
        &[now.into(), device_id.into(), principal.user_id.into()],
    )?;
    if changed == 0 {
        return Err(ApiError::not_found());
    }
    revoke_device_tokens(tx, device_id, now)?;
    tracing::info!(%device_id, user_id = %principal.user_id, "device revoked");
    Ok(())
}

/// Revokes every token of a device.
pub fn revoke_device_tokens(
    tx: &mut dyn Tx,
    device_id: Uuid,
    now: OffsetDateTime,
) -> ApiResult<()> {
    tx.exec(
        "UPDATE refresh_tokens SET revoked_at = ? WHERE device_id = ? AND revoked_at IS NULL",
        &[now.into(), device_id.into()],
    )?;
    tx.exec(
        "DELETE FROM access_tokens WHERE device_id = ?",
        &[device_id.into()],
    )?;
    tx.exec(
        "DELETE FROM auth_tickets WHERE device_id = ?",
        &[device_id.into()],
    )?;
    Ok(())
}

/// `POST /v1/vaults/{personal}/key-epochs` (sections 5.1 and 7.7): a new
/// epoch of the personal vault, also the re-post after a server restore.
pub fn personal_key_epoch(
    tx: &mut dyn Tx,
    shared: &Shared,
    principal: &Principal,
    vault_id: Uuid,
    request: PersonalKeyEpochRequest,
    now: OffsetDateTime,
) -> ApiResult<u32> {
    let user = me(tx, principal)?;
    if vault_id != user.personal_vault_id {
        return Err(ApiError::not_member());
    }
    check_auth_key(shared, &user, request.current_auth_key.as_bytes())?;
    exact_len(request.account_wraps.kek_wrap.as_bytes(), WRAP_LEN)?;
    apply_rotation(
        tx,
        &user,
        &Rotation {
            epoch: request.epoch,
            prev_key_wrap: request.prev_key_wrap,
            recovery_wrap: request.account_wraps.recovery_wrap,
        },
        now,
    )?;
    tx.exec(
        "UPDATE users SET kek_wrap = ? WHERE user_id = ?",
        &[request.account_wraps.kek_wrap.0.into(), user.user_id.into()],
    )?;
    Ok(request.epoch)
}

/// Purges what has expired (section 10.3); run daily by `serve` and by
/// `admin purge`. Returns the number of tombstones purged.
pub fn purge(tx: &mut dyn Tx, retention_days: u64, now: OffsetDateTime) -> ApiResult<u64> {
    let cutoff = now - time::Duration::days(retention_days.min(36_500) as i64);
    // Tombstones: raise each vault's min_valid_seq to its highest purged seq.
    let purged = tx.query(
        "SELECT vault_id, MAX(seq), COUNT(*) FROM records WHERE tombstone AND tombstoned_at < ? \
         GROUP BY vault_id",
        &[cutoff.into()],
        &[Ty::Uuid, Ty::Int, Ty::Int],
    )?;
    let mut count = 0u64;
    for row in &purged {
        let (Some(vault_id), Some(max_seq)) = (row[0].uuid(), row[1].int()) else {
            continue;
        };
        count += row[2].int().unwrap_or(0).max(0) as u64;
        tx.exec(
            "DELETE FROM records WHERE vault_id = ? AND tombstone AND tombstoned_at < ?",
            &[vault_id.into(), cutoff.into()],
        )?;
        tx.exec(
            "UPDATE vaults SET min_valid_seq = ? WHERE vault_id = ? AND min_valid_seq < ?",
            &[max_seq.into(), vault_id.into(), max_seq.into()],
        )?;
    }
    tx.exec(
        "DELETE FROM refresh_tokens WHERE expires_at < ? OR revoked_at IS NOT NULL",
        &[now.into()],
    )?;
    tx.exec(
        "DELETE FROM access_tokens WHERE expires_at < ?",
        &[now.into()],
    )?;
    tx.exec(
        "DELETE FROM auth_tickets WHERE expires_at < ?",
        &[now.into()],
    )?;
    tx.exec(
        "DELETE FROM email_codes WHERE expires_at < ?",
        &[now.into()],
    )?;
    tx.exec(
        "DELETE FROM registration_codes WHERE expires_at < ? OR uses_left <= 0",
        &[now.into()],
    )?;
    tx.exec(
        "DELETE FROM deleted_account_tokens WHERE deleted_at < ?",
        &[(now - time::Duration::days(DELETED_ACCOUNT_DAYS)).into()],
    )?;
    tx.exec(
        "UPDATE invitations SET status = 'expired' WHERE status IN ('pending', 'accepted') \
         AND expires_at < ?",
        &[now.into()],
    )?;
    tx.exec(
        "DELETE FROM invitations WHERE status IN ('confirmed', 'declined', 'revoked', 'expired') \
         AND expires_at < ?",
        &[(now - time::Duration::days(30)).into()],
    )?;
    let _ = REFRESH_LIFETIME;
    Ok(count)
}
