//! The admin CLI (docs/architecture.md, section 10.2; docs/sync-protocol.md,
//! sections 3.4, 10): registration codes, accounts, devices, vaults, the
//! restore marker and the purge. It works on the database directly, with or
//! without a running server. Irreversible events are logged with ids only.

use uuid::Uuid;

use crate::accounts;
use crate::db::{Db, TxKind, Ty};
use crate::error::{ApiError, ApiResult};
use crate::model;
use crate::records;
use crate::secrets;

/// A user given as a UUID or an email.
fn find_user(tx: &mut dyn crate::db::Tx, who: &str) -> ApiResult<Uuid> {
    if let Ok(id) = Uuid::parse_str(who.trim()) {
        return model::user_by_id(tx, id)?
            .map(|user| user.user_id)
            .ok_or_else(ApiError::not_found);
    }
    model::user_by_email(tx, &secrets::normalize_email(who))?
        .map(|user| user.user_id)
        .ok_or_else(ApiError::not_found)
}

/// `admin user invite`: a registration code for an invite-only server
/// (7 days, one use by default). Returns the code; it is shown once.
pub fn invite(db: &Db, uses: u32, days: u32, note: Option<String>) -> ApiResult<String> {
    let code = secrets::new_code();
    let now = model::now();
    let hash = secrets::hash_token(&code);
    db.tx(TxKind::Write, |tx| -> ApiResult<()> {
        tx.exec(
            "INSERT INTO registration_codes (code_hash, note, uses_left, created_at, expires_at) \
             VALUES (?, ?, ?, ?, ?)",
            &[
                hash.into(),
                note.into(),
                i64::from(uses.max(1)).into(),
                now.into(),
                (now + time::Duration::days(i64::from(days.max(1)))).into(),
            ],
        )?;
        Ok(())
    })?;
    Ok(code)
}

/// `admin user delete`: deletes an account (also an account a restore
/// brought back, docs/security.md, section 14.4).
pub fn user_delete(db: &Db, who: &str) -> ApiResult<Uuid> {
    db.tx(TxKind::Write, |tx| {
        let user_id = find_user(tx, who)?;
        accounts::delete_user(tx, user_id)?;
        tracing::info!(%user_id, "account deleted by the admin");
        Ok(user_id)
    })
}

/// `admin user quota`: sets (`Some`) or clears (`None`) an account's limits.
pub fn user_quota(db: &Db, who: &str, records: Option<u64>, bytes: Option<u64>) -> ApiResult<Uuid> {
    db.tx(TxKind::Write, |tx| {
        let user_id = find_user(tx, who)?;
        tx.exec(
            "UPDATE users SET quota_max_records = ?, quota_max_bytes = ? WHERE user_id = ?",
            &[
                records.map(|v| v as i64).into(),
                bytes.map(|v| v as i64).into(),
                user_id.into(),
            ],
        )?;
        Ok(user_id)
    })
}

/// `admin user reset-totp`: the only way to reset a lost authenticator (D11).
pub fn user_reset_totp(db: &Db, who: &str) -> ApiResult<Uuid> {
    db.tx(TxKind::Write, |tx| {
        let user_id = find_user(tx, who)?;
        tx.exec(
            "UPDATE users SET totp_secret_enc = NULL, totp_enabled = FALSE, totp_last_step = NULL \
             WHERE user_id = ?",
            &[user_id.into()],
        )?;
        tracing::info!(%user_id, "TOTP reset by the admin");
        Ok(user_id)
    })
}

/// `admin user disable` / `enable` (`403 account_disabled` while disabled).
pub fn user_disable(db: &Db, who: &str, disabled: bool) -> ApiResult<Uuid> {
    db.tx(TxKind::Write, |tx| {
        let user_id = find_user(tx, who)?;
        tx.exec(
            "UPDATE users SET disabled = ? WHERE user_id = ?",
            &[disabled.into(), user_id.into()],
        )?;
        if disabled {
            model::revoke_all(tx, user_id, model::now())?;
        }
        tracing::info!(%user_id, disabled, "account state changed by the admin");
        Ok(user_id)
    })
}

/// `admin user list`: ids, emails and creation times (for the operator).
pub fn user_list(db: &Db) -> ApiResult<Vec<(Uuid, String, String)>> {
    db.tx(TxKind::Read, |tx| {
        let rows = tx.query(
            "SELECT user_id, email, created_at FROM users ORDER BY created_at",
            &[],
            &[Ty::Uuid, Ty::Text, Ty::Time],
        )?;
        Ok(rows
            .iter()
            .filter_map(|row| {
                Some((
                    row[0].uuid()?,
                    row[1].text()?.to_owned(),
                    model::wire_time(row[2].time()?),
                ))
            })
            .collect())
    })
}

/// `admin device revoke`.
pub fn device_revoke(db: &Db, device_id: Uuid) -> ApiResult<()> {
    db.tx(TxKind::Write, |tx| {
        let now = model::now();
        let changed = tx.exec(
            "UPDATE devices SET revoked_at = ? WHERE device_id = ?",
            &[now.into(), device_id.into()],
        )?;
        if changed == 0 {
            return Err(ApiError::not_found());
        }
        accounts::revoke_device_tokens(tx, device_id, now)?;
        tracing::info!(%device_id, "device revoked by the admin");
        Ok(())
    })
}

/// `admin vault delete`.
pub fn vault_delete(db: &Db, vault_id: Uuid) -> ApiResult<()> {
    db.tx(TxKind::Write, |tx| {
        if !records::delete_vault(tx, vault_id)? {
            return Err(ApiError::not_found());
        }
        tracing::info!(%vault_id, "vault deleted by the admin");
        Ok(())
    })
}

/// `admin restored`: the end of every restore from a backup (section 3.4).
pub fn restored(db: &Db) -> ApiResult<()> {
    db.tx(TxKind::Write, |tx| {
        records::replace_generation(tx, model::now())
    })?;
    tracing::info!("server generation replaced after a restore");
    Ok(())
}

/// `admin purge`.
pub fn purge(db: &Db, retention_days: u64) -> ApiResult<u64> {
    db.tx(TxKind::Write, |tx| {
        accounts::purge(tx, retention_days, model::now())
    })
}
