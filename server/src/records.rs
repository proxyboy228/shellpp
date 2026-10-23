//! Vaults and records (docs/sync-protocol.md, sections 6.3, 6.5, 6.6, 7.2,
//! 7.7): the vault list, the epoch chain, pull and push with the acceptance
//! rules in their order. The team vault endpoints are stage 8; the rules
//! that depend on the vault kind are written for both kinds already.

use axum::http::StatusCode;
use shellpp_sync_api::api::{
    B64, EpochInfo, ItemStatus, PullResponse, PushResponse, PushResult, RecordWrite, SeqEnvelope,
    VaultKeys, VaultList, VaultSummary, code, limits,
};
use shellpp_sync_api::{classes, envelope};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::accounts::Principal;
use crate::db::{Backend, Tx, Ty, Val};
use crate::error::{ApiError, ApiResult};
use crate::model::{self, User};

/// The member row of a vault: kind, role, epoch, sequence numbers.
#[derive(Clone, Debug)]
pub struct Membership {
    pub kind: String,
    pub role: String,
    pub current_epoch: u32,
    pub last_seq: u64,
    pub min_valid_seq: u64,
    pub rotation_pending: bool,
}

/// The caller's membership of `vault_id`; `403 not_member` for unknown
/// vaults too, so a vault's existence is not revealed (section 3.3).
pub fn membership(tx: &mut dyn Tx, vault_id: Uuid, user_id: Uuid) -> ApiResult<Membership> {
    let row = tx
        .query_one(
            "SELECT v.kind, m.role, v.current_epoch, v.last_seq, v.min_valid_seq, \
             v.rotation_pending FROM vaults v JOIN vault_members m ON m.vault_id = v.vault_id \
             WHERE v.vault_id = ? AND m.user_id = ?",
            &[vault_id.into(), user_id.into()],
            &[Ty::Text, Ty::Text, Ty::Int, Ty::Int, Ty::Int, Ty::Bool],
        )?
        .ok_or_else(ApiError::not_member)?;
    Ok(Membership {
        kind: row[0].text().unwrap_or_default().to_owned(),
        role: row[1].text().unwrap_or_default().to_owned(),
        current_epoch: row[2].int().unwrap_or(1).max(1) as u32,
        last_seq: row[3].int().unwrap_or(0).max(0) as u64,
        min_valid_seq: row[4].int().unwrap_or(0).max(0) as u64,
        rotation_pending: row[5].bool().unwrap_or(false),
    })
}

/// `GET /v1/vaults`.
pub fn vaults(tx: &mut dyn Tx, principal: &Principal) -> ApiResult<VaultList> {
    let rows = tx.query(
        "SELECT v.vault_id, v.kind, m.role, v.current_epoch, v.last_seq, v.rotation_pending \
         FROM vaults v JOIN vault_members m ON m.vault_id = v.vault_id WHERE m.user_id = ? \
         ORDER BY v.kind, v.vault_id",
        &[principal.user_id.into()],
        &[Ty::Uuid, Ty::Text, Ty::Text, Ty::Int, Ty::Int, Ty::Bool],
    )?;
    Ok(VaultList {
        vaults: rows
            .iter()
            .filter_map(|row| {
                Some(VaultSummary {
                    vault_id: row[0].uuid()?,
                    kind: row[1].text()?.to_owned(),
                    role: row[2].text()?.to_owned(),
                    current_epoch: row[3].int()?.max(1) as u32,
                    last_seq: row[4].int()?.max(0) as u64,
                    rotation_pending: row[5].bool()?,
                })
            })
            .collect(),
    })
}

/// `GET /v1/vaults/{id}/keys`: the epoch chain (no HPKE wraps for the
/// personal vault; the client opens the account's `kek_wrap`).
pub fn keys(tx: &mut dyn Tx, principal: &Principal, vault_id: Uuid) -> ApiResult<VaultKeys> {
    let member = membership(tx, vault_id, principal.user_id)?;
    let rows = tx.query(
        "SELECT epoch, prev_key_wrap, created_by, created_at FROM vault_key_epochs \
         WHERE vault_id = ? ORDER BY epoch",
        &[vault_id.into()],
        &[Ty::Int, Ty::Bytes, Ty::Uuid, Ty::Time],
    )?;
    Ok(VaultKeys {
        current_epoch: member.current_epoch,
        epoch_chain: rows
            .iter()
            .filter_map(|row| {
                Some(EpochInfo {
                    epoch: row[0].int()?.max(1) as u32,
                    prev_key_wrap: row[1].bytes().map(|b| B64(b.to_vec())),
                    created_by: row[2].uuid()?,
                    created_at: model::wire_time(row[3].time()?),
                })
            })
            .collect(),
        wraps: Vec::new(),
    })
}

fn cursor_expired(reason: &str, min_valid_seq: u64, last_seq: u64) -> ApiError {
    ApiError::new(
        StatusCode::GONE,
        code::CURSOR_EXPIRED,
        "the cursor is older than the oldest retained change or newer than the newest",
    )
    .detail("reason", reason)
    .detail("min_valid_seq", min_valid_seq)
    .detail("last_seq", last_seq)
}

/// Parses `classes=1,2,3` (decimal codes); class 1 is always included.
pub fn parse_classes(text: Option<&str>) -> ApiResult<Option<Vec<u8>>> {
    let Some(text) = text.filter(|text| !text.is_empty()) else {
        return Ok(None);
    };
    let mut list = vec![classes::VAULT];
    for part in text.split(',') {
        let class: u8 = part
            .trim()
            .parse()
            .map_err(|_| ApiError::bad_request("classes holds a value that is not a class"))?;
        if !classes::is_known(class) {
            return Err(ApiError::bad_request("classes holds an unknown class"));
        }
        if !list.contains(&class) {
            list.push(class);
        }
    }
    Ok(Some(list))
}

/// `GET /v1/vaults/{id}/changes` (section 6.3), in one read snapshot.
pub fn pull(
    tx: &mut dyn Tx,
    principal: &Principal,
    vault_id: Uuid,
    since: u64,
    limit: Option<u64>,
    filter: Option<Vec<u8>>,
) -> ApiResult<PullResponse> {
    let member = membership(tx, vault_id, principal.user_id)?;
    if since > 0 && since < member.min_valid_seq {
        return Err(cursor_expired(
            "purged",
            member.min_valid_seq,
            member.last_seq,
        ));
    }
    if since > member.last_seq {
        return Err(cursor_expired(
            "ahead",
            member.min_valid_seq,
            member.last_seq,
        ));
    }
    let limit = limit
        .unwrap_or(limits::DEFAULT_PULL_LIMIT)
        .clamp(1, limits::MAX_PULL_LIMIT);
    let mut sql =
        String::from("SELECT seq, class, envelope FROM records WHERE vault_id = ? AND seq > ?");
    let mut params: Vec<Val> = vec![vault_id.into(), (since as i64).into()];
    if let Some(classes) = &filter {
        let marks: Vec<&str> = classes.iter().map(|_| "?").collect();
        sql.push_str(&format!(" AND class IN ({})", marks.join(", ")));
        params.extend(classes.iter().map(|class| Val::Int(i64::from(*class))));
    }
    sql.push_str(" ORDER BY seq LIMIT ?");
    // One more than the limit tells whether the selection stopped early.
    params.push(((limit + 1) as i64).into());
    let rows = tx.query(&sql, &params, &[Ty::Int, Ty::Int, Ty::Bytes])?;
    let mut records = Vec::new();
    let mut bytes = 0u64;
    let mut stopped_early = false;
    for row in &rows {
        if records.len() as u64 >= limit {
            stopped_early = true;
            break;
        }
        let envelope = row[2].bytes().unwrap_or_default();
        if !records.is_empty() && bytes + envelope.len() as u64 > limits::MAX_PULL_BYTES {
            stopped_early = true;
            break;
        }
        bytes += envelope.len() as u64;
        records.push(SeqEnvelope {
            seq: row[0].int().unwrap_or(0).max(0) as u64,
            envelope: B64(envelope.to_vec()),
        });
    }
    let next_cursor = if stopped_early {
        records.last().map_or(since, |record| record.seq)
    } else {
        member.last_seq
    };
    Ok(PullResponse {
        records,
        next_cursor,
        has_more: stopped_early,
        current_epoch: member.current_epoch,
        min_valid_seq: member.min_valid_seq,
    })
}

/// Whether `class` may live in a vault of `kind` (section 7.5).
fn class_allowed(kind: &str, class: u8) -> bool {
    kind == "personal"
        || !matches!(
            class,
            classes::HISTORY | classes::KNOWN_HOSTS | classes::APPEARANCE | classes::WORKSPACES
        )
}

/// Whether `role` may write `class` in a vault of `kind` (section 7.5).
/// `stored_creator` is the creator of the stored record, when there is one.
fn role_allows(
    kind: &str,
    role: &str,
    class: u8,
    base_revision: u64,
    stored_creator: Option<Uuid>,
    user: Uuid,
) -> bool {
    if kind == "personal" {
        return role == "owner";
    }
    match class {
        classes::VAULT => role == "owner",
        classes::CONNECTION_LOG => {
            role == "owner"
                || (base_revision == 0 && stored_creator.is_none())
                || stored_creator == Some(user)
        }
        _ => role == "owner" || role == "editor",
    }
}

/// A stored record: its vault, class, revision, creator and envelope.
struct Stored {
    vault_id: Uuid,
    class: u8,
    revision: u64,
    tombstone: bool,
    created_by: Uuid,
    seq: u64,
    envelope: Vec<u8>,
}

fn stored(tx: &mut dyn Tx, record_id: Uuid) -> ApiResult<Option<Stored>> {
    Ok(tx
        .query_one(
            "SELECT vault_id, class, revision, tombstone, created_by, seq, envelope \
             FROM records WHERE record_id = ?",
            &[record_id.into()],
            &[
                Ty::Uuid,
                Ty::Int,
                Ty::Int,
                Ty::Bool,
                Ty::Uuid,
                Ty::Int,
                Ty::Bytes,
            ],
        )?
        .and_then(|row| {
            Some(Stored {
                vault_id: row[0].uuid()?,
                class: u8::try_from(row[1].int()?).ok()?,
                revision: row[2].int()?.max(0) as u64,
                tombstone: row[3].bool()?,
                created_by: row[4].uuid()?,
                seq: row[5].int()?.max(0) as u64,
                envelope: row[6].bytes()?.to_vec(),
            })
        }))
}

/// The header of an envelope after the checks of section 6.6, step 2.
fn checked_header(bytes: &[u8], vault_id: Uuid) -> Option<envelope::EnvelopeHeader> {
    let parts = envelope::split(bytes).ok()?;
    let header = parts.header;
    let record_id = Uuid::from_bytes(header.record_id);
    (header.vault_id == *vault_id.as_bytes()
        && record_id.get_version_num() == 7
        && header.revision >= 1
        && header.key_epoch >= 1)
        .then_some(header)
}

/// What a write would do to an account's usage (records, bytes), for the
/// quota check of section 6.6, step 8. `None`: the write does not count.
fn growth(stored: Option<&Stored>, tombstone: bool, len: u64) -> Option<(u64, u64)> {
    if tombstone {
        return None;
    }
    match stored {
        None => Some((1, len)),
        Some(stored) if stored.tombstone => Some((1, len)),
        Some(stored) => {
            let old = stored.envelope.len() as u64;
            (len > old).then_some((0, len - old))
        }
    }
}

/// The per-account usage while a push runs: limits and current use.
struct QuotaState {
    max_records: Option<u64>,
    max_bytes: Option<u64>,
    records: u64,
    bytes: u64,
}

impl QuotaState {
    fn allows(&self, (records, bytes): (u64, u64)) -> bool {
        self.max_records
            .is_none_or(|max| self.records + records <= max)
            && self.max_bytes.is_none_or(|max| self.bytes + bytes <= max)
    }
}

/// Writes one accepted item: `seq = ++last_seq` and the row (step 9).
#[allow(clippy::too_many_arguments)]
fn accept(
    tx: &mut dyn Tx,
    vault_id: Uuid,
    record_id: Uuid,
    header: &envelope::EnvelopeHeader,
    bytes: &[u8],
    exists: bool,
    principal: &Principal,
    now: OffsetDateTime,
) -> ApiResult<u64> {
    let seq_row = tx
        .query_one(
            "SELECT last_seq FROM vaults WHERE vault_id = ?",
            &[vault_id.into()],
            &[Ty::Int],
        )?
        .ok_or_else(ApiError::internal)?;
    let seq = seq_row[0].int().unwrap_or(0) + 1;
    tx.exec(
        "UPDATE vaults SET last_seq = ? WHERE vault_id = ?",
        &[seq.into(), vault_id.into()],
    )?;
    let tombstoned_at: Val = if header.tombstone {
        now.into()
    } else {
        Val::Null
    };
    if exists {
        tx.exec(
            "UPDATE records SET seq = ?, class = ?, revision = ?, key_epoch = ?, tombstone = ?, \
             envelope = ?, updated_by = ?, updated_by_device = ?, received_at = ?, \
             tombstoned_at = ? WHERE vault_id = ? AND record_id = ?",
            &[
                seq.into(),
                i64::from(header.class).into(),
                (header.revision as i64).into(),
                i64::from(header.key_epoch).into(),
                header.tombstone.into(),
                bytes.into(),
                principal.user_id.into(),
                principal.device_id.into(),
                now.into(),
                tombstoned_at,
                vault_id.into(),
                record_id.into(),
            ],
        )?;
    } else {
        tx.exec(
            "INSERT INTO records (vault_id, record_id, seq, class, revision, key_epoch, \
             tombstone, envelope, created_by, updated_by, updated_by_device, received_at, \
             tombstoned_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            &[
                vault_id.into(),
                record_id.into(),
                seq.into(),
                i64::from(header.class).into(),
                (header.revision as i64).into(),
                i64::from(header.key_epoch).into(),
                header.tombstone.into(),
                bytes.into(),
                principal.user_id.into(),
                principal.user_id.into(),
                principal.device_id.into(),
                now.into(),
                tombstoned_at,
            ],
        )?;
    }
    Ok(seq as u64)
}

fn result(record_id: Uuid, status: ItemStatus) -> PushResult {
    PushResult {
        record_id,
        status,
        seq: None,
        current: None,
    }
}

/// The record id of an envelope that may not even have a header.
fn record_id_of(bytes: &[u8]) -> Uuid {
    if bytes.len() >= 36 {
        Uuid::from_slice(&bytes[20..36]).unwrap_or_default()
    } else {
        Uuid::nil()
    }
}

/// `POST /v1/vaults/{id}/changes` (sections 6.5, 6.6): one transaction
/// with the vault locked; each item judged on its own, in order.
pub fn push(
    tx: &mut dyn Tx,
    principal: &Principal,
    vault_id: Uuid,
    items: &[RecordWrite],
    quota_defaults: (Option<u64>, Option<u64>),
    now: OffsetDateTime,
) -> ApiResult<PushResponse> {
    if items.len() as u64 > limits::MAX_PUSH_ITEMS {
        return Err(ApiError::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            code::PAYLOAD_TOO_LARGE,
            "too many items",
        ));
    }
    tx.lock_vault(vault_id)?;
    let member = membership(tx, vault_id, principal.user_id)?;
    let mut quota: Option<QuotaState> = None;
    let mut results = Vec::with_capacity(items.len());
    for item in items {
        let bytes = item.envelope.as_bytes();
        let record_id = record_id_of(bytes);
        // 1. Size.
        if bytes.len() as u64 > limits::MAX_ENVELOPE_BYTES {
            results.push(result(record_id, ItemStatus::PayloadTooLarge));
            continue;
        }
        // 2. Header.
        let Some(header) = checked_header(bytes, vault_id) else {
            results.push(result(record_id, ItemStatus::InvalidEnvelope));
            continue;
        };
        // 3. Class in this vault kind.
        if !class_allowed(&member.kind, header.class) {
            results.push(result(record_id, ItemStatus::ClassNotAllowed));
            continue;
        }
        let existing = stored(tx, record_id)?;
        // 4. Role.
        let creator = existing
            .as_ref()
            .filter(|stored| stored.vault_id == vault_id)
            .map(|stored| stored.created_by);
        if !role_allows(
            &member.kind,
            &member.role,
            header.class,
            item.base_revision,
            creator,
            principal.user_id,
        ) {
            results.push(result(record_id, ItemStatus::InsufficientRole));
            continue;
        }
        // 5. The current epoch.
        if header.key_epoch != member.current_epoch {
            results.push(result(record_id, ItemStatus::StaleKeyEpoch));
            continue;
        }
        // 6. A record's class never changes, and its id belongs to one vault.
        if let Some(stored) = &existing
            && (stored.vault_id != vault_id || stored.class != header.class)
        {
            results.push(result(record_id, ItemStatus::InvalidEnvelope));
            continue;
        }
        // 7. The revision.
        let stored_revision = existing.as_ref().map_or(0, |stored| stored.revision);
        if stored_revision != item.base_revision || header.revision <= item.base_revision {
            results.push(PushResult {
                record_id,
                status: ItemStatus::Conflict,
                seq: None,
                current: Some(existing.as_ref().map(|stored| SeqEnvelope {
                    seq: stored.seq,
                    envelope: B64(stored.envelope.clone()),
                })),
            });
            continue;
        }
        // 8. Quota: only growth of a record the pushing account created.
        let creator_is_caller = existing
            .as_ref()
            .is_none_or(|stored| stored.created_by == principal.user_id);
        let grows = growth(existing.as_ref(), header.tombstone, bytes.len() as u64)
            .filter(|_| creator_is_caller);
        if let Some(delta) = grows {
            if quota.is_none() {
                let user = model::user_by_id(tx, principal.user_id)?
                    .ok_or_else(ApiError::unauthenticated)?;
                let (max_records, max_bytes) = model::limits_of(&user, quota_defaults);
                let (records, used) = if max_records.is_some() || max_bytes.is_some() {
                    model::usage(tx, principal.user_id)?
                } else {
                    (0, 0)
                };
                quota = Some(QuotaState {
                    max_records,
                    max_bytes,
                    records,
                    bytes: used,
                });
            }
            let state = quota.as_mut().expect("set above");
            if !state.allows(delta) {
                results.push(result(record_id, ItemStatus::QuotaExceeded));
                continue;
            }
            state.records += delta.0;
            state.bytes += delta.1;
        } else if let (Some(state), Some(stored)) = (quota.as_mut(), existing.as_ref())
            && header.tombstone
            && !stored.tombstone
            && stored.created_by == principal.user_id
        {
            // A deletion frees its record for the next items.
            state.records = state.records.saturating_sub(1);
            state.bytes = state.bytes.saturating_sub(stored.envelope.len() as u64);
        }
        // 9. Accept.
        let seq = accept(
            tx,
            vault_id,
            record_id,
            &header,
            bytes,
            existing.is_some(),
            principal,
            now,
        )?;
        results.push(PushResult {
            record_id,
            status: ItemStatus::Ok,
            seq: Some(seq),
            current: None,
        });
    }
    Ok(PushResponse { results })
}

/// The `recovery_key` record of a reissue (section 5.3): the push rules,
/// its header naming the personal vault, class `0x01` and the new epoch,
/// and `base_revision` equal to the stored revision (`409
/// revision_conflict` with `details.current` otherwise). Returns its seq.
pub fn write_reissue_record(
    tx: &mut dyn Tx,
    user: &User,
    principal: &Principal,
    record: &RecordWrite,
    new_epoch: u32,
    now: OffsetDateTime,
) -> ApiResult<u64> {
    let vault_id = user.personal_vault_id;
    let bytes = record.envelope.as_bytes();
    let invalid = || {
        ApiError::new(
            StatusCode::BAD_REQUEST,
            code::INVALID_ENVELOPE,
            "the record is not a recovery key of the new epoch",
        )
    };
    if bytes.len() as u64 > limits::MAX_ENVELOPE_BYTES {
        return Err(invalid());
    }
    let header = checked_header(bytes, vault_id).ok_or_else(invalid)?;
    if header.class != classes::VAULT || header.key_epoch != new_epoch || header.tombstone {
        return Err(invalid());
    }
    let record_id = Uuid::from_bytes(header.record_id);
    let existing = stored(tx, record_id)?;
    if let Some(stored) = &existing
        && (stored.vault_id != vault_id || stored.class != header.class)
    {
        return Err(invalid());
    }
    let stored_revision = existing.as_ref().map_or(0, |stored| stored.revision);
    if stored_revision != record.base_revision || header.revision <= record.base_revision {
        let mut error = ApiError::new(
            StatusCode::CONFLICT,
            code::REVISION_CONFLICT,
            "the recovery key record changed",
        );
        if let Some(stored) = &existing {
            error = error.detail(
                "current",
                serde_json::to_value(SeqEnvelope {
                    seq: stored.seq,
                    envelope: B64(stored.envelope.clone()),
                })
                .unwrap_or_default(),
            );
        }
        return Err(error);
    }
    accept(
        tx,
        vault_id,
        record_id,
        &header,
        bytes,
        existing.is_some(),
        principal,
        now,
    )
}

/// The server generation (section 3.4).
pub fn server_generation(tx: &mut dyn Tx) -> ApiResult<Option<Vec<u8>>> {
    Ok(tx
        .query_one(
            "SELECT server_generation FROM server_state WHERE id = 1",
            &[],
            &[Ty::Bytes],
        )?
        .and_then(|row| row[0].bytes().map(<[u8]>::to_vec)))
}

/// `admin restored`: a new random generation (section 3.4).
pub fn replace_generation(tx: &mut dyn Tx, now: OffsetDateTime) -> ApiResult<()> {
    let generation = crate::secrets::random_bytes::<16>().to_vec();
    let changed = tx.exec(
        "UPDATE server_state SET server_generation = ?, changed_at = ? WHERE id = 1",
        &[generation.clone().into(), now.into()],
    )?;
    if changed == 0 {
        tx.exec(
            "INSERT INTO server_state (id, server_generation, changed_at) VALUES (1, ?, ?)",
            &[generation.into(), now.into()],
        )?;
    }
    Ok(())
}

/// Deletes a vault and its rows (`admin vault delete`).
pub fn delete_vault(tx: &mut dyn Tx, vault_id: Uuid) -> ApiResult<bool> {
    let _ = Backend::Sqlite;
    Ok(tx.exec("DELETE FROM vaults WHERE vault_id = ?", &[vault_id.into()])? > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn class_lists_are_parsed() {
        assert_eq!(parse_classes(None).unwrap(), None);
        assert_eq!(parse_classes(Some("2,3")).unwrap(), Some(vec![1, 2, 3]));
        assert_eq!(parse_classes(Some("1,1,2")).unwrap(), Some(vec![1, 2]));
        assert!(parse_classes(Some("11")).is_err());
        assert!(parse_classes(Some("x")).is_err());
    }

    #[test]
    fn roles_by_class() {
        let me = Uuid::now_v7();
        let other = Uuid::now_v7();
        assert!(role_allows(
            "personal",
            "owner",
            classes::HISTORY,
            0,
            None,
            me
        ));
        assert!(!role_allows("team", "viewer", classes::HOSTS, 0, None, me));
        assert!(role_allows("team", "editor", classes::HOSTS, 0, None, me));
        assert!(!role_allows("team", "editor", classes::VAULT, 0, None, me));
        // A viewer may create log copies and change only its own.
        assert!(role_allows(
            "team",
            "viewer",
            classes::CONNECTION_LOG,
            0,
            None,
            me
        ));
        assert!(role_allows(
            "team",
            "viewer",
            classes::CONNECTION_LOG,
            3,
            Some(me),
            me
        ));
        assert!(!role_allows(
            "team",
            "viewer",
            classes::CONNECTION_LOG,
            3,
            Some(other),
            me
        ));
        assert!(!class_allowed("team", classes::KNOWN_HOSTS));
        assert!(class_allowed("personal", classes::KNOWN_HOSTS));
    }

    #[test]
    fn quota_growth_rules() {
        assert_eq!(growth(None, false, 344), Some((1, 344)));
        assert_eq!(growth(None, true, 344), None);
        let live = Stored {
            vault_id: Uuid::nil(),
            class: 2,
            revision: 1,
            tombstone: false,
            created_by: Uuid::nil(),
            seq: 1,
            envelope: vec![0; 600],
        };
        assert_eq!(
            growth(Some(&live), false, 600),
            None,
            "re-encryption does not grow"
        );
        assert_eq!(growth(Some(&live), false, 856), Some((0, 256)));
        let dead = Stored {
            tombstone: true,
            ..live
        };
        assert_eq!(growth(Some(&dead), false, 344), Some((1, 344)));
    }
}
