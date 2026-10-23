//! Rows of the server schema and the helpers every operation shares
//! (docs/sync-protocol.md, section 14).

use shellpp_sync_api::api::{AccountBundle, Kdf, PersonalVaultState, Quota, Tokens};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::db::{Backend, DbError, Tx, Ty, Val, time_text};
use crate::error::{ApiError, ApiResult};
use crate::secrets;

/// Now, at millisecond precision (both backends store milliseconds).
pub fn now() -> OffsetDateTime {
    let now = OffsetDateTime::now_utc();
    now.replace_nanosecond(now.millisecond() as u32 * 1_000_000)
        .unwrap_or(now)
}

/// The day of `t` at 00:00 UTC (device `last_seen_at`, day precision).
pub fn day_of(t: OffsetDateTime) -> OffsetDateTime {
    t.replace_time(time::Time::MIDNIGHT)
}

/// RFC 3339 UTC with milliseconds, the wire form of timestamps.
pub fn wire_time(t: OffsetDateTime) -> String {
    time_text(t)
}

/// The access token lifetime (section 4.6).
pub const ACCESS_LIFETIME: time::Duration = time::Duration::minutes(15);
/// The refresh token lifetime.
pub const REFRESH_LIFETIME: time::Duration = time::Duration::days(30);
/// A used refresh token is accepted once more within this time (D93).
pub const REFRESH_RETRY: time::Duration = time::Duration::seconds(60);
/// Login ticket: 5 minutes; recovery ticket: 10 minutes; 5 attempts each.
pub const LOGIN_TICKET_LIFETIME: time::Duration = time::Duration::minutes(5);
pub const RECOVERY_TICKET_LIFETIME: time::Duration = time::Duration::minutes(10);
pub const TICKET_ATTEMPTS: i64 = 5;
/// Refresh token hashes of a deleted account are kept 30 days (D93).
pub const DELETED_ACCOUNT_DAYS: i64 = 30;

/// A wrap blob of a 32-byte key (docs/storage-format.md, section 6.1).
pub const WRAP_LEN: usize = 73;

/// A `users` row.
#[derive(Clone)]
pub struct User {
    pub user_id: Uuid,
    pub email: String,
    pub email_verified: bool,
    pub auth_verifier: String,
    pub recovery_verifier: String,
    pub kdf: Kdf,
    pub kdf_salt: Vec<u8>,
    pub personal_vault_id: Uuid,
    pub kek_wrap: Vec<u8>,
    pub recovery_wrap: Vec<u8>,
    pub x25519_public_key: Vec<u8>,
    pub ssh_id_public_key: String,
    pub totp_secret_enc: Option<Vec<u8>>,
    pub totp_enabled: bool,
    pub totp_last_step: Option<u64>,
    pub disabled: bool,
    pub quota_max_records: Option<u64>,
    pub quota_max_bytes: Option<u64>,
    pub created_at: OffsetDateTime,
}

impl std::fmt::Debug for User {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("User")
            .field("user_id", &self.user_id)
            .finish_non_exhaustive()
    }
}

pub const USER_COLS: &str = "user_id, email, email_verified, auth_verifier, recovery_verifier, \
    kdf, kdf_salt, personal_vault_id, kek_wrap, recovery_wrap, x25519_public_key, \
    ssh_id_public_key, totp_secret_enc, totp_enabled, totp_last_step, disabled, \
    quota_max_records, quota_max_bytes, created_at";

pub const USER_TYPES: [Ty; 19] = [
    Ty::Uuid,
    Ty::Text,
    Ty::Bool,
    Ty::Text,
    Ty::Text,
    Ty::Json,
    Ty::Bytes,
    Ty::Uuid,
    Ty::Bytes,
    Ty::Bytes,
    Ty::Bytes,
    Ty::Text,
    Ty::Bytes,
    Ty::Bool,
    Ty::Int,
    Ty::Bool,
    Ty::Int,
    Ty::Int,
    Ty::Time,
];

fn corrupt() -> DbError {
    DbError::Failed("unexpected row".to_owned())
}

impl User {
    pub fn from_row(row: &[Val]) -> Result<User, DbError> {
        let int_opt = |v: &Val| v.int().and_then(|v| u64::try_from(v).ok());
        Ok(User {
            user_id: row[0].uuid().ok_or_else(corrupt)?,
            email: row[1].text().ok_or_else(corrupt)?.to_owned(),
            email_verified: row[2].bool().ok_or_else(corrupt)?,
            auth_verifier: row[3].text().ok_or_else(corrupt)?.to_owned(),
            recovery_verifier: row[4].text().ok_or_else(corrupt)?.to_owned(),
            kdf: serde_json::from_value(row[5].json().ok_or_else(corrupt)?.clone())
                .map_err(|_| corrupt())?,
            kdf_salt: row[6].bytes().ok_or_else(corrupt)?.to_vec(),
            personal_vault_id: row[7].uuid().ok_or_else(corrupt)?,
            kek_wrap: row[8].bytes().ok_or_else(corrupt)?.to_vec(),
            recovery_wrap: row[9].bytes().ok_or_else(corrupt)?.to_vec(),
            x25519_public_key: row[10].bytes().ok_or_else(corrupt)?.to_vec(),
            ssh_id_public_key: row[11].text().ok_or_else(corrupt)?.to_owned(),
            totp_secret_enc: row[12].bytes().map(<[u8]>::to_vec),
            totp_enabled: row[13].bool().ok_or_else(corrupt)?,
            totp_last_step: int_opt(&row[14]),
            disabled: row[15].bool().ok_or_else(corrupt)?,
            quota_max_records: int_opt(&row[16]),
            quota_max_bytes: int_opt(&row[17]),
            created_at: row[18].time().ok_or_else(corrupt)?,
        })
    }
}

pub fn user_by_email(tx: &mut dyn Tx, email: &str) -> Result<Option<User>, DbError> {
    tx.query_one(
        &format!("SELECT {USER_COLS} FROM users WHERE email = ?"),
        &[email.into()],
        &USER_TYPES,
    )?
    .map(|row| User::from_row(&row))
    .transpose()
}

/// [`user_by_id`] with the row locked in a PostgreSQL write transaction:
/// the account operations and token issuance of one user take turns.
pub fn user_by_id_locked(tx: &mut dyn Tx, user_id: Uuid) -> Result<Option<User>, DbError> {
    let sql = format!(
        "SELECT {USER_COLS} FROM users WHERE user_id = ?{}",
        tx.for_update()
    );
    tx.query_one(&sql, &[user_id.into()], &USER_TYPES)?
        .map(|row| User::from_row(&row))
        .transpose()
}

pub fn user_by_id(tx: &mut dyn Tx, user_id: Uuid) -> Result<Option<User>, DbError> {
    tx.query_one(
        &format!("SELECT {USER_COLS} FROM users WHERE user_id = ?"),
        &[user_id.into()],
        &USER_TYPES,
    )?
    .map(|row| User::from_row(&row))
    .transpose()
}

/// The current epoch of a vault.
pub fn vault_epoch(tx: &mut dyn Tx, vault_id: Uuid) -> Result<Option<u32>, DbError> {
    Ok(tx
        .query_one(
            "SELECT current_epoch FROM vaults WHERE vault_id = ?",
            &[vault_id.into()],
            &[Ty::Int],
        )?
        .and_then(|row| row[0].int())
        .and_then(|v| u32::try_from(v).ok()))
}

/// The usage of an account: its live records and their envelope bytes in
/// every vault (D93).
pub fn usage(tx: &mut dyn Tx, user_id: Uuid) -> Result<(u64, u64), DbError> {
    let length = match tx.backend() {
        Backend::Sqlite => "length(envelope)",
        Backend::Postgres => "octet_length(envelope)",
    };
    let row = tx
        .query_one(
            &format!(
                "SELECT COUNT(*), COALESCE(SUM({length}), 0) FROM records \
                 WHERE created_by = ? AND NOT tombstone"
            ),
            &[user_id.into()],
            &[Ty::Int, Ty::Int],
        )?
        .ok_or_else(corrupt)?;
    Ok((
        row[0].int().unwrap_or(0).max(0) as u64,
        row[1].int().unwrap_or(0).max(0) as u64,
    ))
}

/// The limits that apply to an account: its own, else the server's.
pub fn limits_of(user: &User, defaults: (Option<u64>, Option<u64>)) -> (Option<u64>, Option<u64>) {
    (
        user.quota_max_records.or(defaults.0),
        user.quota_max_bytes.or(defaults.1),
    )
}

/// The account bundle (section 4.4); `quota` only while a limit applies.
pub fn bundle(
    tx: &mut dyn Tx,
    user: &User,
    defaults: (Option<u64>, Option<u64>),
) -> ApiResult<AccountBundle> {
    let epoch = vault_epoch(tx, user.personal_vault_id)?.ok_or_else(ApiError::internal)?;
    let (max_records, max_bytes) = limits_of(user, defaults);
    let quota = if max_records.is_some() || max_bytes.is_some() {
        let (records, bytes) = usage(tx, user.user_id)?;
        Some(Quota {
            max_records,
            max_bytes,
            records,
            bytes,
        })
    } else {
        None
    };
    Ok(AccountBundle {
        user_id: user.user_id,
        email: user.email.clone(),
        email_verified: user.email_verified,
        totp_enabled: user.totp_enabled,
        kdf: user.kdf.clone(),
        kdf_salt: shellpp_sync_api::api::B64(user.kdf_salt.clone()),
        personal_vault: PersonalVaultState {
            vault_id: user.personal_vault_id,
            epoch,
            kek_wrap: shellpp_sync_api::api::B64(user.kek_wrap.clone()),
            recovery_wrap: shellpp_sync_api::api::B64(user.recovery_wrap.clone()),
        },
        x25519_public_key: shellpp_sync_api::api::B64(user.x25519_public_key.clone()),
        ssh_id_public_key: user.ssh_id_public_key.clone(),
        created_at: wire_time(user.created_at),
        quota,
    })
}

/// Starts a new refresh family for a device: a refresh and an access token.
pub fn issue_family(
    tx: &mut dyn Tx,
    user_id: Uuid,
    device_id: Uuid,
    now: OffsetDateTime,
) -> Result<Tokens, DbError> {
    issue_in_family(tx, Uuid::now_v7(), user_id, device_id, now)
}

/// A refresh token and an access token in `family`.
pub fn issue_in_family(
    tx: &mut dyn Tx,
    family: Uuid,
    user_id: Uuid,
    device_id: Uuid,
    now: OffsetDateTime,
) -> Result<Tokens, DbError> {
    let refresh = secrets::new_token(shellpp_sync_api::api::token_prefix::REFRESH);
    let refresh_expires = now + REFRESH_LIFETIME;
    tx.exec(
        "INSERT INTO refresh_tokens (token_hash, family_id, device_id, user_id, created_at, \
         expires_at) VALUES (?, ?, ?, ?, ?, ?)",
        &[
            secrets::hash_token(&refresh).into(),
            family.into(),
            device_id.into(),
            user_id.into(),
            now.into(),
            refresh_expires.into(),
        ],
    )?;
    let (access, access_expires) = issue_access(tx, family, user_id, device_id, now)?;
    Ok(Tokens {
        access_token: shellpp_sync_api::api::Secret(access),
        access_expires_at: wire_time(access_expires),
        refresh_token: shellpp_sync_api::api::Secret(refresh),
        refresh_expires_at: wire_time(refresh_expires),
    })
}

/// A new access token in `family`.
pub fn issue_access(
    tx: &mut dyn Tx,
    family: Uuid,
    user_id: Uuid,
    device_id: Uuid,
    now: OffsetDateTime,
) -> Result<(String, OffsetDateTime), DbError> {
    let access = secrets::new_token(shellpp_sync_api::api::token_prefix::ACCESS);
    let expires = now + ACCESS_LIFETIME;
    tx.exec(
        "INSERT INTO access_tokens (token_hash, family_id, device_id, user_id, expires_at) \
         VALUES (?, ?, ?, ?, ?)",
        &[
            secrets::hash_token(&access).into(),
            family.into(),
            device_id.into(),
            user_id.into(),
            expires.into(),
        ],
    )?;
    Ok((access, expires))
}

/// Revokes every refresh family of a user and deletes every access token
/// and ticket (password change, email change, recovery, reissue).
pub fn revoke_all(tx: &mut dyn Tx, user_id: Uuid, now: OffsetDateTime) -> Result<(), DbError> {
    tx.exec(
        "UPDATE refresh_tokens SET revoked_at = ? WHERE user_id = ? AND revoked_at IS NULL",
        &[now.into(), user_id.into()],
    )?;
    tx.exec(
        "DELETE FROM access_tokens WHERE user_id = ?",
        &[user_id.into()],
    )?;
    tx.exec(
        "DELETE FROM auth_tickets WHERE user_id = ?",
        &[user_id.into()],
    )?;
    Ok(())
}

/// Revokes one refresh family and deletes its access tokens.
pub fn revoke_family(tx: &mut dyn Tx, family: Uuid, now: OffsetDateTime) -> Result<(), DbError> {
    tx.exec(
        "UPDATE refresh_tokens SET revoked_at = ? WHERE family_id = ? AND revoked_at IS NULL",
        &[now.into(), family.into()],
    )?;
    tx.exec(
        "DELETE FROM access_tokens WHERE family_id = ?",
        &[family.into()],
    )?;
    Ok(())
}

/// Creates or refreshes the device row of a sign-in; a revoked device is
/// restored (the sign-in proved the password again, section 4.7). A device
/// id of another user is `422` with `details.field = "device_id"`.
pub fn upsert_device(
    tx: &mut dyn Tx,
    device_id: Uuid,
    user_id: Uuid,
    now: OffsetDateTime,
) -> ApiResult<()> {
    let owner = tx.query_one(
        "SELECT user_id FROM devices WHERE device_id = ?",
        &[device_id.into()],
        &[Ty::Uuid],
    )?;
    match owner {
        Some(row) if row[0].uuid() != Some(user_id) => Err(ApiError::validation("device_id")),
        Some(_) => {
            tx.exec(
                "UPDATE devices SET revoked_at = NULL, last_seen_at = ? WHERE device_id = ?",
                &[day_of(now).into(), device_id.into()],
            )?;
            Ok(())
        }
        None => {
            tx.exec(
                "INSERT INTO devices (device_id, user_id, created_at, last_seen_at) \
                 VALUES (?, ?, ?, ?)",
                &[
                    device_id.into(),
                    user_id.into(),
                    now.into(),
                    day_of(now).into(),
                ],
            )?;
            Ok(())
        }
    }
}

/// A device that may be used for `user_id` (not another user's).
pub fn device_free_for(tx: &mut dyn Tx, device_id: Uuid, user_id: Option<Uuid>) -> ApiResult<()> {
    let owner = tx.query_one(
        "SELECT user_id FROM devices WHERE device_id = ?",
        &[device_id.into()],
        &[Ty::Uuid],
    )?;
    match owner {
        Some(row) if row[0].uuid() != user_id => Err(ApiError::validation("device_id")),
        _ => Ok(()),
    }
}

/// Checks the size of a binary field (`400 bad_request` otherwise).
pub fn exact_len(bytes: &[u8], len: usize) -> ApiResult<()> {
    if bytes.len() == len {
        Ok(())
    } else {
        Err(ApiError::bad_request("binary field of a wrong size"))
    }
}
