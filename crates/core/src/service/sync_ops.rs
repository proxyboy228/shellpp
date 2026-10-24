//! Account operations and the sync cycle of stage 7
//! (docs/sync-protocol.md, sections 2-6; data-model 17-18.5; storage-format
//! 6.6; UI 12.2, 13.5, 13.6, 13.7, 14).
//!
//! Network calls never run under the core mutex: every operation reads what
//! it needs under the lock, talks to the server without it, and writes the
//! result under the lock again after checking that nothing it read changed.
//! `sync_guard` keeps two operations from talking to the server at once.
//!
//! Sync starts only after the user connects a server (plan, rule 1.7.4):
//! without a `server_account` row every cycle is a no-op.
//!
//! Not in run 7 (refused with a code, see the report): registration or
//! sign-in on a device whose personal vault was bound to another account
//! (the archive of data-model 18.5, `previous_account`), personal epochs the
//! server holds above the local one (`newer_epoch`), and recovery or a
//! recovery key reissue while bound (`signed_in`).

use std::collections::{HashMap, HashSet};
use std::time::{Duration, SystemTime};

use data_encoding::BASE64URL_NOPAD;
use secrecy::{ExposeSecret, SecretString};
use serde::Serialize;
use serde_json::{Map, Value};
use shellpp_sync_api::api::{self as wire, B64, Secret, code};
use shellpp_sync_api::{classes, envelope};
use uuid::Uuid;
use zeroize::{Zeroize, Zeroizing};

use super::lock_ops::{self, KeyContext};
use super::session::Session;
use super::{Core, Inner, recordings};
use crate::crypto::{self, KdfParams, Key32, WrapPurpose};
use crate::error::CoreError;
use crate::index::IndexedRecord;
use crate::lock::PasswordMaterial;
use crate::model::hlc::{Hlc, HlcClock};
use crate::model::payload::Payload;
use crate::model::time::{format_timestamp, format_unix_ms, unix_ms, uuid_v7_timestamp};
use crate::model::types::RecordKind;
use crate::service::dto::{CoreEventKind, LockState};
use crate::store::{
    DeletedOrigin, DuplicatePairRow, KeyWrapRow, LocalAuthRow, META_ACCOUNT_SERVER_URL,
    META_ACCOUNT_USER_ID, META_CREATED_AT, META_DB_ID, META_DEVICE_ID, RecentlyDeletedRow,
    RecordRow, ServerAccountRow, Store, SyncStateRow, TlsTrustKind, TlsTrustRow, VaultRow,
    WrapKind,
};
use crate::sync::engine::{self, Selection};
use crate::sync::{HttpApi, HttpError, TlsProblem, TrustMode, natural, normalize_origin, trash};
use crate::vault::{self, KeyRing, Rotation};

/// Pull rounds of one cycle before it yields (a large first pull continues
/// in the next cycle).
const MAX_PULL_PAGES: usize = 10_000;
/// Push rounds of one cycle (section 6.11).
const MAX_PUSH_ROUNDS: usize = 3;
/// Push batches of one cycle (500 items each): a first sync of a large
/// vault goes up in one cycle.
const MAX_PUSH_BATCHES: usize = 1_000;
/// The longest automatic retry pause.
const MAX_BACKOFF: Duration = Duration::from_secs(15 * 60);

/// Types never carried into another personal vault by the merge at sign-in
/// (data-model 18.5).
const NOT_MERGED: [&str; 8] = [
    "vault_meta",
    "recovery_key",
    "device_info",
    "user_keypair",
    "ssh_id",
    "member_pin",
    "roster_entry",
    "credential_binding",
];

fn refused(reason: &'static str) -> CoreError {
    CoreError::Sync(HttpError::Refused(reason))
}

/// The code stored in `sync_state.last_error`.
fn error_code(error: &CoreError) -> String {
    match error {
        CoreError::Sync(error) => error.code(),
        other => other.code().to_owned(),
    }
}

impl From<HttpError> for CoreError {
    fn from(error: HttpError) -> Self {
        CoreError::Sync(error)
    }
}

/// Memory-only sync state; wiped on lock.
#[derive(Default)]
pub(super) struct SyncRuntime {
    access: Option<Zeroizing<String>>,
    /// The origin that issued `access` (D188): the token is used only with
    /// a client of that origin.
    access_origin: Option<String>,
    pending: Option<PendingSignIn>,
    /// Items the server refused, with the status; not retried
    /// automatically (section 6.5): "Sync now" retries them all, a pushed
    /// deletion those refused for the quota.
    failed: HashMap<Uuid, String>,
    failures: u32,
    backoff_until: Option<SystemTime>,
    integrity_warnings: usize,
    /// The server's `recovery_wrap` does not hold the personal vault key.
    recovery_mismatch: bool,
    /// The `recovery_key` record (id and revision) and the personal epoch
    /// the server's `recovery_wrap` was last found right for; checked in
    /// every cycle until then, and again when either changes (D173, 1.17).
    recovery_checked: Option<(Uuid, u64, u32)>,
    newer_epoch: bool,
    /// The user confirmed "Reset sync state of this vault": the next cycle
    /// refuses headers with an epoch the server does not hold.
    reset_refuse_newer: bool,
    /// Tests: the next merge at sign-in stops after its pull.
    #[cfg(any(test, feature = "test-hooks"))]
    fail_merge: bool,
    /// The last sign-in refused weaker KDF parameters (D189).
    pub(super) weaker_kdf: Option<super::sync_trust::WeakerKdf>,
}

impl SyncRuntime {
    /// Keeps the access token `origin` issued.
    fn hold_access(&mut self, token: Zeroizing<String>, origin: &str) {
        self.access = Some(token);
        self.access_origin = Some(origin.to_owned());
    }

    /// Drops the access token and a half-finished sign-in (lock).
    pub(super) fn wipe(&mut self) {
        self.access = None;
        self.access_origin = None;
        self.pending = None;
        self.recovery_checked = None;
    }
}

/// A sign-in waiting for its TOTP code.
struct PendingSignIn {
    /// The server's ticket lives 5 minutes; `kek` no longer than that.
    started: SystemTime,
    origin: String,
    email: String,
    ticket: Zeroizing<String>,
    kek: Key32,
    kdf: KdfParams,
    kdf_salt: [u8; 16],
}

/// The connect wizard's first step (UI 13.5).
#[derive(Clone, Debug)]
pub enum ServerCheck {
    Ready {
        origin: String,
        invite_only: bool,
        version: String,
        server_generation: Option<Vec<u8>>,
    },
    /// The certificate was refused: the trust dialog (section 2.1).
    Certificate { origin: String, problem: TlsProblem },
}

/// What the user chose in the trust dialog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TrustChoice {
    System,
    Pin([u8; 32]),
    Ca(Vec<u8>),
}

/// Registration or sign-in input.
pub struct SignInInput {
    pub origin: String,
    pub email: String,
    pub password: SecretString,
    /// The invite code of an invite-only server (registration only).
    pub registration_code: Option<SecretString>,
}

impl std::fmt::Debug for SignInInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SignInInput")
            .field("origin", &self.origin)
            .finish_non_exhaustive()
    }
}

/// The result of a sign-in step.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SignInOutcome {
    Done(SyncStatus),
    TotpRequired,
}

/// Settings → Account & Sync (UI 12.2) and the status line.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct SyncStatus {
    pub bound: bool,
    /// A refresh token is held; `false` after a refused refresh or an
    /// offline recovery: the user signs in again.
    pub signed_in: bool,
    pub server_url: Option<String>,
    pub email: Option<String>,
    pub user_id: Option<Uuid>,
    pub last_sync_at: Option<String>,
    pub last_error: Option<String>,
    /// Local changes not on the server yet (enabled classes).
    pub pending: usize,
    /// Items the server refused.
    pub failed: usize,
    pub integrity_warnings: usize,
    pub recovery_mismatch: bool,
    pub newer_epoch: bool,
    pub conflicts: usize,
    pub duplicates: usize,
    pub brought_back: usize,
    pub recently_deleted: usize,
}

/// What one cycle did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SyncReport {
    pub pulled: usize,
    pub pushed: usize,
    pub conflicts: usize,
    pub integrity_warnings: usize,
    pub failed: usize,
    pub newer_epoch: bool,
    pub brought_back: usize,
    pub restored: bool,
}

/// A Recently deleted entry (UI 14). `Debug` shows no title or data: they
/// carry addresses and user names (security 9.1).
#[derive(Clone, PartialEq, Eq, Serialize)]
pub struct DeletedItem {
    pub record_id: Uuid,
    pub vault_id: Uuid,
    pub type_name: String,
    pub title: String,
    pub deleted_at: String,
    pub from_sync: bool,
    /// The last live version's data for the read-only details panel,
    /// secrets masked.
    pub data: Option<Map<String, Value>>,
}

impl std::fmt::Debug for ConflictItem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConflictItem")
            .field("conflict_id", &self.conflict_id)
            .field("original_record_id", &self.original_record_id)
            .field("original_type", &self.original_type)
            .field("losing_deleted", &self.losing_deleted)
            .finish_non_exhaustive()
    }
}

impl std::fmt::Debug for DeletedItem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeletedItem")
            .field("record_id", &self.record_id)
            .field("type_name", &self.type_name)
            .field("deleted_at", &self.deleted_at)
            .field("from_sync", &self.from_sync)
            .finish_non_exhaustive()
    }
}

/// A conflict copy (data-model 15; UI 13.6). `Debug` shows no title or
/// data of either version: they carry addresses and user names (security
/// 9.1).
#[derive(Clone, PartialEq, Serialize)]
pub struct ConflictItem {
    pub conflict_id: Uuid,
    pub original_record_id: Uuid,
    pub original_type: String,
    pub title: String,
    pub losing_deleted: bool,
    /// The losing version's data with secret members masked.
    pub losing_data: Option<Map<String, Value>>,
    pub losing_device_id: Option<String>,
    /// The fields that differ between the current version and the copy
    /// (UI 13.6), secrets masked.
    pub differences: Vec<FieldDifference>,
    /// The current version is a deletion.
    pub current_deleted: bool,
    /// The original is not here (data-model 15: shown as "missing"): no
    /// row (purged) or one this device cannot open (its epoch key is
    /// missing, damaged). "Keep other" recreates a purged one; one still
    /// here but unreadable is never replaced ([`KeepOther::Unreadable`]).
    pub current_missing: bool,
    /// The current version's data with secret members masked (`None` when
    /// it is deleted or missing).
    pub current_data: Option<Map<String, Value>>,
    /// What "Keep other" does (the type rules of sync-protocol 6.7 hold).
    pub keep_other: KeepOther,
    pub current_device_id: Option<String>,
    /// The devices' names (their `device_info` records) when known.
    pub current_device_name: Option<String>,
    pub losing_device_name: Option<String>,
    /// The version was made on this device.
    pub current_this_device: bool,
    pub losing_this_device: bool,
    /// When each version was changed (RFC 3339, from its HLC).
    pub current_changed_at: Option<String>,
    pub losing_changed_at: Option<String>,
}

/// One field of a conflict's comparison. `Debug` shows the path only: the
/// values carry addresses, user names and commands (security 9.1).
#[derive(Clone, PartialEq, Eq, Serialize)]
pub struct FieldDifference {
    /// Dotted path, for example `ssh.port`.
    pub path: String,
    pub current: String,
    pub other: String,
}

impl std::fmt::Debug for FieldDifference {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FieldDifference")
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

/// What "Keep other" does with a conflict (UI 13.6). The type rules of
/// sync-protocol 6.7 hold for it as for the merge: a revoked known host key
/// stays revoked. Refused choices keep the conflict.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum KeepOther {
    /// The other version replaces the current one.
    Replaces,
    /// The current version is a revoked known host key and the other is
    /// not (or holds another key): the revoked key stays (`marker`,
    /// `key_type`, `public_key`), the other version's remaining fields
    /// replace the current ones.
    KeepsRevocation,
    /// Refused: the other version deletes a revoked known host key.
    RevokedStays,
    /// Refused: this app cannot write the object's type (one from a newer
    /// client, or one it never writes, as `member_pin`).
    Unsupported,
    /// Refused: the current version is here but this device cannot read
    /// it (damaged, its epoch key missing, a newer payload version), so
    /// its type rules cannot be checked: it is never replaced or dropped
    /// for a copy.
    Unreadable,
}

impl KeepOther {
    /// "Keep other" can be chosen.
    pub fn allowed(self) -> bool {
        matches!(self, Self::Replaces | Self::KeepsRevocation)
    }
}

/// Which version a conflict keeps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConflictChoice {
    /// The current version; the copy goes away.
    Current,
    /// The copy's version becomes the record's next revision.
    Copy,
}

/// A duplicate pair from sync (data-model 18.3; UI 13.7).
#[derive(Clone, PartialEq, Eq, Serialize)]
pub struct DuplicateItem {
    pub record_id: Uuid,
    pub other_record_id: Uuid,
    pub type_name: String,
    pub title: String,
    /// The two records side by side (UI 13.7): on the left the older one
    /// (the smaller `record_id`, data-model 18.3), on the right the newer.
    pub left: DuplicateSide,
    pub right: DuplicateSide,
    /// The fields of either record, secrets masked.
    pub fields: Vec<PairField>,
}

/// One record of a duplicate pair.
#[derive(Clone, PartialEq, Eq, Serialize)]
pub struct DuplicateSide {
    pub record_id: Uuid,
    pub title: String,
    pub created_at: String,
    pub modified_at: String,
    /// The device of the last change (its payload's `device_id`), as in
    /// the conflict comparison (UI 13.6).
    pub device_id: String,
    /// That device's name (its `device_info` record) when known.
    pub device_name: Option<String>,
    /// The last change was made on this device.
    pub this_device: bool,
}

/// One field of a duplicate pair's comparison: its value on each side
/// ("—" where a side lacks it), secrets masked.
#[derive(Clone, PartialEq, Eq, Serialize)]
pub struct PairField {
    /// Dotted path, for example `ssh.port`.
    pub path: String,
    pub left: String,
    pub right: String,
    /// The values differ (compared before masking: a different secret is
    /// a difference).
    pub differs: bool,
}

// Debug without host data: titles, addresses, usernames and field values
// stay out of logs (docs/security.md, section 9.1).
impl std::fmt::Debug for DuplicateItem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DuplicateItem")
            .field("record_id", &self.record_id)
            .field("other_record_id", &self.other_record_id)
            .field("type_name", &self.type_name)
            .field("left", &self.left)
            .field("right", &self.right)
            .field("fields", &self.fields)
            .finish_non_exhaustive()
    }
}

impl std::fmt::Debug for DuplicateSide {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DuplicateSide")
            .field("record_id", &self.record_id)
            .field("this_device", &self.this_device)
            .finish_non_exhaustive()
    }
}

impl std::fmt::Debug for PairField {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PairField")
            .field("path", &self.path)
            .field("differs", &self.differs)
            .finish_non_exhaustive()
    }
}

/// A record a resync brought back (section 6.9, step 5). `Debug` shows no
/// title: it carries addresses and user names (security 9.1).
#[derive(Clone, PartialEq, Eq, Serialize)]
pub struct BroughtBackItem {
    pub record_id: Uuid,
    pub type_name: String,
    pub title: String,
    pub returned_at: String,
}

impl std::fmt::Debug for BroughtBackItem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BroughtBackItem")
            .field("record_id", &self.record_id)
            .field("type_name", &self.type_name)
            .field("returned_at", &self.returned_at)
            .finish_non_exhaustive()
    }
}

/// A device of the account (UI 12.2).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct DeviceItem {
    pub device_id: Uuid,
    pub created_at: String,
    pub last_seen_at: String,
    pub revoked: bool,
    pub current: bool,
}

/// TOTP setup (UI 13.5): the secret for manual entry and the QR text.
pub struct TotpSetup {
    pub secret: SecretString,
    /// The `otpauth://totp/` URI for the QR code, made by the client from
    /// the secret and the account email (D188); `None` shows only the
    /// secret.
    pub otpauth_uri: Option<SecretString>,
}

impl std::fmt::Debug for TotpSetup {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TotpSetup { .. }")
    }
}

/// Keys derived from the master password for one origin.
struct Derived {
    kek: Key32,
    auth_key: Key32,
}

/// The account password's keys with the email bound into the salt
/// (section 4.1). Refuses parameters outside the floor or the ceiling.
fn derive(
    kdf: &KdfParams,
    kdf_salt: &[u8; 16],
    email: &str,
    password: &SecretString,
    origin: &str,
    class: crypto::PlatformClass,
) -> Result<Derived, CoreError> {
    kdf.check(class).map_err(|_| refused("unsafe_kdf"))?;
    let mut salt = crypto::argon2_salt(Some(email), kdf_salt);
    let master_key = crypto::derive_master_key(password, kdf, &salt, class);
    salt.zeroize();
    let master_key = master_key?;
    Ok(Derived {
        kek: crypto::derive_kek(&master_key),
        auth_key: crypto::derive_auth_key(&master_key, origin),
    })
}

fn wire_kdf(kdf: &KdfParams) -> wire::Kdf {
    wire::Kdf {
        alg: kdf.alg.clone(),
        v: kdf.v,
        m_kib: kdf.m_kib,
        t: kdf.t,
        p: kdf.p,
    }
}

fn local_kdf(kdf: &wire::Kdf) -> KdfParams {
    KdfParams {
        alg: kdf.alg.clone(),
        v: kdf.v,
        m_kib: kdf.m_kib,
        t: kdf.t,
        p: kdf.p,
    }
}

/// The `rotation` object of a request (section 5.1, step 5).
fn wire_rotation(rotation: &Rotation) -> Result<wire::Rotation, CoreError> {
    let row = |kind| {
        rotation
            .row(kind)
            .map(|row: &KeyWrapRow| B64(row.wrap.clone()))
            .ok_or(CoreError::Internal("rotation"))
    };
    Ok(wire::Rotation {
        epoch: rotation.new_epoch,
        prev_key_wrap: row(WrapKind::Chain)?,
        recovery_wrap: row(WrapKind::Recovery)?,
    })
}

fn key_b64(key: &Key32) -> B64 {
    B64(key.expose().to_vec())
}

fn secret(text: &str) -> Secret {
    Secret(text.to_owned())
}

/// The trust mode stored for an origin.
pub(super) fn trust_of(store: &Store, origin: &str) -> Result<TrustMode, CoreError> {
    Ok(match store.server_tls(origin)? {
        Some(row) if row.kind == TlsTrustKind::Pinned => TrustMode::Pinned(row.cert_sha256),
        Some(row) => match row.ca_der {
            Some(der) => TrustMode::Ca(der),
            None => TrustMode::System,
        },
        None => TrustMode::System,
    })
}

/// The selection of the personal vault from `sync.toggles` (data-model 17).
pub(super) fn selection(store: &Store) -> Selection {
    let toggles: Map<String, Value> = store
        .setting("sync.toggles")
        .ok()
        .flatten()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default();
    let on =
        |key: &str, default: bool| toggles.get(key).and_then(Value::as_bool).unwrap_or(default);
    let mut list = vec![classes::VAULT];
    for (key, class, default) in [
        ("hosts", classes::HOSTS, true),
        ("keys", classes::KEYS, true),
        ("snippets", classes::SNIPPETS, true),
        ("forwarding", classes::FORWARDING, true),
        ("history", classes::HISTORY, false),
        ("connection_log", classes::CONNECTION_LOG, false),
        ("known_hosts", classes::KNOWN_HOSTS, true),
        ("workspaces", classes::WORKSPACES, true),
    ] {
        if on(key, default) {
            list.push(class);
        }
    }
    let themes = on("terminal_themes", true);
    let rules = on("highlight_rules", true);
    let mut skip_types = Vec::new();
    if themes || rules {
        list.push(classes::APPEARANCE);
    }
    if !themes {
        skip_types.push("terminal_theme".to_owned());
    }
    if !rules {
        skip_types.push("highlight_rule".to_owned());
    }
    list.sort_unstable();
    Selection {
        classes: list,
        skip_types,
    }
}

/// The payload of any row of the ring's vault, live or not, of any type.
fn open_payload(ring: &mut KeyRing, row: &RecordRow) -> Option<Payload> {
    let record_key = ring.record_key(row.key_epoch)?;
    let (_, bytes) = crypto::open_envelope(record_key, &row.envelope).ok()?;
    Payload::from_json_bytes(&bytes).ok()
}

/// The payload of a stored envelope (Recently deleted).
fn open_envelope_payload(ring: &mut KeyRing, bytes: &[u8]) -> Option<Payload> {
    let header = envelope::split(bytes).ok()?.header;
    let record_key = ring.record_key(header.key_epoch)?;
    let (_, plain) = crypto::open_envelope(record_key, bytes).ok()?;
    Payload::from_json_bytes(&plain).ok()
}

/// A short title for lists: the first non-empty of the usual name fields.
fn title_of(type_name: &str, data: Option<&Map<String, Value>>) -> String {
    let Some(data) = data else {
        return type_name.to_owned();
    };
    for field in [
        "label", "name", "address", "host", "command", "pattern", "title",
    ] {
        if let Some(text) = data.get(field).and_then(Value::as_str)
            && !text.is_empty()
        {
            return text.to_owned();
        }
    }
    type_name.to_owned()
}

/// Replaces every string (and object key) equal to a mapped record id.
fn remap(value: &mut Value, map: &HashMap<String, String>) {
    match value {
        Value::String(text) => {
            if let Some(new) = map.get(text.as_str()) {
                *text = new.clone();
            }
        }
        Value::Array(items) => items.iter_mut().for_each(|item| remap(item, map)),
        Value::Object(object) => {
            let entries: Vec<(String, Value)> = std::mem::take(object).into_iter().collect();
            for (key, mut item) in entries {
                remap(&mut item, map);
                let key = map.get(key.as_str()).cloned().unwrap_or(key);
                object.insert(key, item);
            }
        }
        _ => {}
    }
}

fn remap_data(data: &Map<String, Value>, map: &HashMap<String, String>) -> Map<String, Value> {
    let mut value = Value::Object(data.clone());
    remap(&mut value, map);
    match value {
        Value::Object(object) => object,
        _ => Map::new(),
    }
}

/// `kek_wrap` of the account bundle opened with `kek` (section 4.4).
fn open_account_wrap(
    vault_id: Uuid,
    epoch: u32,
    wrap: &[u8],
    wrapping_key: &Key32,
    kind: WrapKind,
) -> Option<Key32> {
    let purpose = match kind {
        WrapKind::Kek => WrapPurpose::Kek,
        _ => WrapPurpose::Recovery,
    };
    let aad = crypto::vault_wrap_aad(purpose, &vault_id, epoch);
    crypto::unwrap_key(wrapping_key, &aad, wrap).ok()
}

/// The key ring of an account vault: the current key and the older ones
/// through the server's epoch chain (its `prev_key_wrap` are the `chain`
/// rows), which are returned for storage.
fn ring_from_chain(
    vault_id: Uuid,
    epoch: u32,
    key: Key32,
    chain: &[wire::EpochInfo],
) -> Result<(KeyRing, Vec<KeyWrapRow>), CoreError> {
    let mut ring = KeyRing::new(vault_id, epoch, key);
    let mut rows = Vec::new();
    let mut current = epoch;
    while current > 1 {
        let wrap = chain
            .iter()
            .find(|info| info.epoch == current)
            .and_then(|info| info.prev_key_wrap.as_ref())
            .ok_or_else(|| refused("integrity"))?;
        let row = KeyWrapRow {
            vault_id,
            epoch: current,
            kind: WrapKind::Chain,
            sender_user_id: None,
            hpke_enc: None,
            wrap: wrap.0.clone(),
        };
        let held = ring.key(current).ok_or_else(|| refused("integrity"))?;
        let chain_key = crypto::derive_chain_key(held, &vault_id, current);
        let aad = crypto::vault_wrap_aad(WrapPurpose::EpochChain, &vault_id, current - 1);
        let older =
            crypto::unwrap_key(&chain_key, &aad, &row.wrap).map_err(|_| refused("integrity"))?;
        ring.push_epoch(current - 1, older);
        rows.push(row);
        current -= 1;
    }
    // `push_epoch` moved the current epoch down; put the newest back.
    let newest = ring
        .key(epoch)
        .map(Key32::duplicate)
        .ok_or_else(|| refused("integrity"))?;
    ring.push_epoch(epoch, newest);
    Ok((ring, rows))
}

/// The refresh token wrapped for `server_account` (storage-format 6.4).
fn wrap_refresh(
    ring: &KeyRing,
    device_id: &Uuid,
    origin: &str,
    token: &str,
) -> Result<(Vec<u8>, u32), CoreError> {
    let epoch = ring.current_epoch();
    let key = crypto::derive_local_secrets_key(ring.current_key(), device_id, epoch);
    let blob = crypto::wrap_seal(
        &key,
        &crypto::refresh_token_aad(device_id, origin),
        token.as_bytes(),
    )?;
    Ok((blob, epoch))
}

fn open_refresh(
    ring: &KeyRing,
    device_id: &Uuid,
    account: &ServerAccountRow,
) -> Option<Zeroizing<String>> {
    let blob = account.refresh_token.as_ref()?;
    let vault_key = ring.key(account.secrets_epoch)?;
    let key = crypto::derive_local_secrets_key(vault_key, device_id, account.secrets_epoch);
    // Bound to the origin it was stored with (D188): another server_url
    // never opens it.
    let aad = crypto::refresh_token_aad(device_id, &account.server_url);
    let plain = crypto::wrap_open(&key, &aad, blob).ok()?;
    std::str::from_utf8(&plain)
        .ok()
        .map(|text| Zeroizing::new(text.to_owned()))
}

/// What an authenticated call needs, read under the lock.
struct Link {
    api: HttpApi,
    vault_id: Uuid,
    device_id: Uuid,
}

impl Core {
    fn sync_lock(&self) -> std::sync::MutexGuard<'_, ()> {
        self.sync_guard
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    pub(super) fn api(&self, store: &Store, origin: &str) -> Result<HttpApi, CoreError> {
        self.api_with(origin, trust_of(store, origin)?)
    }

    /// A client of `origin` with `trust` (the trust dialog checks a choice
    /// with it before storing it, `sync_trust.rs`).
    pub(super) fn api_with(&self, origin: &str, trust: TrustMode) -> Result<HttpApi, CoreError> {
        Ok(HttpApi::with_roots(
            origin,
            trust,
            &self.system_roots(),
            &self.config.app_version,
        )?)
    }

    fn emit_sync(&self, inner: &mut Inner) {
        self.emit(inner, CoreEventKind::SyncChanged {});
    }

    /// Runs `f` on the unlocked store and session.
    fn with_session<T>(
        &self,
        f: impl FnOnce(&mut Inner) -> Result<T, CoreError>,
    ) -> Result<T, CoreError> {
        let mut guard = self.inner();
        let inner = &mut *guard;
        if inner.state != LockState::Unlocked || inner.session.is_none() {
            return Err(CoreError::Locked);
        }
        f(inner)
    }

    // -----------------------------------------------------------------
    // Server and certificate trust
    // -----------------------------------------------------------------

    /// The connect wizard's first step: `GET /v1/server` with the stored
    /// trust for the origin.
    pub fn sync_server_check(&self, origin: &str) -> Result<ServerCheck, CoreError> {
        let origin = normalize_origin(origin).map_err(HttpError::from)?;
        let api = self.api(&self.inner().store, &origin)?;
        Self::server_check_with(&api, origin)
    }

    /// `GET /v1/server` through `api`: ready, or the certificate problem.
    pub(super) fn server_check_with(
        api: &HttpApi,
        origin: String,
    ) -> Result<ServerCheck, CoreError> {
        match api.server_info() {
            Ok(info) => {
                if !info.api_versions.iter().any(|version| version == "v1") {
                    return Err(refused("unsupported_server"));
                }
                Ok(ServerCheck::Ready {
                    origin,
                    invite_only: matches!(info.registration, wire::Registration::Invite),
                    version: info.version,
                    server_generation: info.server_generation.map(wire::B64::into_inner),
                })
            }
            Err(HttpError::Tls(problem)) => Ok(ServerCheck::Certificate {
                origin,
                problem: *problem,
            }),
            Err(error) => Err(error.into()),
        }
    }

    /// Stores the trust choice for an origin (section 2.1; storage 6.6).
    pub fn sync_server_trust(&self, origin: &str, choice: TrustChoice) -> Result<(), CoreError> {
        let origin = normalize_origin(origin).map_err(HttpError::from)?;
        let now_text = format_timestamp(self.now());
        let mut guard = self.inner();
        guard.store.write(|tx| match &choice {
            TrustChoice::System => tx.server_tls_delete(&origin),
            TrustChoice::Pin(sha256) => tx.server_tls_put(&TlsTrustRow {
                server_url: origin.clone(),
                kind: TlsTrustKind::Pinned,
                cert_sha256: *sha256,
                ca_der: None,
                created_at: now_text.clone(),
            }),
            TrustChoice::Ca(der) => tx.server_tls_put(&TlsTrustRow {
                server_url: origin.clone(),
                kind: TlsTrustKind::Ca,
                cert_sha256: crate::sync::tls::sha256(der),
                ca_der: Some(der.clone()),
                created_at: now_text.clone(),
            }),
        })?;
        Ok(())
    }

    // -----------------------------------------------------------------
    // Registration (section 4.3)
    // -----------------------------------------------------------------

    /// Binds the local data to a new account on `origin`, then pushes it.
    pub fn sync_register(&self, input: SignInInput) -> Result<SyncStatus, CoreError> {
        let _sync = self.sync_lock();
        let origin = normalize_origin(&input.origin).map_err(HttpError::from)?;
        let email = crypto::normalize_email(&input.email);
        let class = self.platform.class;
        let (context, api) = {
            let mut guard = self.inner();
            Self::require_session(&guard)?;
            if guard.store.server_account()?.is_some() {
                return Err(refused("already_bound"));
            }
            let context = lock_ops::key_context(&guard.store)?;
            if guard
                .store
                .vault_binding(&context.vault.vault_id)?
                .0
                .is_some()
            {
                return Err(refused("previous_account"));
            }
            let api = self.api(&guard.store, &origin)?;
            let _ = &mut guard;
            (context, api)
        };
        // The typed password must open the local copy: registration keeps it.
        let material = PasswordMaterial::from_row(&context.local_auth)?;
        let old_kek = material.derive_kek(&input.password, class)?;
        vault::open_wrap(&context.kek_row, &old_kek).map_err(|_| CoreError::InvalidPassword)?;
        drop(old_kek);
        let derived = derive(
            &material.kdf,
            &material.kdf_salt,
            &email,
            &input.password,
            &origin,
            class,
        )?;
        let info = api.server_info()?;
        let generation = info.server_generation.map(wire::B64::into_inner);
        let code_text = input
            .registration_code
            .as_ref()
            .map(|code| Zeroizing::new(code.expose_secret().trim().to_owned()));

        // Keys, wraps and the two new records, under the lock.
        let now = self.now();
        let now_text = format_timestamp(now);
        let (request, rows, kek_row) = self.with_session(|inner| {
            if !lock_ops::key_rows_unchanged(&inner.store, &context)? {
                return Err(CoreError::WrongState);
            }
            let ids = &inner.ids;
            let session = inner.session.as_mut().ok_or(CoreError::Locked)?;
            let vault_id = session.ring.vault_id();
            let epoch = session.ring.current_epoch();
            let vault_key = session.ring.current_key().duplicate();
            let kek_row =
                vault::wrap_row(WrapKind::Kek, vault_id, epoch, &derived.kek, &vault_key)?;
            let recovery_key = lock_ops::recovery_key_of(session)?;
            let recovery_kek = crypto::derive_recovery_kek(&recovery_key);
            let recovery_row = vault::wrap_row(
                WrapKind::Recovery,
                vault_id,
                epoch,
                &recovery_kek,
                &vault_key,
            )?;
            let mut chain = Vec::new();
            for e in 2..=epoch {
                let row = inner
                    .store
                    .key_wrap(&vault_id, e, WrapKind::Chain)?
                    .ok_or(CoreError::Damaged)?;
                chain.push(wire::ChainLink {
                    epoch: e,
                    prev_key_wrap: B64(row.wrap),
                });
            }
            // D13: the X25519 keypair and the SSH ID, created now.
            let x_secret_bytes = Key32::generate()?;
            let x_secret = x25519_dalek::StaticSecret::from(*x_secret_bytes.expose());
            let x_public = x25519_dalek::PublicKey::from(&x_secret);
            let ssh_private = crate::keys::generate::generate(
                crate::keys::generate::KeyKind::Ed25519,
                &format!("shellpp:{email}"),
                None,
            )?;
            let parsed =
                crate::keys::parse(&ssh_private).map_err(|_| CoreError::Internal("ssh id"))?;
            let keypair_data = Map::from_iter([
                (
                    "private_key".to_owned(),
                    Value::String(BASE64URL_NOPAD.encode(x_secret_bytes.expose())),
                ),
                (
                    "public_key".to_owned(),
                    Value::String(BASE64URL_NOPAD.encode(x_public.as_bytes())),
                ),
            ]);
            let ssh_data = Map::from_iter([
                (
                    "private_key".to_owned(),
                    Value::String(ssh_private.to_string()),
                ),
                (
                    "public_key".to_owned(),
                    Value::String(parsed.public_key.clone()),
                ),
            ]);
            let mut rows = Vec::new();
            for (type_name, data) in [("user_keypair", keypair_data), ("ssh_id", ssh_data)] {
                let hlc = session.hlc.tick(unix_ms(now));
                let payload = Payload::live(type_name, 1, hlc, session.device_id, data);
                let record_id = ids.next(now);
                rows.push(vault::seal_row(
                    &mut session.ring,
                    classes::VAULT,
                    record_id,
                    &payload,
                    None,
                    &now_text,
                )?);
            }
            let request = wire::RegisterRequest {
                email: email.clone(),
                registration_code: code_text.as_ref().map(|code| secret(code)),
                kdf: wire_kdf(&material.kdf),
                kdf_salt: B64(material.kdf_salt.to_vec()),
                auth_key: key_b64(&derived.auth_key),
                recovery_auth_key: key_b64(&crypto::derive_recovery_auth_key(
                    &recovery_key,
                    &origin,
                )),
                personal_vault: wire::PersonalVaultRegistration {
                    vault_id,
                    epoch,
                    kek_wrap: B64(kek_row.wrap.clone()),
                    recovery_wrap: B64(recovery_row.wrap),
                    epoch_chain: chain,
                },
                x25519_public_key: B64(x_public.as_bytes().to_vec()),
                ssh_id_public_key: parsed.public_key,
                device_id: session.device_id,
            };
            Ok((request, rows, kek_row))
        })?;

        let response = api.register(&request)?;
        drop(request);

        let status = self.with_session(|inner| {
            let session = inner.session.as_mut().ok_or(CoreError::Locked)?;
            let vault_id = session.ring.vault_id();
            let device_id = session.device_id;
            let (refresh, secrets_epoch) = wrap_refresh(
                &session.ring,
                &device_id,
                &origin,
                &response.tokens.refresh_token.0,
            )?;
            let mut local_auth = context.local_auth.clone();
            local_auth.kdf_email = Some(email.clone());
            local_auth.updated_at = now_text.clone();
            let mut vault_row = context.vault.clone();
            vault_row.bound = true;
            let account = ServerAccountRow {
                server_url: origin.clone(),
                user_id: response.user_id,
                email: email.clone(),
                refresh_token: Some(refresh),
                secrets_epoch,
                bound_at: now_text.clone(),
                server_generation: generation.clone(),
            };
            inner.store.write(|tx| {
                tx.key_wrap_put(&kek_row)?;
                tx.local_auth_put(&local_auth)?;
                tx.vault_put(&vault_row)?;
                tx.vault_binding_set(&vault_id, Some(&response.user_id), Some(&origin))?;
                tx.server_account_put(&account)?;
                tx.sync_state_put(&SyncStateRow {
                    vault_id,
                    ..SyncStateRow::default()
                })?;
                tx.meta_set(META_ACCOUNT_USER_ID, &response.user_id.to_string())?;
                tx.meta_set(META_ACCOUNT_SERVER_URL, &origin)?;
                for row in &rows {
                    tx.record_put(row)?;
                }
                Ok(())
            })?;
            inner.sync.hold_access(
                Zeroizing::new(response.tokens.access_token.0.clone()),
                &origin,
            );
            tracing::info!("account registered");
            super::checkpoint(&inner.store);
            self.emit_sync(inner);
            Ok(())
        });
        status?;
        drop(_sync);
        // The first push; a failure leaves the records pending for later.
        if let Err(error) = self.sync_now() {
            tracing::warn!(code = error.code(), "first push deferred");
        }
        self.sync_status()
    }

    // -----------------------------------------------------------------
    // Sign-in (section 4.4)
    // -----------------------------------------------------------------

    /// Signs in on `origin`. At first run this is "Restore from sync"
    /// (UI 13.2): the device gets the account's vault. With a local vault it
    /// is a rebind (the same personal vault) or the merge of data-model 18.5.
    pub fn sync_sign_in(&self, input: SignInInput) -> Result<SignInOutcome, CoreError> {
        self.sign_in_with(input, None)
    }

    /// The sign-in of a bound, signed-out device after the user confirmed
    /// the weaker KDF parameters of `sync_weaker_kdf` (D189): it goes on only
    /// when the server sends exactly `accepted` again.
    pub fn sync_sign_in_accepting_kdf(
        &self,
        input: SignInInput,
        accepted: KdfParams,
    ) -> Result<SignInOutcome, CoreError> {
        self.sign_in_with(input, Some(&accepted))
    }

    fn sign_in_with(
        &self,
        input: SignInInput,
        accepted: Option<&KdfParams>,
    ) -> Result<SignInOutcome, CoreError> {
        let _sync = self.sync_lock();
        let origin = normalize_origin(&input.origin).map_err(HttpError::from)?;
        let email = crypto::normalize_email(&input.email);
        let class = self.platform.class;
        let (api, device_id, bound_kdf) = {
            let mut guard = self.inner();
            guard.sync.weaker_kdf = None;
            let mut bound_kdf = None;
            match guard.state {
                LockState::FirstRun => {}
                LockState::Unlocked => {
                    // A bound device signed out (refused refresh, offline
                    // recovery) signs in again: the rebind below.
                    let account = guard.store.server_account()?;
                    if account
                        .as_ref()
                        .is_some_and(|row| row.refresh_token.is_some())
                    {
                        return Err(refused("already_bound"));
                    }
                    // A device bound now or before (an account row, or the
                    // personal vault's binding, which neither Log out nor
                    // an `account_deleted` answer clears) compares with its
                    // own parameters, whatever the origin (D189): a server
                    // must not turn the check off by unbinding the device.
                    let vault_bound = match guard.personal_vault_id {
                        Some(vault_id) => guard.store.vault_binding(&vault_id)?.0.is_some(),
                        None => false,
                    };
                    if account.is_some() || vault_bound {
                        let row = guard.store.local_auth()?.ok_or(CoreError::Damaged)?;
                        bound_kdf = Some(
                            KdfParams::from_json(&row.kdf_json).map_err(|_| CoreError::Damaged)?,
                        );
                    }
                }
                _ => return Err(CoreError::Locked),
            }
            let device_id = guard
                .store
                .meta_get(META_DEVICE_ID)?
                .and_then(|text| Uuid::parse_str(&text).ok());
            (self.api(&guard.store, &origin)?, device_id, bound_kdf)
        };
        let device_id = match device_id {
            Some(id) => id,
            None => self.inner().ids.next(self.now()),
        };
        let prelogin = api.prelogin(&email)?;
        let kdf = local_kdf(&prelogin.kdf);
        let kdf_salt: [u8; 16] = prelogin
            .kdf_salt
            .exact()
            .ok_or_else(|| refused("unsafe_kdf"))?;
        kdf.check(class).map_err(|_| refused("unsafe_kdf"))?;
        // Weaker than this device's own parameters: a possible downgrade
        // attack (D189). Nothing is derived or sent until the user confirms
        // these very parameters.
        if let Some(local) = bound_kdf
            && super::sync_trust::kdf_weaker(&kdf, &local)
            && accepted != Some(&kdf)
        {
            self.inner().sync.weaker_kdf =
                Some(super::sync_trust::WeakerKdf { local, server: kdf });
            return Err(refused("weaker_kdf"));
        }
        let derived = derive(&kdf, &kdf_salt, &email, &input.password, &origin, class)?;
        drop(input);
        let response = api.login(&wire::LoginRequest {
            email: email.clone(),
            auth_key: key_b64(&derived.auth_key),
            device_id,
        })?;
        if response.status.as_deref() == Some("totp_required") {
            let ticket = response.login_ticket.ok_or(HttpError::Malformed)?;
            self.inner().sync.pending = Some(PendingSignIn {
                started: self.now(),
                origin,
                email,
                ticket: Zeroizing::new(ticket.into_inner()),
                kek: derived.kek,
                kdf,
                kdf_salt,
            });
            return Ok(SignInOutcome::TotpRequired);
        }
        let tokens = response.tokens.ok_or(HttpError::Malformed)?;
        let account = response.account.ok_or(HttpError::Malformed)?;
        let pending = PendingSignIn {
            started: self.now(),
            origin,
            email,
            ticket: Zeroizing::new(String::new()),
            kek: derived.kek,
            kdf,
            kdf_salt,
        };
        self.finish_sign_in(&api, pending, device_id, tokens, account)?;
        drop(_sync);
        Ok(SignInOutcome::Done(self.sync_status()?))
    }

    /// The wizard's Cancel: the derived key of a TOTP step goes, and so
    /// does the vault an abandoned merge at sign-in left (run 7 finding 18).
    /// While a sign-in still runs nothing is removed; the next unlock does
    /// it.
    pub fn sync_sign_in_cancel(&self) {
        self.inner().sync.pending = None;
        self.abandoned_merge_cleanup();
    }

    /// Removes the account vaults an interrupted merge at sign-in left
    /// (data-model 18.5): personal vaults that are neither the local
    /// personal vault nor bound nor archived, with their records and rows,
    /// and the `.rekey` files of recordings that name them. The local
    /// vault's data is never touched: the merge copies it, and only its
    /// last transaction switches the personal vault. Runs only when no
    /// sign-in or sync is talking to the server (the guard is free).
    pub(super) fn abandoned_merge_cleanup(&self) {
        let Ok(_sync) = self.sync_guard.try_lock() else {
            return;
        };
        let removed = (|| -> Result<Vec<Uuid>, CoreError> {
            let mut guard = self.inner();
            let Some(personal) = guard.personal_vault_id else {
                return Ok(Vec::new());
            };
            let leftovers: Vec<Uuid> = guard
                .store
                .vaults()?
                .into_iter()
                .filter(|vault| {
                    vault.kind == crate::service::dto::VaultKind::Personal
                        && vault.status == crate::service::dto::VaultStatus::Active
                        && !vault.bound
                        && vault.vault_id != personal
                })
                .map(|vault| vault.vault_id)
                .collect();
            // A merge names its account vault's account only in its last
            // transaction: a vault that names one was bound and is kept.
            let mut unbound = Vec::new();
            for vault_id in leftovers {
                if guard.store.vault_binding(&vault_id)?.0.is_none() {
                    unbound.push(vault_id);
                }
            }
            let leftovers = unbound;
            if leftovers.is_empty() {
                return Ok(leftovers);
            }
            guard.store.write(|tx| {
                for vault_id in &leftovers {
                    tx.vault_purge(vault_id)?;
                }
                Ok(())
            })?;
            for vault_id in &leftovers {
                recordings::remove_rekeys_for(&guard.store, *vault_id);
            }
            Ok(leftovers)
        })();
        match removed {
            Ok(removed) if !removed.is_empty() => {
                tracing::info!(count = removed.len(), "abandoned sign-in merge removed");
            }
            Ok(_) => {}
            Err(error) => tracing::warn!(code = error.code(), "abandoned merge not removed"),
        }
    }

    /// The TOTP step of a sign-in.
    pub fn sync_sign_in_totp(&self, code: &str) -> Result<SignInOutcome, CoreError> {
        let _sync = self.sync_lock();
        let (pending, api, device_id) = {
            let mut guard = self.inner();
            let pending = guard.sync.pending.take().ok_or(CoreError::WrongState)?;
            if self.now() > pending.started + Duration::from_secs(5 * 60) {
                return Err(refused("ticket_expired"));
            }
            let api = self.api(&guard.store, &pending.origin)?;
            let device_id = guard
                .store
                .meta_get(META_DEVICE_ID)?
                .and_then(|text| Uuid::parse_str(&text).ok());
            (pending, api, device_id)
        };
        let device_id = match device_id {
            Some(id) => id,
            None => self.inner().ids.next(self.now()),
        };
        let result = api.login_totp(&wire::LoginTotpRequest {
            login_ticket: secret(&pending.ticket),
            code: secret(code.trim()),
        });
        let response = match result {
            Ok(response) => response,
            Err(error) => {
                // A wrong code keeps the ticket for another try.
                if error.api_code() == Some(code::INVALID_TOTP) {
                    self.inner().sync.pending = Some(pending);
                }
                return Err(error.into());
            }
        };
        let tokens = response.tokens.ok_or(HttpError::Malformed)?;
        let account = response.account.ok_or(HttpError::Malformed)?;
        self.finish_sign_in(&api, pending, device_id, tokens, account)?;
        drop(_sync);
        Ok(SignInOutcome::Done(self.sync_status()?))
    }

    fn finish_sign_in(
        &self,
        api: &HttpApi,
        pending: PendingSignIn,
        device_id: Uuid,
        tokens: wire::Tokens,
        account: wire::AccountBundle,
    ) -> Result<(), CoreError> {
        let vault_id = account.personal_vault.vault_id;
        let epoch = account.personal_vault.epoch;
        let vault_key = open_account_wrap(
            vault_id,
            epoch,
            account.personal_vault.kek_wrap.as_bytes(),
            &pending.kek,
            WrapKind::Kek,
        )
        .ok_or_else(|| refused("integrity"))?;
        let access = tokens.access_token.0.as_str();
        let keys = if epoch > 1 {
            api.vault_keys(access, vault_id)?.epoch_chain
        } else {
            Vec::new()
        };
        let (ring, chain_rows) = ring_from_chain(vault_id, epoch, vault_key, &keys)?;
        let state = self.inner().state;
        match state {
            LockState::FirstRun => self.sign_in_empty(
                api, &pending, device_id, &tokens, &account, ring, chain_rows,
            ),
            LockState::Unlocked => {
                let local_vault = self.inner().personal_vault_id;
                if local_vault == Some(vault_id) {
                    self.sign_in_rebind(&pending, &tokens, &account, ring, chain_rows)
                } else {
                    self.sign_in_merge(api, &pending, &tokens, &account, ring, chain_rows)
                }
            }
            _ => Err(CoreError::Locked),
        }
    }

    /// The `server_account` row and local rows every binding writes.
    #[allow(clippy::too_many_arguments)]
    fn binding_rows(
        &self,
        ring: &KeyRing,
        device_id: &Uuid,
        pending: &PendingSignIn,
        tokens: &wire::Tokens,
        account: &wire::AccountBundle,
        generation: Option<Vec<u8>>,
        now_text: &str,
    ) -> Result<(ServerAccountRow, LocalAuthRow, KeyWrapRow), CoreError> {
        let (refresh, secrets_epoch) =
            wrap_refresh(ring, device_id, &pending.origin, &tokens.refresh_token.0)?;
        let vault_id = ring.vault_id();
        let account_row = ServerAccountRow {
            server_url: pending.origin.clone(),
            user_id: account.user_id,
            email: pending.email.clone(),
            refresh_token: Some(refresh),
            secrets_epoch,
            bound_at: now_text.to_owned(),
            server_generation: generation,
        };
        let local_auth = LocalAuthRow {
            personal_vault_id: vault_id,
            kdf_json: pending.kdf.to_json(),
            kdf_salt: pending.kdf_salt,
            kdf_email: Some(pending.email.clone()),
            updated_at: now_text.to_owned(),
        };
        let kek_row = vault::wrap_row(
            WrapKind::Kek,
            vault_id,
            ring.current_epoch(),
            &pending.kek,
            ring.current_key(),
        )?;
        Ok((account_row, local_auth, kek_row))
    }

    fn generation(api: &HttpApi) -> Option<Vec<u8>> {
        api.server_info()
            .ok()
            .and_then(|info| info.server_generation.map(wire::B64::into_inner))
    }

    /// Sign-in on an empty device (storage 6.6): the account's vault becomes
    /// the local one, then the first pull runs.
    #[allow(clippy::too_many_arguments)]
    fn sign_in_empty(
        &self,
        api: &HttpApi,
        pending: &PendingSignIn,
        device_id: Uuid,
        tokens: &wire::Tokens,
        account: &wire::AccountBundle,
        ring: KeyRing,
        chain_rows: Vec<KeyWrapRow>,
    ) -> Result<(), CoreError> {
        let generation = Self::generation(api);
        let now = self.now();
        let now_text = format_timestamp(now);
        let mut guard = self.inner();
        let inner = &mut *guard;
        if inner.state != LockState::FirstRun {
            return Err(CoreError::WrongState);
        }
        let vault_id = ring.vault_id();
        let db_id = inner.ids.next(now);
        let (account_row, local_auth, kek_row) = self.binding_rows(
            &ring, &device_id, pending, tokens, account, generation, &now_text,
        )?;
        let vault_row = VaultRow {
            vault_id,
            kind: crate::service::dto::VaultKind::Personal,
            status: crate::service::dto::VaultStatus::Active,
            role: None,
            current_epoch: ring.current_epoch(),
            bound: true,
            created_at: now_text.clone(),
        };
        let created = inner.store.write(|tx| {
            if tx.local_auth()?.is_some() {
                return Ok(false);
            }
            tx.meta_set(META_DB_ID, &db_id.to_string())?;
            tx.meta_set(META_DEVICE_ID, &device_id.to_string())?;
            tx.meta_set(META_CREATED_AT, &now_text)?;
            tx.local_auth_put(&local_auth)?;
            tx.vault_put(&vault_row)?;
            tx.key_wrap_put(&kek_row)?;
            for row in &chain_rows {
                tx.key_wrap_put(row)?;
            }
            tx.vault_binding_set(&vault_id, Some(&account.user_id), Some(&pending.origin))?;
            tx.server_account_put(&account_row)?;
            tx.sync_state_put(&SyncStateRow {
                vault_id,
                ..SyncStateRow::default()
            })?;
            tx.meta_set(META_ACCOUNT_USER_ID, &account.user_id.to_string())?;
            tx.meta_set(META_ACCOUNT_SERVER_URL, &pending.origin)?;
            Ok(true)
        })?;
        if !created {
            return Err(CoreError::WrongState);
        }
        let session = Session::new(ring, db_id, device_id, HlcClock::start(None, unix_ms(now)));
        inner.session = Some(session);
        inner.personal_vault_id = Some(vault_id);
        inner.state = LockState::Unlocked;
        inner.auto_lock.activity(now);
        inner.sync.hold_access(
            Zeroizing::new(tokens.access_token.0.clone()),
            &pending.origin,
        );
        tracing::info!("signed in on an empty device");
        self.emit(
            inner,
            CoreEventKind::LockChanged {
                locked: false,
                state: LockState::Unlocked,
            },
        );
        drop(guard);
        // The first pull brings `recovery_key`; then the recovery row, the
        // check of the server's recovery wrap, and this device's record.
        if let Err(error) = self.cycle(true) {
            // The device is bound and unlocked: later cycles finish the pull
            // and build the recovery row (`ensure_recovery_row`).
            tracing::warn!(code = error.code(), "first pull deferred");
        }
        self.after_first_pull(account)?;
        let _ = self.cycle(false);
        Ok(())
    }

    /// Sign-in to the account the personal vault was last bound to (rebind).
    fn sign_in_rebind(
        &self,
        pending: &PendingSignIn,
        tokens: &wire::Tokens,
        account: &wire::AccountBundle,
        ring: KeyRing,
        chain_rows: Vec<KeyWrapRow>,
    ) -> Result<(), CoreError> {
        let now = self.now();
        let now_text = format_timestamp(now);
        self.with_session(|inner| {
            let session = inner.session.as_mut().ok_or(CoreError::Locked)?;
            let local_epoch = session.ring.current_epoch();
            let epoch = ring.current_epoch();
            // Epochs never go back, and a held epoch keeps its key (storage
            // 6.2): the server's chain must reach this device's key.
            if epoch < local_epoch {
                // A server restored from an older backup. When its key is
                // the one held here for that epoch, the device signs in (so
                // the user can re-post the epochs) but no key row changes.
                let held = session
                    .ring
                    .key(epoch)
                    .ok_or_else(|| refused("rolled_back"))?;
                if !ring.current_key().ct_eq(held) {
                    return Err(refused("rolled_back"));
                }
                let device_id = session.device_id;
                let (account_row, _, _) = self.binding_rows(
                    &session.ring,
                    &device_id,
                    pending,
                    tokens,
                    account,
                    None,
                    &now_text,
                )?;
                let vault_id = session.ring.vault_id();
                let mut vault_row = inner.store.vault(&vault_id)?.ok_or(CoreError::Damaged)?;
                vault_row.bound = true;
                inner.store.write(|tx| {
                    tx.vault_put(&vault_row)?;
                    tx.vault_binding_set(&vault_id, Some(&account.user_id), Some(&pending.origin))?;
                    tx.server_account_put(&account_row)
                })?;
                inner.sync.hold_access(
                    Zeroizing::new(tokens.access_token.0.clone()),
                    &pending.origin,
                );
                tracing::warn!("signed in to a server whose personal epoch is behind");
                return Err(refused("rolled_back"));
            }
            let reached = ring
                .key(local_epoch)
                .ok_or_else(|| refused("rolled_back"))?;
            if !reached.ct_eq(session.ring.current_key()) {
                return Err(refused("rolled_back"));
            }
            // Newer epochs (a rotation on another device): their chain rows,
            // and the old epoch's kek, recovery and device rows go.
            let new_rows: Vec<KeyWrapRow> = chain_rows
                .into_iter()
                .filter(|row| row.epoch > local_epoch)
                .collect();
            for e in local_epoch + 1..=epoch {
                let key = ring
                    .key(e)
                    .map(Key32::duplicate)
                    .ok_or_else(|| refused("integrity"))?;
                session.ring.push_epoch(e, key);
            }
            let device_id = session.device_id;
            let (account_row, local_auth, kek_row) = self.binding_rows(
                &session.ring,
                &device_id,
                pending,
                tokens,
                account,
                None,
                &now_text,
            )?;
            let vault_id = session.ring.vault_id();
            let mut vault_row = inner.store.vault(&vault_id)?.ok_or(CoreError::Damaged)?;
            vault_row.bound = true;
            let sel = selection(&inner.store);
            let mut state = inner.store.sync_state(&vault_id)?;
            state.vault_id = vault_id;
            state.cursor = 0;
            state.resync_classes = sel.classes.clone();
            vault_row.current_epoch = epoch;
            let written = inner.store.write(|tx| {
                for row in &new_rows {
                    tx.key_wrap_put(row)?;
                }
                if epoch > local_epoch {
                    for kind in [WrapKind::Kek, WrapKind::Recovery, WrapKind::Device] {
                        tx.key_wrap_delete(&vault_id, local_epoch, kind)?;
                    }
                }
                tx.local_auth_put(&local_auth)?;
                tx.key_wrap_put(&kek_row)?;
                tx.vault_put(&vault_row)?;
                tx.vault_binding_set(&vault_id, Some(&account.user_id), Some(&pending.origin))?;
                tx.server_account_put(&account_row)?;
                tx.sync_state_put(&state)?;
                Ok(())
            });
            if let Err(error) = written {
                if let Some(session) = inner.session.as_mut() {
                    session.ring.pop_epoch(local_epoch);
                }
                return Err(error.into());
            }
            inner.sync.hold_access(
                Zeroizing::new(tokens.access_token.0.clone()),
                &pending.origin,
            );
            super::checkpoint(&inner.store);
            tracing::info!(epoch, "personal vault bound again");
            Ok(())
        })?;
        // The generation is learnt by the first cycle (a restore check
        // against nothing would push everything back for no reason).
        self.store_generation_if_unset()?;
        let _ = self.cycle(false);
        // A new `recovery_key` revision or epoch: rebuild the recovery row.
        self.after_first_pull(account)?;
        Ok(())
    }

    fn store_generation_if_unset(&self) -> Result<(), CoreError> {
        let origin = {
            let guard = self.inner();
            match guard.store.server_account()? {
                Some(row) if row.server_generation.is_none() => row.server_url,
                _ => return Ok(()),
            }
        };
        let api = self.api(&self.inner().store, &origin)?;
        if let Some(generation) = Self::generation(&api) {
            let mut guard = self.inner();
            if let Some(mut row) = guard.store.server_account()? {
                row.server_generation = Some(generation);
                guard.store.write(|tx| tx.server_account_put(&row))?;
            }
        }
        Ok(())
    }

    /// The merge at sign-in (data-model 18.5; storage 6.6): the account's
    /// personal vault replaces the local one, and every live local record
    /// is re-encrypted into it with a new id unless an account record has
    /// its natural key. A backup of the database is written first.
    fn sign_in_merge(
        &self,
        api: &HttpApi,
        pending: &PendingSignIn,
        tokens: &wire::Tokens,
        account: &wire::AccountBundle,
        ring: KeyRing,
        chain_rows: Vec<KeyWrapRow>,
    ) -> Result<(), CoreError> {
        let generation = Self::generation(api);
        let access = Zeroizing::new(tokens.access_token.0.clone());
        let account_vault = ring.vault_id();
        let now = self.now();
        let now_text = format_timestamp(now);
        // Step 1: the account vault's rows, so the pull can write records.
        let (db_id, device_id, old_vault) = self.with_session(|inner| {
            let old_vault = inner
                .session
                .as_ref()
                .map(|s| s.ring.vault_id())
                .ok_or(CoreError::Locked)?;
            if inner.store.vault_binding(&old_vault)?.0.is_some() {
                return Err(refused("previous_account"));
            }
            let session = inner.session.as_ref().ok_or(CoreError::Locked)?;
            let (db_id, device_id) = (session.db_id, session.device_id);
            inner.store.backup_now(now)?;
            let vault_row = VaultRow {
                vault_id: account_vault,
                kind: crate::service::dto::VaultKind::Personal,
                status: crate::service::dto::VaultStatus::Active,
                role: None,
                current_epoch: ring.current_epoch(),
                bound: false,
                created_at: now_text.clone(),
            };
            inner.store.write(|tx| {
                tx.vault_put(&vault_row)?;
                for row in &chain_rows {
                    tx.key_wrap_put(row)?;
                }
                tx.sync_state_put(&SyncStateRow {
                    vault_id: account_vault,
                    ..SyncStateRow::default()
                })
            })?;
            Ok((db_id, device_id, old_vault))
        })?;
        // Step 2: pull the account vault into a session of its own. Rows
        // left by an interrupted earlier attempt are loaded first, so the
        // natural-key dedupe below sees them.
        let mut new_session = {
            let guard = self.inner();
            Session::load(&guard.store, ring, db_id, device_id, now)?
        };
        let all = Selection::all();
        let mut cursor = 0;
        for _ in 0..MAX_PULL_PAGES {
            let page = api.pull(&access, account_vault, cursor, None, None)?;
            let mut guard = self.inner();
            let state = SyncStateRow {
                vault_id: account_vault,
                cursor,
                ..SyncStateRow::default()
            };
            let result = engine::apply_page(
                &mut guard.store,
                &mut new_session,
                &page.records,
                page.next_cursor,
                &state,
                &all,
                &HashMap::new(),
                false,
                now,
            )?;
            if result.stopped && page.current_epoch <= new_session.ring.current_epoch() {
                // An epoch the server does not hold (D183): never accepted.
                return Err(refused("forged_epoch"));
            }
            if result.stopped {
                return Err(refused("newer_epoch"));
            }
            cursor = page.next_cursor;
            if !page.has_more {
                break;
            }
        }
        // Step 3: re-encrypt the local records into it, in one transaction
        // with the switch of `local_auth`.
        let mut guard = self.inner();
        let inner = &mut *guard;
        if inner.state != LockState::Unlocked {
            return Err(CoreError::Locked);
        }
        #[cfg(any(test, feature = "test-hooks"))]
        if std::mem::take(&mut inner.sync.fail_merge) {
            return Err(CoreError::Internal("injected merge failure"));
        }
        let mut old_session = inner.session.take().ok_or(CoreError::Locked)?;
        let outcome = self.merge_records(inner, &mut old_session, &mut new_session, now);
        let (rows, trash_rows, pairs) = match outcome {
            Ok(done) => done,
            Err(error) => {
                inner.session = Some(old_session);
                return Err(error);
            }
        };
        let (account_row, local_auth, kek_row) = match self.binding_rows(
            &new_session.ring,
            &device_id,
            pending,
            tokens,
            account,
            generation,
            &now_text,
        ) {
            Ok(done) => done,
            Err(error) => {
                inner.session = Some(old_session);
                return Err(error);
            }
        };
        let mut vault_row = VaultRow {
            vault_id: account_vault,
            kind: crate::service::dto::VaultKind::Personal,
            status: crate::service::dto::VaultStatus::Active,
            role: None,
            current_epoch: new_session.ring.current_epoch(),
            bound: true,
            created_at: now_text.clone(),
        };
        if let Ok(Some(existing)) = inner.store.vault(&account_vault) {
            vault_row.created_at = existing.created_at;
        }
        // The recordings follow their vault (storage 8.4): their keys are
        // kept for the account vault before the old vault's keys go.
        if let Err(error) =
            recordings::prepare_rekeys(&inner.store, &old_session.ring, &new_session.ring)
        {
            inner.session = Some(old_session);
            return Err(error);
        }
        let written = inner.store.write(|tx| {
            tx.vault_put(&vault_row)?;
            tx.key_wrap_put(&kek_row)?;
            tx.local_auth_put(&local_auth)?;
            for row in &rows {
                tx.record_put(row)?;
            }
            for row in &trash_rows {
                tx.recently_deleted_put(row)?;
            }
            for pair in &pairs {
                tx.duplicate_pair_put(pair)?;
            }
            tx.vault_binding_set(
                &account_vault,
                Some(&account.user_id),
                Some(&pending.origin),
            )?;
            tx.server_account_put(&account_row)?;
            tx.meta_set(META_ACCOUNT_USER_ID, &account.user_id.to_string())?;
            tx.meta_set(META_ACCOUNT_SERVER_URL, &pending.origin)?;
            tx.vault_purge(&old_vault)
        });
        if let Err(error) = written {
            inner.session = Some(old_session);
            return Err(error.into());
        }
        drop(old_session);
        let session = Session::load(&inner.store, new_session.ring, db_id, device_id, now)?;
        inner.session = Some(session);
        inner.personal_vault_id = Some(account_vault);
        inner.sync.hold_access(access, &pending.origin);
        super::checkpoint(&inner.store);
        tracing::info!(merged = rows.len(), "local data merged into the account");
        self.emit(
            inner,
            CoreEventKind::VaultsChanged {
                vault_ids: vec![old_vault, account_vault],
            },
        );
        drop(guard);
        self.recordings_resume();
        self.after_first_pull(account)?;
        let _ = self.cycle(false);
        Ok(())
    }

    /// The rows of the merged records and Recently deleted entries.
    fn merge_records(
        &self,
        inner: &mut Inner,
        old: &mut Session,
        new: &mut Session,
        now: SystemTime,
    ) -> Result<MergedRows, CoreError> {
        let now_text = format_timestamp(now);
        let old_vault = old.ring.vault_id();
        let new_vault = new.ring.vault_id();
        // Natural keys of the account's live records (data-model 18.6, in
        // the account vault).
        let mut account_keys: HashMap<(RecordKind, String), Vec<Uuid>> = HashMap::new();
        {
            let keys = natural::Keys::new(&new.index);
            for record in new.index.iter() {
                if let Some(key) = keys.key_of(record) {
                    account_keys
                        .entry((record.kind, key))
                        .or_default()
                        .push(record.record_id);
                }
            }
        }
        struct Item {
            old_id: Uuid,
            new_id: Uuid,
            class: u8,
            type_name: String,
            v: u32,
            data: Map<String, Value>,
            /// The account records with the same natural key.
            twins: Vec<Uuid>,
            /// The account record it is dropped for (equal data).
            dropped: Option<Uuid>,
        }
        // Every local record that moves, with a new id and the account
        // records of its natural key (computed in the local vault: parents
        // by their natural keys, a host's effective port and username).
        let mut items = Vec::new();
        {
            let local_keys = natural::Keys::new(&old.index);
            for row in inner.store.records(&old_vault, None)? {
                if row.tombstone {
                    continue;
                }
                let Some(payload) = open_payload(&mut old.ring, &row) else {
                    continue; // Damaged rows stay in the backup.
                };
                if NOT_MERGED.contains(&payload.type_name.as_str()) || payload.deleted {
                    continue;
                }
                let Some(data) = payload.data.clone() else {
                    continue;
                };
                let twins = RecordKind::from_name(&payload.type_name)
                    .and_then(|kind| {
                        let key = match old.index.get(&row.record_id) {
                            Some(record) if record.kind == kind => local_keys.key_of(record),
                            _ => local_keys.key(kind, &data),
                        };
                        key.and_then(|key| account_keys.get(&(kind, key)))
                    })
                    .cloned()
                    .unwrap_or_default();
                items.push(Item {
                    old_id: row.record_id,
                    new_id: inner.ids.next(now),
                    class: row.class,
                    type_name: payload.type_name.clone(),
                    v: payload.v,
                    data,
                    twins,
                    dropped: None,
                });
            }
        }
        // A record is dropped only when its data, with the references
        // rewritten to the records they map to, equals an account record of
        // its natural key. The references are compared after rewriting (run
        // 7 finding 6), as the greatest set of drops that holds: every
        // record with an account twin starts as dropped, and a record whose
        // rewritten data matches no twin is kept, until nothing changes. So
        // neither the order of the rows nor references in a cycle (a group
        // whose jump host is in the group) leave an extra pair, and every
        // drop is checked against the final map. A record that only shares
        // the natural key stays beside it as a duplicate pair (UI 13.7):
        // principle 3 before tidiness.
        let account_data: HashMap<Uuid, &Map<String, Value>> = items
            .iter()
            .flat_map(|item| item.twins.iter())
            .filter_map(|id| new.index.get(id).map(|record| (*id, record.data())))
            .collect();
        for item in &mut items {
            item.dropped = item.twins.first().copied();
        }
        let map_of = |items: &[Item]| -> HashMap<String, String> {
            items
                .iter()
                .map(|item| {
                    let to = item.dropped.unwrap_or(item.new_id);
                    (item.old_id.to_string(), to.to_string())
                })
                .collect()
        };
        let equal = |id: &Uuid, data: &Map<String, Value>| {
            account_data
                .get(id)
                .is_some_and(|account| engine::data_equal(Some(account), Some(data)))
        };
        // Choosing among several twins may change other records' choices;
        // the rounds are bounded, and the last loop below only keeps.
        for _ in 0..=items.len() {
            let map = map_of(&items);
            let mut changed = false;
            for item in items.iter_mut() {
                let Some(current) = item.dropped else {
                    continue;
                };
                let data = remap_data(&item.data, &map);
                if equal(&current, &data) {
                    continue;
                }
                item.dropped = item.twins.iter().copied().find(|twin| equal(twin, &data));
                changed = true;
            }
            if !changed {
                break;
            }
        }
        loop {
            let map = map_of(&items);
            let mut changed = false;
            for item in items.iter_mut() {
                if let Some(current) = item.dropped
                    && !equal(&current, &remap_data(&item.data, &map))
                {
                    item.dropped = None;
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        let map = map_of(&items);
        let mut pairs = Vec::new();
        for item in items.iter().filter(|item| item.dropped.is_none()) {
            if let Some(existing) = item.twins.first() {
                pairs.push(DuplicatePairRow {
                    vault_id: new_vault,
                    record_id: *existing,
                    other_record_id: item.new_id,
                    found_at: now_text.clone(),
                    kept: false,
                });
            }
        }
        items.retain(|item| item.dropped.is_none());
        let mut rows = Vec::with_capacity(items.len());
        for item in &items {
            let data = remap_data(&item.data, &map);
            let hlc = new.hlc.tick(unix_ms(now));
            let payload = Payload::live(&item.type_name, item.v, hlc, new.device_id, data);
            rows.push(vault::seal_row(
                &mut new.ring,
                item.class,
                item.new_id,
                &payload,
                None,
                &now_text,
            )?);
        }
        // Recently deleted entries move with new ids (D93).
        let mut trash_rows = Vec::new();
        for entry in inner.store.recently_deleted()? {
            if entry.vault_id != old_vault {
                continue;
            }
            let Some(payload) = open_envelope_payload(&mut old.ring, &entry.envelope) else {
                continue;
            };
            let Some(data) = payload.data.as_ref() else {
                continue;
            };
            let new_id = inner.ids.next(now);
            let payload = Payload::live(
                &payload.type_name,
                payload.v,
                payload.hlc,
                payload.device_id,
                remap_data(data, &map),
            );
            let row = vault::seal_row(
                &mut new.ring,
                entry.class,
                new_id,
                &payload,
                None,
                &now_text,
            )?;
            trash_rows.push(RecentlyDeletedRow {
                vault_id: new_vault,
                record_id: new_id,
                class: entry.class,
                deleted_at: entry.deleted_at,
                origin: entry.origin,
                envelope: row.envelope,
            });
        }
        Ok((rows, trash_rows, pairs))
    }

    /// After the first pull of a personal vault (section 4.4): the local
    /// `recovery` row from the pulled `recovery_key` record, the check of
    /// the server's `recovery_wrap`, and this device's `device_info`.
    fn after_first_pull(&self, account: &wire::AccountBundle) -> Result<(), CoreError> {
        let now = self.now();
        let mut written = Vec::new();
        self.with_session(|inner| {
            let session = inner.session.as_mut().ok_or(CoreError::Locked)?;
            let vault_id = session.ring.vault_id();
            let epoch = session.ring.current_epoch();
            if let Ok(recovery_key) = lock_ops::recovery_key_of(session) {
                let recovery_kek = crypto::derive_recovery_kek(&recovery_key);
                let row = vault::wrap_row(
                    WrapKind::Recovery,
                    vault_id,
                    epoch,
                    &recovery_kek,
                    session.ring.current_key(),
                )?;
                inner.store.write(|tx| tx.key_wrap_put(&row))?;
                let server = open_account_wrap(
                    vault_id,
                    account.personal_vault.epoch,
                    account.personal_vault.recovery_wrap.as_bytes(),
                    &recovery_kek,
                    WrapKind::Recovery,
                );
                let matches = server.is_some_and(|key| {
                    account.personal_vault.epoch == epoch && key.ct_eq(session.ring.current_key())
                });
                inner.sync.recovery_mismatch = !matches;
                // The bundle may predate the record pulled after it (a
                // reissue in between): the next cycle checks again.
                inner.sync.recovery_checked = None;
                if !matches {
                    tracing::warn!("the server's recovery wrap does not hold the vault key");
                }
            }
            let device = session.device_id.to_string();
            let has_record = session.index.of_kind(RecordKind::DeviceInfo).any(|record| {
                record.data().get("device_id").and_then(Value::as_str) == Some(device.as_str())
            });
            if !has_record {
                let data = Map::from_iter([
                    ("device_id".to_owned(), Value::String(device)),
                    (
                        "name".to_owned(),
                        Value::String(lock_ops::device_name(
                            &self.platform.device_name,
                            self.platform.os,
                        )),
                    ),
                    (
                        "platform".to_owned(),
                        Value::String(lock_ops::os_name(self.platform.os).to_owned()),
                    ),
                    (
                        "app_version".to_owned(),
                        Value::String(lock_ops::short_version(&self.config.app_version)),
                    ),
                ]);
                let record_id = inner.ids.next(now);
                let write = session.prepare_write(
                    &inner.store,
                    RecordKind::DeviceInfo,
                    record_id,
                    data,
                    now,
                )?;
                inner.store.write(|tx| tx.record_put(&write.row))?;
                session.commit(vec![write]);
                written.push(record_id);
            }
            if !written.is_empty() {
                self.emit(
                    inner,
                    CoreEventKind::RecordsChanged {
                        vault_id,
                        record_ids: written.clone(),
                        classes: vec![classes::VAULT],
                    },
                );
            }
            Ok(())
        })
    }

    // -----------------------------------------------------------------
    // Tokens
    // -----------------------------------------------------------------

    fn link(&self) -> Result<Link, CoreError> {
        let guard = self.inner();
        let session = Self::require_session(&guard)?;
        let account = guard.store.server_account()?.ok_or(CoreError::WrongState)?;
        Ok(Link {
            api: self.api(&guard.store, &account.server_url)?,
            vault_id: session.ring.vault_id(),
            device_id: session.device_id,
        })
    }

    /// A new access token with the stored refresh token; the rotated
    /// refresh token is stored. A refused refresh signs this device out.
    fn refresh(&self, link: &Link) -> Result<Zeroizing<String>, CoreError> {
        // The wrapping key of the successor is taken now: a lock during the
        // request must not lose the rotated token (the old one stops working
        // 60 s later, D93).
        let (token, secrets_key, secrets_epoch) = {
            let guard = self.inner();
            let session = Self::require_session(&guard)?;
            let account = guard.store.server_account()?.ok_or(CoreError::WrongState)?;
            // The address changed after this client was made: nothing is
            // sent; the next cycle uses the stored one (D188).
            if account.server_url != link.api.origin() {
                return Err(refused("signed_out"));
            }
            let Some(token) = open_refresh(&session.ring, &link.device_id, &account) else {
                // A stored token that does not open for this origin (the
                // server address changed, D188, or the blob is damaged) is
                // never sent anywhere: the device signs in again.
                let stored = account.refresh_token.is_some();
                drop(guard);
                if stored {
                    self.signed_out_locally()?;
                }
                return Err(refused("signed_out"));
            };
            let epoch = session.ring.current_epoch();
            let key = crypto::derive_local_secrets_key(
                session.ring.current_key(),
                &link.device_id,
                epoch,
            );
            (token, key, epoch)
        };
        // One retry of a lost answer within the server's 60 s window.
        let result = match link.api.refresh(&token) {
            Err(HttpError::Network | HttpError::Timeout) => link.api.refresh(&token),
            other => other,
        };
        match result {
            Ok(tokens) => {
                let access = Zeroizing::new(tokens.access_token.0.clone());
                let blob = crypto::wrap_seal(
                    &secrets_key,
                    &crypto::refresh_token_aad(&link.device_id, link.api.origin()),
                    tokens.refresh_token.0.as_bytes(),
                )?;
                let mut guard = self.inner();
                if let Some(mut row) = guard.store.server_account()? {
                    row.refresh_token = Some(blob);
                    row.secrets_epoch = secrets_epoch;
                    guard.store.write(|tx| tx.server_account_put(&row))?;
                }
                if Self::require_session(&guard).is_err() {
                    return Err(CoreError::Locked);
                }
                guard.sync.hold_access(access.clone(), link.api.origin());
                Ok(access)
            }
            Err(error) => {
                let code = error.api_code().map(str::to_owned);
                match code.as_deref() {
                    Some(code::REFRESH_TOKEN_REUSED) | Some(code::UNAUTHENTICATED) => {
                        self.signed_out_locally()?;
                    }
                    Some(code::ACCOUNT_DELETED) => self.account_deleted_locally()?,
                    _ => {}
                }
                Err(error.into())
            }
        }
    }

    /// Runs an authenticated call, refreshing the access token once when it
    /// is missing or refused.
    fn authed<T>(
        &self,
        link: &Link,
        call: impl Fn(&HttpApi, &str) -> Result<T, HttpError>,
    ) -> Result<T, CoreError> {
        // Only a token of this client's origin (D188).
        let held = {
            let inner = self.inner();
            inner
                .sync
                .access
                .clone()
                .filter(|_| inner.sync.access_origin.as_deref() == Some(link.api.origin()))
        };
        let token = match held {
            Some(token) => token,
            None => self.refresh(link)?,
        };
        match call(&link.api, &token) {
            Err(error) if error.api_code() == Some(code::UNAUTHENTICATED) => {
                self.inner().sync.access = None;
                let token = self.refresh(link)?;
                call(&link.api, &token).map_err(|error| self.on_call_error(error))
            }
            other => other.map_err(|error| self.on_call_error(error)),
        }
    }

    fn on_call_error(&self, error: HttpError) -> CoreError {
        if error.api_code() == Some(code::ACCOUNT_DELETED)
            && let Err(local) = self.account_deleted_locally()
        {
            tracing::warn!(code = local.code(), "account deletion not recorded");
        }
        error.into()
    }

    /// The refresh token was refused: the device stays bound, signed out.
    fn signed_out_locally(&self) -> Result<(), CoreError> {
        let mut guard = self.inner();
        guard.sync.access = None;
        if let Some(mut row) = guard.store.server_account()? {
            row.refresh_token = None;
            guard.store.write(|tx| tx.server_account_put(&row))?;
        }
        tracing::info!("signed out: the refresh token was refused");
        self.emit_sync(&mut guard);
        Ok(())
    }

    /// `401 account_deleted` (storage 6.6): the account row goes, the
    /// personal vault stays as a local copy.
    fn account_deleted_locally(&self) -> Result<(), CoreError> {
        let mut guard = self.inner();
        let inner = &mut *guard;
        inner.sync.access = None;
        let Some(vault_id) = inner.personal_vault_id else {
            return Ok(());
        };
        let vault = inner.store.vault(&vault_id)?;
        inner.store.write(|tx| {
            tx.server_account_delete()?;
            if let Some(mut vault) = vault {
                vault.bound = false;
                tx.vault_put(&vault)?;
            }
            Ok(())
        })?;
        tracing::info!("the account was deleted; the local copy stays");
        self.emit_sync(inner);
        Ok(())
    }

    // -----------------------------------------------------------------
    // The sync cycle (section 6.11)
    // -----------------------------------------------------------------

    /// "Sync now" (the user's action): items the server refused earlier
    /// (`quota_exceeded` and the like) are tried again (section 6.5), then
    /// one cycle runs. A no-op without a bound, signed-in account.
    pub fn sync_now(&self) -> Result<SyncReport, CoreError> {
        let _sync = self.sync_lock();
        self.inner().sync.failed.clear();
        self.inner().sync.backoff_until = None;
        self.cycle(false)
    }

    /// "Reset sync state of this vault" (section 6.9; UI 12.2; D173, item
    /// 1.7), after the user's confirmation: a full resync of the personal
    /// vault from cursor 0, in which a record whose header names a key
    /// epoch above this device's while the server reports none above it is
    /// refused like a damaged one (not applied, the cursor goes on, the
    /// integrity warning counts it). Such an epoch is never accepted: no
    /// key row changes. Local data is not touched: records the snapshot
    /// lacks are kept and pushed back, as in every full resync.
    pub fn sync_reset_vault_state(&self) -> Result<SyncReport, CoreError> {
        let _sync = self.sync_lock();
        {
            let mut guard = self.inner();
            let inner = &mut *guard;
            let session = Self::require_session(inner)?;
            let vault_id = session.ring.vault_id();
            match inner.store.server_account()? {
                Some(row) if row.refresh_token.is_some() => {}
                _ => return Err(CoreError::WrongState),
            }
            let classes = selection(&inner.store).classes;
            let mut state = inner.store.sync_state(&vault_id)?;
            state.vault_id = vault_id;
            state.cursor = 0;
            state.resync_classes = classes;
            state.last_error = None;
            inner.store.write(|tx| tx.sync_state_put(&state))?;
            inner.sync.reset_refuse_newer = true;
            inner.sync.failed.clear();
            inner.sync.backoff_until = None;
            tracing::info!("sync state reset by the user");
        }
        let result = self.cycle(false);
        self.inner().sync.reset_refuse_newer = false;
        result
    }

    /// The automatic cycle (after unlock, after a change, every 60 s):
    /// refused items stay parked; nothing runs during the retry pause.
    pub fn sync_auto(&self) -> Result<SyncReport, CoreError> {
        if !self.sync_due() {
            return Ok(SyncReport::default());
        }
        let _sync = self.sync_lock();
        self.cycle(false)
    }

    /// `true` when an automatic cycle may run now (bound, signed in, past
    /// the retry pause).
    pub fn sync_due(&self) -> bool {
        let guard = self.inner();
        if Self::require_session(&guard).is_err() {
            return false;
        }
        let signed_in =
            matches!(guard.store.server_account(), Ok(Some(row)) if row.refresh_token.is_some());
        signed_in
            && guard
                .sync
                .backoff_until
                .is_none_or(|until| self.now() >= until)
    }

    fn cycle(&self, first: bool) -> Result<SyncReport, CoreError> {
        {
            let guard = self.inner();
            if Self::require_session(&guard).is_err() {
                return Ok(SyncReport::default());
            }
            match guard.store.server_account()? {
                Some(row) if row.refresh_token.is_some() => {}
                _ => return Ok(SyncReport::default()),
            }
        }
        let link = self.link()?;
        let result = self.cycle_inner(&link, first);
        let now = self.now();
        let mut guard = self.inner();
        let inner = &mut *guard;
        let error_code = result.as_ref().err().map(error_code);
        if let Ok(mut state) = inner.store.sync_state(&link.vault_id) {
            state.vault_id = link.vault_id;
            match &result {
                Ok(_) => {
                    state.last_sync_at = Some(format_timestamp(now));
                    state.last_error = None;
                }
                Err(_) => state.last_error = error_code.clone(),
            }
            let _ = inner.store.write(|tx| tx.sync_state_put(&state));
        }
        match &result {
            Ok(report) => {
                inner.sync.failures = 0;
                inner.sync.backoff_until = None;
                inner.sync.integrity_warnings += report.integrity_warnings;
                inner.sync.newer_epoch = report.newer_epoch;
            }
            Err(CoreError::Sync(error)) if error.retryable() => {
                inner.sync.failures = inner.sync.failures.saturating_add(1);
                let wait = match error {
                    HttpError::Api {
                        retry_after: Some(seconds),
                        ..
                    } => Duration::from_secs(*seconds),
                    _ => Duration::from_secs(5u64 << inner.sync.failures.min(8)),
                };
                inner.sync.backoff_until = Some(now + wait.min(MAX_BACKOFF));
            }
            Err(_) => {}
        }
        self.emit_sync(inner);
        result
    }

    fn cycle_inner(&self, link: &Link, first: bool) -> Result<SyncReport, CoreError> {
        // The confirmed "Reset sync state" runs this cycle only.
        let refuse_newer = std::mem::take(&mut self.inner().sync.reset_refuse_newer);
        let mut report = SyncReport::default();
        let mut changed: Vec<Uuid> = Vec::new();
        let mut changed_classes: Vec<u8> = Vec::new();
        let purge_cutoff = self.now() - Duration::from_secs(trash::KEEP_DAYS * 86_400);
        {
            let mut guard = self.inner();
            let cutoff = format_timestamp(purge_cutoff);
            guard.store.write(|tx| {
                tx.recently_deleted_purge(&cutoff)?;
                tx.resync_returned_purge(&cutoff)?;
                Ok(())
            })?;
        }
        // A server restored from a backup (section 6.9): a new generation.
        let info = link.api.server_info()?;
        let generation = info.server_generation.map(wire::B64::into_inner);
        let stored = self
            .inner()
            .store
            .server_account()?
            .and_then(|row| row.server_generation);
        if !first && generation.is_some() && stored.is_some() && generation != stored {
            self.restore(link, &mut report, &mut changed, refuse_newer)?;
        }
        if generation.is_some() && generation != stored {
            let mut guard = self.inner();
            if let Some(mut row) = guard.store.server_account()? {
                row.server_generation = generation.clone();
                guard.store.write(|tx| tx.server_account_put(&row))?;
            }
        }
        // Pull.
        let selection = selection(&self.inner().store);
        let mut received: HashSet<Uuid> = HashSet::new();
        // The revision the server holds of each authenticated version, for
        // the rollbacks a full resync repairs (below).
        let mut held_by_server: HashMap<Uuid, u64> = HashMap::new();
        let mut resync = self
            .inner()
            .store
            .sync_state(&link.vault_id)?
            .resync_classes;
        if !resync.is_empty() {
            self.set_cursor(link.vault_id, 0)?;
        }
        let mut pages = 0;
        // The pull reached `has_more = false` (a resync needs the whole list).
        let mut complete = false;
        loop {
            pages += 1;
            if pages > MAX_PULL_PAGES {
                break;
            }
            let state = self.inner().store.sync_state(&link.vault_id)?;
            let pulled = self.authed(link, |api, token| {
                api.pull(
                    token,
                    link.vault_id,
                    state.cursor,
                    None,
                    Some(&selection.classes),
                )
            });
            let page = match pulled {
                Ok(page) => page,
                Err(CoreError::Sync(HttpError::Api {
                    status: 410,
                    details,
                    ..
                })) => {
                    let reason = details
                        .get("reason")
                        .and_then(Value::as_str)
                        .unwrap_or("purged");
                    if reason == "ahead" {
                        self.restore(link, &mut report, &mut changed, refuse_newer)?;
                        report.restored = true;
                        continue;
                    }
                    // Purged tombstones: a full resync of the selection.
                    resync = selection.classes.clone();
                    self.set_resync(link.vault_id, &resync)?;
                    received.clear();
                    continue;
                }
                Err(error) => return Err(error),
            };
            let local_epoch = self.with_session(|inner| {
                Ok(inner
                    .session
                    .as_ref()
                    .map(|s| s.ring.current_epoch())
                    .unwrap_or(0))
            })?;
            if page.current_epoch > local_epoch {
                report.newer_epoch = true;
            } else if page.current_epoch < local_epoch {
                // "Server state rolled back" (section 5.1): nothing changes
                // until the user re-posts the epochs.
                return Err(refused("rolled_back"));
            }
            let result = self.with_session(|inner| {
                let session = inner.session.as_mut().ok_or(CoreError::Locked)?;
                if session.ring.vault_id() != link.vault_id {
                    return Err(CoreError::WrongState);
                }
                let mut state = inner.store.sync_state(&link.vault_id)?;
                state.vault_id = link.vault_id;
                engine::apply_page(
                    &mut inner.store,
                    session,
                    &page.records,
                    page.next_cursor,
                    &state,
                    &selection,
                    &HashMap::new(),
                    refuse_newer && page.current_epoch <= local_epoch,
                    self.now(),
                )
            })?;
            if result.stopped && page.current_epoch <= local_epoch {
                // A header names an epoch the server itself does not hold:
                // forged or broken. Never accepted; the vault's sync stops
                // until the user resets its sync state (section 6.9; D173,
                // item 1.7). What the page applied before it is kept.
                return Err(refused("forged_epoch"));
            }
            report.pulled += page.records.len();
            report.conflicts += result.effects.conflicts;
            report.integrity_warnings += result.effects.integrity_warnings;
            report.newer_epoch |= result.effects.newer_epoch;
            changed.extend(result.effects.changed.iter().copied());
            changed_classes.extend(result.effects.classes.iter().copied());
            received.extend(result.received.iter().map(|(id, _)| *id));
            for (id, revision) in &result.received {
                let entry = held_by_server.entry(*id).or_insert(0);
                *entry = (*entry).max(*revision);
            }
            if !page.has_more && !result.stopped {
                complete = true;
            }
            if result.stopped || !page.has_more {
                break;
            }
        }
        if !resync.is_empty() && !report.newer_epoch && complete {
            let back = self.with_session(|inner| {
                let session = inner.session.as_mut().ok_or(CoreError::Locked)?;
                let ids = engine::push_back_missing(
                    &mut inner.store,
                    session,
                    &received,
                    &resync,
                    true,
                    self.now(),
                )?;
                // A version older than the one this device synced (a server
                // restored while the device was out of the account: the
                // rebind has no generation to compare) is skipped by the
                // pull; the restore's mark pushes the newer local version,
                // an edit or a deletion, again (section 6.9, step 2).
                engine::restore_mark(&mut inner.store, session, &held_by_server, &resync)?;
                let mut state = inner.store.sync_state(&link.vault_id)?;
                state.vault_id = link.vault_id;
                state.resync_classes.clear();
                inner.store.write(|tx| tx.sync_state_put(&state))?;
                Ok(ids)
            })?;
            report.brought_back += back.len();
        }
        // The server's `recovery_wrap`, until it is found right (after the
        // pull, so a failing push does not skip it).
        if let Err(error) = self.check_recovery_wrap(link) {
            tracing::warn!(code = error.code(), "recovery wrap not checked");
        }
        // Push.
        if !report.newer_epoch {
            // Batches continue while records wait; rounds that only re-push
            // after conflicts are capped (section 6.11).
            let mut repush_rounds = 0;
            let mut stale_rounds = 0;
            // Items still answered `stale_key_epoch` after the one retry:
            // left out for the rest of this cycle (pending, not refused).
            let mut stale_skip: HashSet<Uuid> = HashSet::new();
            for _ in 0..MAX_PUSH_BATCHES {
                let mut skip: HashSet<Uuid> = self.inner().sync.failed.keys().copied().collect();
                skip.extend(stale_skip.iter().copied());
                let batch = self.with_session(|inner| {
                    let session = inner.session.as_mut().ok_or(CoreError::Locked)?;
                    engine::build_push(&mut inner.store, session, &selection, &skip, self.now())
                })?;
                if batch.items.is_empty() {
                    break;
                }
                let items = batch.items.clone();
                let response = self.authed(link, |api, token| {
                    api.push(token, link.vault_id, items.clone())
                })?;
                let effects = self.with_session(|inner| {
                    let session = inner.session.as_mut().ok_or(CoreError::Locked)?;
                    engine::apply_push_results(
                        &mut inner.store,
                        session,
                        &batch.sent,
                        &response.results,
                        &selection,
                        self.now(),
                    )
                })?;
                report.pushed += response
                    .results
                    .iter()
                    .filter(|result| result.status == wire::ItemStatus::Ok)
                    .count();
                report.conflicts += effects.conflicts;
                report.integrity_warnings += effects.integrity_warnings;
                report.failed += effects.failed.len();
                report.newer_epoch |= effects.newer_epoch;
                changed.extend(effects.changed.iter().copied());
                changed_classes.extend(effects.classes.iter().copied());
                // Section 6.5: a pushed deletion frees room, so the items
                // refused for the quota go again (in this cycle).
                let mut retry = false;
                {
                    let mut guard = self.inner();
                    let failed = &mut guard.sync.failed;
                    failed.extend(effects.failed.iter().cloned());
                    if effects.pushed_deletion {
                        let before = failed.len();
                        failed.retain(|_, status| status != "quota_exceeded");
                        retry = failed.len() != before;
                    }
                }
                // `stale_key_epoch`: the keys, then the push again in this
                // cycle (section 6.5). The personal vault's newer key comes
                // only with a sign-in (section 5.1): the records stay
                // pending, not refused.
                if !effects.stale_key_epoch.is_empty() && stale_rounds > 0 {
                    stale_skip.extend(effects.stale_key_epoch.iter().copied());
                    retry = true;
                } else if !effects.stale_key_epoch.is_empty() {
                    let keys =
                        self.authed(link, |api, token| api.vault_keys(token, link.vault_id))?;
                    let local = self.with_session(|inner| {
                        Ok(inner
                            .session
                            .as_ref()
                            .map_or(0, |session| session.ring.current_epoch()))
                    })?;
                    match keys.current_epoch.cmp(&local) {
                        std::cmp::Ordering::Greater => report.newer_epoch = true,
                        std::cmp::Ordering::Less => return Err(refused("rolled_back")),
                        std::cmp::Ordering::Equal => {
                            // Re-sealed under the current epoch by the next
                            // batch; once per cycle.
                            stale_rounds += 1;
                            retry = true;
                        }
                    }
                }
                if effects.repush && !batch.more {
                    repush_rounds += 1;
                }
                if !(effects.repush || batch.more || retry)
                    || effects.newer_epoch
                    || report.newer_epoch
                    || repush_rounds >= MAX_PUSH_ROUNDS
                {
                    break;
                }
            }
        }
        // The local `recovery` row follows the synced `recovery_key` record
        // (section 4.4): built when missing, rebuilt on a new revision.
        if let Err(error) = self.ensure_recovery_row(&changed) {
            tracing::warn!(code = error.code(), "recovery row not rebuilt");
        }
        if !changed.is_empty() {
            changed.sort_unstable();
            changed.dedup();
            changed_classes.sort_unstable();
            changed_classes.dedup();
            let mut guard = self.inner();
            self.emit(
                &mut guard,
                CoreEventKind::RecordsChanged {
                    vault_id: link.vault_id,
                    record_ids: changed,
                    classes: changed_classes,
                },
            );
        }
        Ok(report)
    }

    fn ensure_recovery_row(&self, changed: &[Uuid]) -> Result<(), CoreError> {
        self.with_session(|inner| {
            let session = inner.session.as_mut().ok_or(CoreError::Locked)?;
            let Some(record_id) = session
                .index
                .singleton(RecordKind::RecoveryKey)
                .map(|record| record.record_id)
            else {
                return Ok(());
            };
            let vault_id = session.ring.vault_id();
            let epoch = session.ring.current_epoch();
            let missing = inner
                .store
                .key_wrap(&vault_id, epoch, WrapKind::Recovery)?
                .is_none();
            if !missing && !changed.contains(&record_id) {
                return Ok(());
            }
            let recovery_key = lock_ops::recovery_key_of(session)?;
            let recovery_kek = crypto::derive_recovery_kek(&recovery_key);
            let row = vault::wrap_row(
                WrapKind::Recovery,
                vault_id,
                epoch,
                &recovery_kek,
                session.ring.current_key(),
            )?;
            inner.store.write(|tx| tx.key_wrap_put(&row))?;
            Ok(())
        })
    }

    /// Section 4.4: opens the server's `recovery_wrap` with the key of the
    /// synced `recovery_key` record and compares it with the personal vault
    /// key. Runs in every cycle until the check passes, and again whenever
    /// the record gets a new revision (a reissue on any device) or the
    /// vault a new epoch; a device without the record yet waits for it
    /// (run 7 finding 17).
    fn check_recovery_wrap(&self, link: &Link) -> Result<(), CoreError> {
        let mark = self.with_session(|inner| {
            let session = inner.session.as_ref().ok_or(CoreError::Locked)?;
            let Some(record) = session.index.singleton(RecordKind::RecoveryKey) else {
                return Ok(None);
            };
            let mark = (
                record.record_id,
                record.revision,
                session.ring.current_epoch(),
            );
            Ok((inner.sync.recovery_checked != Some(mark)).then_some(mark))
        })?;
        let Some(mark) = mark else {
            return Ok(());
        };
        let account = self.authed(link, |api, token| api.account(token))?;
        self.with_session(|inner| {
            let session = inner.session.as_mut().ok_or(CoreError::Locked)?;
            let vault_id = session.ring.vault_id();
            let epoch = session.ring.current_epoch();
            let current = session
                .index
                .singleton(RecordKind::RecoveryKey)
                .map(|record| (record.record_id, record.revision, epoch));
            if account.personal_vault.vault_id != vault_id {
                // Another personal vault: not this device's wrap at all.
                inner.sync.recovery_mismatch = true;
                tracing::warn!("the server names another personal vault");
                return Ok(());
            }
            if current != Some(mark) || account.personal_vault.epoch != epoch {
                // Changed meanwhile, or an epoch this device does not hold
                // yet: the next cycle checks again.
                return Ok(());
            }
            let recovery_key = lock_ops::recovery_key_of(session)?;
            let recovery_kek = crypto::derive_recovery_kek(&recovery_key);
            let matches = open_account_wrap(
                vault_id,
                epoch,
                account.personal_vault.recovery_wrap.as_bytes(),
                &recovery_kek,
                WrapKind::Recovery,
            )
            .is_some_and(|key| key.ct_eq(session.ring.current_key()));
            inner.sync.recovery_mismatch = !matches;
            if matches {
                inner.sync.recovery_checked = Some(mark);
            } else {
                tracing::warn!("the server's recovery wrap does not hold the vault key");
            }
            Ok(())
        })
    }

    fn set_cursor(&self, vault_id: Uuid, cursor: u64) -> Result<(), CoreError> {
        let mut guard = self.inner();
        let mut state = guard.store.sync_state(&vault_id)?;
        state.vault_id = vault_id;
        state.cursor = cursor;
        guard.store.write(|tx| tx.sync_state_put(&state))?;
        Ok(())
    }

    fn set_resync(&self, vault_id: Uuid, classes: &[u8]) -> Result<(), CoreError> {
        let mut guard = self.inner();
        let mut state = guard.store.sync_state(&vault_id)?;
        state.vault_id = vault_id;
        state.cursor = 0;
        state.resync_classes = classes.to_vec();
        guard.store.write(|tx| tx.sync_state_put(&state))?;
        Ok(())
    }

    /// The server restore (section 6.9): every page from 0 is read first,
    /// the synced revisions are set to the server's, and the pages are
    /// applied without the stale copies of rule 5; the push sends back what
    /// the server lost.
    fn restore(
        &self,
        link: &Link,
        report: &mut SyncReport,
        changed: &mut Vec<Uuid>,
        refuse_newer: bool,
    ) -> Result<(), CoreError> {
        let selection = selection(&self.inner().store);
        let mut pages = Vec::new();
        // What the server holds: only versions that opened (an unreadable
        // header proves nothing about the server's copy).
        let mut server: HashMap<Uuid, u64> = HashMap::new();
        let mut cursor = 0;
        let mut complete = false;
        for _ in 0..MAX_PULL_PAGES {
            let page = self.authed(link, |api, token| {
                api.pull(token, link.vault_id, cursor, None, Some(&selection.classes))
            })?;
            cursor = page.next_cursor;
            let more = page.has_more;
            pages.push(page);
            if !more {
                complete = true;
                break;
            }
        }
        if !complete {
            // A snapshot cut off by the page limit is not a snapshot.
            return Err(refused("incomplete_snapshot"));
        }
        self.with_session(|inner| {
            let session = inner.session.as_mut().ok_or(CoreError::Locked)?;
            // A backup from before a rotation: the epochs come first (the
            // re-post of section 5.1, only with the user's confirmation).
            if pages
                .iter()
                .any(|page| page.current_epoch < session.ring.current_epoch())
            {
                return Err(refused("rolled_back"));
            }
            // Step 1 first: the snapshot is applied against the synced
            // revisions held now (pending edits meet the server's versions
            // through the conflict rules); step 3 uses the same revisions.
            let held: HashMap<Uuid, u64> = inner
                .store
                .records(&link.vault_id, Some(&selection.classes))?
                .into_iter()
                .map(|row| (row.record_id, row.synced_revision))
                .collect();
            let mut state = inner.store.sync_state(&link.vault_id)?;
            state.vault_id = link.vault_id;
            state.cursor = 0;
            for page in &pages {
                let result = engine::apply_page(
                    &mut inner.store,
                    session,
                    &page.records,
                    page.next_cursor,
                    &state,
                    &selection,
                    &held,
                    refuse_newer && page.current_epoch <= session.ring.current_epoch(),
                    self.now(),
                )?;
                report.integrity_warnings += result.effects.integrity_warnings;
                report.conflicts += result.effects.conflicts;
                changed.extend(result.effects.changed.iter().copied());
                for (id, revision) in &result.received {
                    let entry = server.entry(*id).or_insert(0);
                    *entry = (*entry).max(*revision);
                }
                if result.stopped && page.current_epoch <= session.ring.current_epoch() {
                    // An epoch the server does not hold (D183).
                    return Err(refused("forged_epoch"));
                }
                if result.stopped {
                    // A newer epoch: the device signs in again first; the
                    // cursor stays before the record.
                    report.newer_epoch = true;
                    return Err(refused("newer_epoch"));
                }
                state.cursor = page.next_cursor;
            }
            // Step 2: the synced revisions become the server's (or 0).
            engine::restore_mark(&mut inner.store, session, &server, &selection.classes)?;
            Ok(())
        })?;
        report.restored = true;
        tracing::info!("server restore handled");
        Ok(())
    }

    // -----------------------------------------------------------------
    // Status, sign-out, account and devices
    // -----------------------------------------------------------------

    /// The state for Settings → Account & Sync (no network).
    pub fn sync_status(&self) -> Result<SyncStatus, CoreError> {
        let mut guard = self.inner();
        let inner = &mut *guard;
        let account = inner.store.server_account()?;
        let mut status = SyncStatus {
            integrity_warnings: inner.sync.integrity_warnings,
            recovery_mismatch: inner.sync.recovery_mismatch,
            newer_epoch: inner.sync.newer_epoch,
            failed: inner.sync.failed.len(),
            ..SyncStatus::default()
        };
        if let Some(account) = &account {
            status.bound = true;
            status.signed_in = account.refresh_token.is_some();
            status.server_url = Some(account.server_url.clone());
            status.email = Some(account.email.clone());
            status.user_id = Some(account.user_id);
        }
        let Some(session) = inner.session.as_mut() else {
            return Ok(status);
        };
        let vault_id = session.ring.vault_id();
        let state = inner.store.sync_state(&vault_id)?;
        status.last_sync_at = state.last_sync_at;
        status.last_error = state.last_error;
        if account.is_some() {
            let sel = selection(&inner.store);
            status.pending = inner
                .store
                .records_pending(&vault_id)?
                .iter()
                .filter(|row| sel.class(row.class) && !(row.tombstone && row.synced_revision == 0))
                .count();
        }
        status.recently_deleted = inner
            .store
            .recently_deleted()?
            .iter()
            .filter(|row| row.vault_id == vault_id)
            .count();
        status.brought_back = inner
            .store
            .resync_returned()?
            .iter()
            .filter(|(vault, _, _)| *vault == vault_id)
            .count();
        status.duplicates = live_pairs(&inner.store, session)?.len();
        status.conflicts = conflict_rows(&inner.store, session)?.len();
        Ok(status)
    }

    /// Log out (section 5.7): the refresh token is revoked and the account
    /// row removed; the local copy stays and opens with the same password.
    /// With `delete_local` the data file is archived and the app returns to
    /// the first run (UI 12.2).
    pub fn sync_sign_out(&self, delete_local: bool) -> Result<(), CoreError> {
        let _sync = self.sync_lock();
        if delete_local {
            // Nothing may open the copy set aside below without the master
            // password (D190): the device key and its `device` row go first,
            // and when the OS keychain refuses, nothing has changed yet.
            let remembered = {
                let guard = self.inner();
                Self::require_session(&guard)?;
                super::device_row_exists(&guard)
            };
            if remembered {
                self.remember_device_set_impl(false)?;
            }
        }
        if let Ok(link) = self.link() {
            let token = {
                let guard = self.inner();
                let session = Self::require_session(&guard)?;
                guard
                    .store
                    .server_account()?
                    .and_then(|account| open_refresh(&session.ring, &link.device_id, &account))
            };
            if let Some(token) = token
                && let Err(error) = link.api.revoke(&token)
            {
                tracing::warn!(code = %error.code(), "refresh token not revoked");
            }
        }
        {
            let mut guard = self.inner();
            let inner = &mut *guard;
            inner.sync = SyncRuntime::default();
            if let Some(vault_id) = inner.personal_vault_id {
                let vault = inner.store.vault(&vault_id)?;
                inner.store.write(|tx| {
                    tx.server_account_delete()?;
                    if let Some(mut vault) = vault {
                        vault.bound = false;
                        tx.vault_put(&vault)?;
                    }
                    Ok(())
                })?;
            }
            tracing::info!("signed out");
            self.emit_sync(inner);
        }
        if delete_local {
            // The local copy is set aside as "Start with a new vault" does
            // (D190): renamed next to the new store, never deleted. One step
            // from Unlocked: a failure keeps the vault open, signed out.
            self.set_aside_store(true)?;
        }
        Ok(())
    }

    /// The current password's auth key for a sensitive operation (4.8).
    fn current_keys(&self, password: &SecretString) -> Result<(Link, Derived), CoreError> {
        let link = self.link()?;
        let (context, origin) = {
            let guard = self.inner();
            let account = guard.store.server_account()?.ok_or(CoreError::WrongState)?;
            (lock_ops::key_context(&guard.store)?, account.server_url)
        };
        let material = PasswordMaterial::from_row(&context.local_auth)?;
        let email = material.kdf_email.clone().unwrap_or_default();
        let derived = derive(
            &material.kdf,
            &material.kdf_salt,
            &email,
            password,
            &origin,
            self.platform.class,
        )?;
        vault::open_wrap(&context.kek_row, &derived.kek).map_err(|_| CoreError::InvalidPassword)?;
        Ok((link, derived))
    }

    /// Deletes the account on the server (section 5.8); the local copy stays.
    pub fn sync_account_delete(
        &self,
        password: SecretString,
        totp_code: Option<String>,
    ) -> Result<(), CoreError> {
        let _sync = self.sync_lock();
        let (link, keys) = self.current_keys(&password)?;
        let auth_key = keys.auth_key;
        drop(password);
        let request = wire::AccountDeleteRequest {
            current_auth_key: key_b64(&auth_key),
            totp_code: totp_code.as_deref().map(|code| secret(code.trim())),
        };
        self.authed(&link, |api, token| api.account_delete(token, &request))?;
        self.account_deleted_locally()
    }

    /// The account's devices (UI 12.2).
    pub fn sync_devices(&self) -> Result<Vec<DeviceItem>, CoreError> {
        // One refresh at a time (a parallel refresh would look like reuse).
        let _sync = self.sync_lock();
        let link = self.link()?;
        let list = self.authed(&link, |api, token| api.devices(token))?;
        Ok(list
            .devices
            .into_iter()
            .map(|device| DeviceItem {
                device_id: device.device_id,
                created_at: device.created_at,
                last_seen_at: device.last_seen_at,
                revoked: device.revoked,
                current: device.current,
            })
            .collect())
    }

    /// "Sign out this device" / "Lost or stolen" (section 4.7).
    pub fn sync_device_revoke(&self, device_id: Uuid) -> Result<(), CoreError> {
        let _sync = self.sync_lock();
        let link = self.link()?;
        self.authed(&link, |api, token| api.device_revoke(token, device_id))
    }

    /// TOTP setup (section 5.5): the secret and the `otpauth://` text.
    pub fn sync_totp_setup(&self, password: SecretString) -> Result<TotpSetup, CoreError> {
        let _sync = self.sync_lock();
        let (link, keys) = self.current_keys(&password)?;
        let auth_key = keys.auth_key;
        drop(password);
        let request = wire::CurrentAuth {
            current_auth_key: key_b64(&auth_key),
        };
        let response = self.authed(&link, |api, token| api.totp_setup(token, &request))?;
        // The secret only when it is one; the QR code's URI is made here
        // (D188): the server's `otpauth_uri` is never shown.
        let Some(checked) = super::sync_trust::checked_totp_secret(&response.secret.0) else {
            tracing::warn!("the server's TOTP secret is not base32");
            return Err(refused("totp_secret"));
        };
        let email = self
            .inner()
            .store
            .server_account()?
            .map(|account| account.email)
            .unwrap_or_default();
        let uri = super::sync_trust::otpauth_uri(&checked, &email);
        drop(response);
        Ok(TotpSetup {
            secret: SecretString::from(checked),
            otpauth_uri: Some(SecretString::from(uri)),
        })
    }

    pub fn sync_totp_enable(&self, code_text: &str) -> Result<(), CoreError> {
        let _sync = self.sync_lock();
        let link = self.link()?;
        let request = wire::TotpCode {
            code: secret(code_text.trim()),
        };
        self.authed(&link, |api, token| api.totp_enable(token, &request))
    }

    pub fn sync_totp_disable(
        &self,
        password: SecretString,
        code_text: &str,
    ) -> Result<(), CoreError> {
        let _sync = self.sync_lock();
        let (link, keys) = self.current_keys(&password)?;
        let auth_key = keys.auth_key;
        drop(password);
        let request = wire::TotpDisableRequest {
            current_auth_key: key_b64(&auth_key),
            code: secret(code_text.trim()),
        };
        self.authed(&link, |api, token| api.totp_disable(token, &request))
    }

    /// The password change of a bound vault (section 5.1): the server takes
    /// the new verifier and `kek_wrap` first, then the local rows change.
    /// With `rotate` ("my old password may be compromised") the personal
    /// vault also gets a new epoch, written locally only after the server
    /// accepted it; records are re-sealed afterwards.
    pub(super) fn password_change_bound(
        &self,
        context: &KeyContext,
        current: SecretString,
        new: SecretString,
        rotate: bool,
    ) -> Result<(), CoreError> {
        let _sync = self.sync_lock();
        // A rotation rewraps the `device` row too ("remember on this device").
        let _remember = self.remember_lock();
        let device_wrap_key = if rotate {
            self.rotation_device_key(context)
        } else {
            None
        };
        let class = self.platform.class;
        let (link, current_keys) = self.current_keys(&current)?;
        drop(current);
        let origin = link.api.origin().to_owned();
        let email = context.local_auth.kdf_email.clone().unwrap_or_default();
        let material = PasswordMaterial::fresh(class, Some(email.clone()))?;
        let derived = derive(
            &material.kdf,
            &material.kdf_salt,
            &email,
            &new,
            &origin,
            class,
        )?;
        drop(new);
        let (kek_row, rotation, vault_id) = self.with_session(|inner| {
            if !lock_ops::key_rows_unchanged(&inner.store, context)? {
                return Err(CoreError::WrongState);
            }
            let session = inner.session.as_ref().ok_or(CoreError::Locked)?;
            let vault_id = session.ring.vault_id();
            if rotate {
                let recovery_key = lock_ops::recovery_key_of(session)?;
                let recovery_kek = crypto::derive_recovery_kek(&recovery_key);
                let rotation = Rotation::prepare(
                    &session.ring,
                    &context.vault,
                    &derived.kek,
                    &recovery_kek,
                    device_wrap_key.as_ref(),
                )?;
                let kek_row = rotation
                    .row(WrapKind::Kek)
                    .cloned()
                    .ok_or(CoreError::Internal("rotation"))?;
                return Ok((kek_row, Some(rotation), vault_id));
            }
            let row = vault::wrap_row(
                WrapKind::Kek,
                vault_id,
                session.ring.current_epoch(),
                &derived.kek,
                session.ring.current_key(),
            )?;
            Ok((row, None, vault_id))
        })?;
        let request = wire::PasswordChangeRequest {
            current_auth_key: key_b64(&current_keys.auth_key),
            kdf: wire_kdf(&material.kdf),
            kdf_salt: B64(material.kdf_salt.to_vec()),
            auth_key: key_b64(&derived.auth_key),
            kek_wrap: B64(kek_row.wrap.clone()),
            rotation: rotation.as_ref().map(wire_rotation).transpose()?,
        };
        let tokens = self.authed(&link, |api, token| api.password_change(token, &request))?;
        let now = self.now();
        let local_auth = material.to_row(vault_id, format_timestamp(now));
        self.with_session(|inner| {
            let ids = Self::commit_new_keys(
                inner,
                &link.device_id,
                rotation.as_ref(),
                Some(&kek_row),
                Some(&local_auth),
                None,
                &tokens,
                now,
            )?;
            tracing::info!(
                rotated = rotate,
                "master password changed on the server and here"
            );
            self.records_rewritten(inner, ids);
            Ok(())
        })
    }

    /// A password change with a personal vault rotation: through the server
    /// when bound, locally otherwise (section 5.1, "In local mode").
    pub(super) fn password_change_rotating_impl(
        &self,
        current: SecretString,
        new: SecretString,
    ) -> Result<(), CoreError> {
        let context = {
            let guard = self.inner();
            Self::require_session(&guard)?;
            lock_ops::key_context(&guard.store)?
        };
        crate::lock::check_new_password(&new, "new_password")?;
        if self.bound()? {
            return self.password_change_bound(&context, current, new, true);
        }
        let _remember = self.remember_lock();
        let device_wrap_key = self.rotation_device_key(&context);
        let class = self.platform.class;
        let old = PasswordMaterial::from_row(&context.local_auth)?;
        let old_kek = old.derive_kek(&current, class)?;
        vault::open_wrap(&context.kek_row, &old_kek).map_err(|_| CoreError::InvalidPassword)?;
        drop(current);
        let material = PasswordMaterial::fresh(class, context.local_auth.kdf_email.clone())?;
        let new_kek = material.derive_kek(&new, class)?;
        drop(new);
        let now = self.now();
        self.with_session(|inner| {
            if !lock_ops::key_rows_unchanged(&inner.store, &context)? {
                return Err(CoreError::WrongState);
            }
            let Inner { store, session, .. } = &mut *inner;
            let session = session.as_mut().ok_or(CoreError::Locked)?;
            let recovery_key = lock_ops::recovery_key_of(session)?;
            let recovery_kek = crypto::derive_recovery_kek(&recovery_key);
            let rotation = Rotation::prepare(
                &session.ring,
                &context.vault,
                &new_kek,
                &recovery_kek,
                device_wrap_key.as_ref(),
            )?;
            let vault_id = session.ring.vault_id();
            let local_auth = material.to_row(vault_id, format_timestamp(now));
            session
                .ring
                .push_epoch(rotation.new_epoch, rotation.new_key());
            if let Err(error) = store.write(|tx| {
                rotation.write(tx)?;
                tx.local_auth_put(&local_auth)
            }) {
                session.ring.pop_epoch(rotation.old_epoch);
                return Err(error.into());
            }
            let ids = lock_ops::reseal_or_defer(session, store, now);
            super::checkpoint(store);
            tracing::info!(
                epoch = rotation.new_epoch,
                "master password changed; vault rotated"
            );
            self.records_rewritten(inner, ids);
            Ok(())
        })
    }

    /// The device wrap key for a rotation while "remember on this device" is
    /// on (the keychain is read without the core mutex; `remember` is held).
    fn rotation_device_key(&self, context: &KeyContext) -> Option<Key32> {
        context.device_row.as_ref()?;
        self.fetch_device_key(&context.db_id)
            .map(|key| crypto::derive_device_wrap_key(&key, &context.db_id))
    }

    /// Writes the key rows of a server-accepted change in one transaction:
    /// the rotation (or a single `kek` row), `local_auth`, an extra record
    /// row and the new refresh token; then re-seals older records.
    #[allow(clippy::too_many_arguments)]
    fn commit_new_keys(
        inner: &mut Inner,
        device_id: &Uuid,
        rotation: Option<&Rotation>,
        kek_row: Option<&KeyWrapRow>,
        local_auth: Option<&LocalAuthRow>,
        record: Option<super::session::PendingWrite>,
        tokens: &wire::Tokens,
        now: SystemTime,
    ) -> Result<Vec<Uuid>, CoreError> {
        let Inner {
            store,
            session,
            sync,
            ..
        } = inner;
        let session = session.as_mut().ok_or(CoreError::Locked)?;
        if let Some(rotation) = rotation {
            session
                .ring
                .push_epoch(rotation.new_epoch, rotation.new_key());
        }
        let account = store.server_account()?;
        let origin = account
            .as_ref()
            .map(|row| row.server_url.clone())
            .unwrap_or_default();
        let (blob, epoch) =
            wrap_refresh(&session.ring, device_id, &origin, &tokens.refresh_token.0)?;
        let written = store.write(|tx| {
            if let Some(rotation) = rotation {
                rotation.write(tx)?;
            } else if let Some(row) = kek_row {
                tx.key_wrap_put(row)?;
            }
            if let Some(local_auth) = local_auth {
                tx.local_auth_put(local_auth)?;
            }
            if let Some(write) = &record {
                tx.record_put(&write.row)?;
            }
            if let Some(mut account) = account {
                account.refresh_token = Some(blob);
                account.secrets_epoch = epoch;
                tx.server_account_put(&account)?;
            }
            Ok(())
        });
        if let Err(error) = written {
            if let Some(rotation) = rotation {
                session.ring.pop_epoch(rotation.old_epoch);
            }
            return Err(error.into());
        }
        sync.hold_access(Zeroizing::new(tokens.access_token.0.clone()), &origin);
        let mut ids = Vec::new();
        if let Some(write) = record {
            ids.push(write.record_id);
            session.commit(vec![write]);
        }
        if rotation.is_some() {
            ids.extend(lock_ops::reseal_or_defer(session, store, now));
        }
        super::checkpoint(store);
        Ok(ids)
    }

    fn records_rewritten(&self, inner: &mut Inner, ids: Vec<Uuid>) {
        if ids.is_empty() {
            return;
        }
        let Some(vault_id) = inner.personal_vault_id else {
            return;
        };
        self.emit(
            inner,
            CoreEventKind::RecordsChanged {
                vault_id,
                record_ids: ids,
                classes: classes::ALL.to_vec(),
            },
        );
    }

    /// The recovery key reissue of a bound vault (section 5.3): a new
    /// recovery key and a new personal epoch, committed by the server
    /// together with the new `recovery_key` record; written here after.
    pub(super) fn recovery_key_reissue_bound(
        &self,
        password: SecretString,
    ) -> Result<crate::service::dto::RecoveryKeyResult, CoreError> {
        let _sync = self.sync_lock();
        let _remember = self.remember_lock();
        let (link, keys) = self.current_keys(&password)?;
        drop(password);
        let context = lock_ops::key_context(&self.inner().store)?;
        let origin = link.api.origin().to_owned();
        let new_recovery_key = Key32::generate()?;
        let device_wrap_key = self.rotation_device_key(&context);
        let recovery_kek = crypto::derive_recovery_kek(&new_recovery_key);
        let recovery_auth_key = crypto::derive_recovery_auth_key(&new_recovery_key, &origin);
        let now = self.now();
        let (rotation, write) = self.with_session(|inner| {
            if !lock_ops::key_rows_unchanged(&inner.store, &context)? {
                return Err(CoreError::WrongState);
            }
            let session = inner.session.as_mut().ok_or(CoreError::Locked)?;
            let record_id = session
                .index
                .singleton(RecordKind::RecoveryKey)
                .map(|record| record.record_id)
                .ok_or(CoreError::Damaged)?;
            let rotation = Rotation::prepare(
                &session.ring,
                &context.vault,
                &keys.kek,
                &recovery_kek,
                device_wrap_key.as_ref(),
            )?;
            session
                .ring
                .push_epoch(rotation.new_epoch, rotation.new_key());
            let data = lock_ops::recovery_record_data(&new_recovery_key, &format_timestamp(now));
            let write =
                session.prepare_write(&inner.store, RecordKind::RecoveryKey, record_id, data, now);
            session.ring.pop_epoch(rotation.old_epoch);
            Ok((rotation, write?))
        })?;
        let request = wire::RecoveryKeyReissueRequest {
            current_auth_key: key_b64(&keys.auth_key),
            recovery_auth_key: key_b64(&recovery_auth_key),
            kek_wrap: B64(rotation
                .row(WrapKind::Kek)
                .ok_or(CoreError::Internal("rotation"))?
                .wrap
                .clone()),
            rotation: wire_rotation(&rotation)?,
            record: wire::RecordWrite {
                base_revision: write.row.synced_revision,
                envelope: B64(write.row.envelope.clone()),
            },
        };
        let response = self.authed(&link, |api, token| {
            api.recovery_key_reissue(token, &request)
        })?;
        let mut write = write;
        write.row.synced_revision = write.row.revision;
        write.row.base_envelope = None;
        write.row.server_seq = Some(response.seq as i64);
        self.with_session(|inner| {
            let ids = Self::commit_new_keys(
                inner,
                &link.device_id,
                Some(&rotation),
                None,
                None,
                Some(write),
                &response.tokens,
                now,
            )?;
            inner.recovery_grant.grant(now);
            tracing::info!(
                epoch = rotation.new_epoch,
                "recovery key reissued on the server"
            );
            self.records_rewritten(inner, ids);
            Ok(())
        })?;
        Ok(crate::service::dto::RecoveryKeyResult {
            recovery_key: crypto::encode_recovery_key(&new_recovery_key),
        })
    }

    /// "Bring the server up to date…" (section 5.1, re-post; D93): after
    /// the user confirmed and re-entered the account password, every
    /// personal epoch the server lacks is posted with the keys this device
    /// holds, then the server restore procedure pushes the records back.
    pub fn sync_repost_epochs(&self, password: SecretString) -> Result<SyncReport, CoreError> {
        let _sync = self.sync_lock();
        let (link, keys) = self.current_keys(&password)?;
        drop(password);
        let vaults = self.authed(&link, |api, token| api.vaults(token))?;
        let server_epoch = vaults
            .vaults
            .iter()
            .find(|vault| vault.vault_id == link.vault_id)
            .map(|vault| vault.current_epoch)
            .ok_or(HttpError::Malformed)?;
        let steps = self.with_session(|inner| {
            let session = inner.session.as_mut().ok_or(CoreError::Locked)?;
            let local = session.ring.current_epoch();
            let recovery_key = lock_ops::recovery_key_of(session)?;
            let recovery_kek = crypto::derive_recovery_kek(&recovery_key);
            let mut steps = Vec::new();
            for epoch in server_epoch + 1..=local {
                let chain = inner
                    .store
                    .key_wrap(&link.vault_id, epoch, WrapKind::Chain)?
                    .ok_or(CoreError::Damaged)?;
                let key = session.ring.key(epoch).ok_or(CoreError::Damaged)?;
                let kek_row = vault::wrap_row(WrapKind::Kek, link.vault_id, epoch, &keys.kek, key)?;
                let recovery_row =
                    vault::wrap_row(WrapKind::Recovery, link.vault_id, epoch, &recovery_kek, key)?;
                steps.push(wire::PersonalKeyEpochRequest {
                    current_auth_key: key_b64(&keys.auth_key),
                    epoch,
                    prev_key_wrap: B64(chain.wrap),
                    account_wraps: wire::AccountWraps {
                        kek_wrap: B64(kek_row.wrap),
                        recovery_wrap: B64(recovery_row.wrap),
                    },
                });
            }
            Ok(steps)
        })?;
        for step in &steps {
            self.authed(&link, |api, token| {
                api.personal_key_epoch(token, link.vault_id, step)
            })?;
        }
        let mut report = SyncReport::default();
        let mut changed = Vec::new();
        self.restore(&link, &mut report, &mut changed, false)?;
        let pushed = self.cycle(false)?;
        report.pushed = pushed.pushed;
        tracing::info!(epochs = steps.len(), "personal epochs re-posted");
        Ok(report)
    }

    // -----------------------------------------------------------------
    // Recovery through the server (section 5.2; UI 13.2 "Restore from sync")
    // -----------------------------------------------------------------

    /// "Forgot password? → Restore from sync" on a bound, locked device:
    /// the recovery key opens the local `recovery` row, the server checks
    /// it (`recover/begin`), the new password and a new personal epoch go to
    /// the server (`recover/complete`), then the device unlocks with them.
    /// Other devices are signed out. A server that cannot be reached or
    /// refuses leaves everything as it was: the interface then offers the
    /// local recovery (`recovery_unlock`).
    pub fn recovery_unlock_server(
        &self,
        recovery_key: SecretString,
        new_password: SecretString,
        email: &str,
        totp_code: Option<String>,
    ) -> Result<crate::service::dto::LockStatus, CoreError> {
        let _sync = self.sync_lock();
        let available = self.remember_available();
        let _remember = self.remember_lock();
        if !crypto::password_length_ok(&new_password) {
            return Err(CoreError::Validation(vec![crate::error::FieldError::new(
                "new_password",
                crate::error::ValidationReason::TooShort,
            )]));
        }
        let decoded = crypto::decode_recovery_key(recovery_key.expose_secret())
            .map_err(|_| CoreError::InvalidRecoveryKey)?;
        drop(recovery_key);
        let (context, account, api) = {
            let guard = self.inner();
            if guard.state != LockState::Locked {
                return Err(CoreError::WrongState);
            }
            let account = guard.store.server_account()?.ok_or(CoreError::WrongState)?;
            let api = self.api(&guard.store, &account.server_url)?;
            (lock_ops::key_context(&guard.store)?, account, api)
        };
        let recovery_kek = crypto::derive_recovery_kek(&decoded);
        let recovery_row = context
            .recovery_row
            .as_ref()
            .ok_or(CoreError::InvalidRecoveryKey)?;
        let vault_key = vault::open_wrap(recovery_row, &recovery_kek)
            .map_err(|_| CoreError::InvalidRecoveryKey)?;
        let origin = account.server_url.clone();
        let email = crypto::normalize_email(email);
        let begun = api.recover_begin(&wire::RecoverBeginRequest {
            email: email.clone(),
            recovery_auth_key: key_b64(&crypto::derive_recovery_auth_key(&decoded, &origin)),
            device_id: context.device_id,
        })?;
        if crypto::normalize_email(&begun.email) != email {
            return Err(refused("email_mismatch"));
        }
        // A rolled-back or foreign server state changes nothing (section
        // 5.1): the same epoch and the same key, or no server recovery.
        let epoch = context.vault.current_epoch;
        if begun.personal_vault_id != context.vault.vault_id || begun.epoch != epoch {
            return Err(refused("rolled_back"));
        }
        let held = open_account_wrap(
            context.vault.vault_id,
            epoch,
            begun.recovery_wrap.as_bytes(),
            &recovery_kek,
            WrapKind::Recovery,
        );
        if !held.is_some_and(|key| key.ct_eq(&vault_key)) {
            return Err(refused("rolled_back"));
        }
        let class = self.platform.class;
        let material = PasswordMaterial::fresh(class, Some(email.clone()))?;
        let derived = derive(
            &material.kdf,
            &material.kdf_salt,
            &email,
            &new_password,
            &origin,
            class,
        )?;
        drop(new_password);
        let mut ring = KeyRing::new(context.vault.vault_id, epoch, vault_key);
        {
            let guard = self.inner();
            ring.open_chain(&guard.store)?;
        }
        let device_wrap_key = self.rotation_device_key(&context);
        let rotation = Rotation::prepare(
            &ring,
            &context.vault,
            &derived.kek,
            &recovery_kek,
            device_wrap_key.as_ref(),
        )?;
        let tokens = api.recover_complete(&wire::RecoverCompleteRequest {
            recovery_ticket: begun.recovery_ticket,
            totp_code: totp_code.as_deref().map(|code| secret(code.trim())),
            email: email.clone(),
            kdf: wire_kdf(&material.kdf),
            kdf_salt: B64(material.kdf_salt.to_vec()),
            auth_key: key_b64(&derived.auth_key),
            kek_wrap: B64(rotation
                .row(WrapKind::Kek)
                .ok_or(CoreError::Internal("rotation"))?
                .wrap
                .clone()),
            rotation: wire_rotation(&rotation)?,
            device_id: context.device_id,
        })?;
        // Committed on the server: the local rows follow in one transaction.
        let now = self.now();
        let mut guard = self.inner();
        let inner = &mut *guard;
        if inner.state != LockState::Locked
            || !lock_ops::key_rows_unchanged(&inner.store, &context)?
        {
            return Err(CoreError::WrongState);
        }
        ring.push_epoch(rotation.new_epoch, rotation.new_key());
        let (blob, secrets_epoch) = wrap_refresh(
            &ring,
            &context.device_id,
            &account.server_url,
            &tokens.refresh_token.0,
        )?;
        let mut account_row = account;
        account_row.email = email;
        account_row.refresh_token = Some(blob);
        account_row.secrets_epoch = secrets_epoch;
        let local_auth = material.to_row(context.vault.vault_id, format_timestamp(now));
        inner.store.write(|tx| {
            rotation.write(tx)?;
            tx.local_auth_put(&local_auth)?;
            tx.server_account_put(&account_row)
        })?;
        inner.sync.hold_access(
            Zeroizing::new(tokens.access_token.0.clone()),
            &account_row.server_url,
        );
        tracing::info!(
            epoch = rotation.new_epoch,
            "recovered through the server; vault rotated"
        );
        drop(ring);
        let started = match lock_ops::key_context(&inner.store) {
            Ok(new_context) => self.start_session(inner, &new_context, rotation.new_key(), now),
            Err(error) => Err(error),
        };
        if let Err(error) = started {
            tracing::warn!(code = error.code(), "unlock after recovery failed");
        }
        super::checkpoint(&inner.store);
        Ok(self.status(inner, available))
    }

    /// `true` while the personal vault is bound to an account.
    pub(super) fn bound(&self) -> Result<bool, CoreError> {
        Ok(self.inner().store.server_account()?.is_some())
    }

    // -----------------------------------------------------------------
    // Recently deleted (UI 14)
    // -----------------------------------------------------------------

    /// The entries of the last 90 days, newest first. Older ones are purged
    /// first, so they leave in local mode too, not only in a sync cycle
    /// (UI 14: at the start and every 24 hours, when the screen lists).
    pub fn recently_deleted_list(&self) -> Result<Vec<DeletedItem>, CoreError> {
        let cutoff = format_timestamp(self.now() - Duration::from_secs(trash::KEEP_DAYS * 86_400));
        self.with_session(|inner| {
            let session = inner.session.as_mut().ok_or(CoreError::Locked)?;
            let vault_id = session.ring.vault_id();
            inner.store.write(|tx| {
                tx.recently_deleted_purge(&cutoff)?;
                Ok(())
            })?;
            let mut items = Vec::new();
            for row in inner.store.recently_deleted()? {
                if row.vault_id != vault_id {
                    continue;
                }
                let Some(payload) = open_envelope_payload(&mut session.ring, &row.envelope) else {
                    continue;
                };
                items.push(DeletedItem {
                    record_id: row.record_id,
                    vault_id: row.vault_id,
                    title: title_of(&payload.type_name, payload.data.as_ref()),
                    type_name: payload.type_name.clone(),
                    deleted_at: row.deleted_at,
                    from_sync: row.origin == DeletedOrigin::Sync,
                    data: payload.data.as_ref().map(masked),
                });
            }
            Ok(items)
        })
    }

    /// Restores entries as the next revision of their records (a new id
    /// when the record row is gone).
    pub fn recently_deleted_restore(&self, record_ids: &[Uuid]) -> Result<Vec<Uuid>, CoreError> {
        let now = self.now();
        self.with_session(|inner| {
            let session = inner.session.as_mut().ok_or(CoreError::Locked)?;
            let vault_id = session.ring.vault_id();
            let entries: Vec<RecentlyDeletedRow> = inner
                .store
                .recently_deleted()?
                .into_iter()
                .filter(|row| row.vault_id == vault_id && record_ids.contains(&row.record_id))
                .collect();
            let mut writes = Vec::new();
            // Entries that end here: restored now, or live again already.
            let mut done = Vec::new();
            for entry in &entries {
                let Some(payload) = open_envelope_payload(&mut session.ring, &entry.envelope)
                else {
                    continue;
                };
                let (Some(kind), Some(data)) = (
                    RecordKind::from_name(&payload.type_name),
                    payload.data.clone(),
                ) else {
                    continue;
                };
                // A live record with this id (restored elsewhere) is kept.
                if session.index.get(&entry.record_id).is_some() {
                    done.push(entry.record_id);
                    continue;
                }
                done.push(entry.record_id);
                writes.push(session.prepare_write(
                    &inner.store,
                    kind,
                    entry.record_id,
                    data,
                    now,
                )?);
            }
            let restored: Vec<Uuid> = writes.iter().map(|write| write.record_id).collect();
            inner.store.write(|tx| {
                for write in &writes {
                    tx.record_put(&write.row)?;
                }
                // An entry that could not be restored stays (never lost here).
                for id in &done {
                    tx.recently_deleted_delete(&vault_id, id)?;
                }
                Ok(())
            })?;
            let classes: Vec<u8> = writes.iter().map(|write| write.row.class).collect();
            session.commit(writes);
            if !restored.is_empty() {
                self.emit(
                    inner,
                    CoreEventKind::RecordsChanged {
                        vault_id,
                        record_ids: restored.clone(),
                        classes,
                    },
                );
            }
            Ok(restored)
        })
    }

    /// "Delete forever".
    pub fn recently_deleted_delete(&self, record_ids: &[Uuid]) -> Result<(), CoreError> {
        self.with_session(|inner| {
            let vault_id = inner
                .session
                .as_ref()
                .map(|s| s.ring.vault_id())
                .ok_or(CoreError::Locked)?;
            inner.store.write(|tx| {
                for id in record_ids {
                    tx.recently_deleted_delete(&vault_id, id)?;
                }
                Ok(())
            })?;
            Ok(())
        })
    }

    /// "Empty".
    pub fn recently_deleted_empty(&self) -> Result<usize, CoreError> {
        self.with_session(|inner| Ok(inner.store.write(|tx| tx.recently_deleted_clear())?))
    }

    // -----------------------------------------------------------------
    // Conflicts (UI 13.6), duplicates (UI 13.7), brought back (6.9)
    // -----------------------------------------------------------------

    pub fn sync_conflicts(&self) -> Result<Vec<ConflictItem>, CoreError> {
        self.with_session(|inner| {
            let session = inner.session.as_mut().ok_or(CoreError::Locked)?;
            let vault_id = session.ring.vault_id();
            let rows = conflict_rows(&inner.store, session)?;
            let this_device = session.device_id.to_string();
            let device_names = device_names(session);
            // Records deleted here whose rows went at once (a vault not
            // bound yet): deleted, not missing.
            let trashed: HashSet<Uuid> = if rows.is_empty() {
                HashSet::new()
            } else {
                inner
                    .store
                    .recently_deleted()?
                    .into_iter()
                    .filter(|entry| entry.vault_id == vault_id)
                    .map(|entry| entry.record_id)
                    .collect()
            };
            let mut items = Vec::with_capacity(rows.len());
            for (row, payload) in rows {
                let data = payload.data.clone().unwrap_or_default();
                let text = |name: &str| data.get(name).and_then(Value::as_str).map(str::to_owned);
                let original_record_id = text("original_record_id")
                    .and_then(|id| Uuid::parse_str(&id).ok())
                    .unwrap_or_default();
                let original_type = text("original_type").unwrap_or_default();
                let losing_data = data.get("losing_data").and_then(Value::as_object).cloned();
                // The current version as stored: its device and time, and the
                // data of a lazy class the index has not opened yet.
                let stored_row = inner.store.record(&vault_id, &original_record_id)?;
                let stored = stored_row
                    .as_ref()
                    .and_then(|row| open_payload(&mut session.ring, row));
                let indexed = session
                    .index
                    .get(&original_record_id)
                    .map(|record| record.data().clone());
                // Missing (data-model 15): no row and no deletion here (a
                // purged tombstone), or a live row this device cannot open.
                let (current_deleted, current_missing) = match (&stored_row, &stored) {
                    (Some(_), Some(stored)) => (stored.deleted, false),
                    (Some(row), None) => (row.tombstone, !row.tombstone),
                    (None, _) => {
                        let deleted = trashed.contains(&original_record_id);
                        (deleted, !deleted)
                    }
                };
                let current_data = if current_deleted || current_missing {
                    None
                } else {
                    indexed.or_else(|| stored.as_ref().and_then(|stored| stored.data.clone()))
                };
                let current_device_id = stored.as_ref().map(|stored| stored.device_id.to_string());
                let losing_device_id = text("losing_device_id");
                let name_of =
                    |id: &Option<String>| id.as_ref().and_then(|id| device_names.get(id)).cloned();
                let losing_deleted = data
                    .get("losing_deleted")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let keep_other = keep_other_of(
                    &original_type,
                    unreadable(&mut session.ring, stored_row.as_ref()),
                    current_data.as_ref(),
                    losing_deleted,
                    losing_data.as_ref(),
                );
                items.push(ConflictItem {
                    conflict_id: row.record_id,
                    original_record_id,
                    title: current_data
                        .as_ref()
                        .map(|data| title_of(&original_type, Some(data)))
                        .unwrap_or_else(|| title_of(&original_type, losing_data.as_ref())),
                    original_type,
                    losing_deleted,
                    differences: differences(current_data.as_ref(), losing_data.as_ref()),
                    losing_data: losing_data.as_ref().map(masked),
                    current_deleted,
                    current_missing,
                    current_data: current_data.as_ref().map(masked),
                    keep_other,
                    current_device_name: name_of(&current_device_id),
                    losing_device_name: name_of(&losing_device_id),
                    current_this_device: current_device_id.as_deref() == Some(this_device.as_str()),
                    losing_this_device: losing_device_id.as_deref() == Some(this_device.as_str()),
                    current_changed_at: stored
                        .as_ref()
                        .map(|stored| format_unix_ms(stored.hlc.ms())),
                    losing_changed_at: text("losing_hlc")
                        .and_then(|hlc| Hlc::parse(&hlc))
                        .map(|hlc| format_unix_ms(hlc.ms())),
                    current_device_id,
                    losing_device_id,
                });
            }
            Ok(items)
        })
    }

    /// Test support (feature `test-hooks`): writes a conflict copy of a
    /// record as a sync merge does when the other version loses
    /// (data-model 15), so the comparison (UI 13.6) can be shown and tested
    /// without a server. The losing version is `losing` (`None`: a
    /// deletion), made on `device_id` at `changed_at`.
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn sync_conflict_insert_for_test(
        &self,
        record_id: Uuid,
        losing: Option<Map<String, Value>>,
        device_id: Uuid,
        changed_at: SystemTime,
    ) -> Result<Uuid, CoreError> {
        let now = self.now();
        let now_text = format_timestamp(now);
        self.with_session(|inner| {
            let session = inner.session.as_mut().ok_or(CoreError::Locked)?;
            let vault_id = session.ring.vault_id();
            let original = inner
                .store
                .record(&vault_id, &record_id)?
                .ok_or(CoreError::NotFound)?;
            let current = open_payload(&mut session.ring, &original).ok_or(CoreError::NotFound)?;
            let hlc = Hlc::new(unix_ms(changed_at), 0);
            let losing = match losing {
                Some(data) => Payload::live(&current.type_name, current.v, hlc, device_id, data),
                None => Payload::tombstone(&current.type_name, current.v, hlc, device_id),
            };
            let copy_id = Uuid::now_v7();
            let stamp = session.hlc.tick(unix_ms(now));
            let payload = Payload::live(
                "conflict",
                1,
                stamp,
                session.device_id,
                engine::conflict_data(&original, &losing),
            );
            let row = vault::seal_row(
                &mut session.ring,
                original.class,
                copy_id,
                &payload,
                None,
                &now_text,
            )?;
            inner.store.write(|tx| tx.record_put(&row))?;
            self.emit_sync(inner);
            Ok(copy_id)
        })
    }

    /// Resolves a conflict (UI 13.6): the copy is deleted; with
    /// [`ConflictChoice::Copy`] its version becomes the record's next one
    /// (a deletion tombstones the original). "Keep other" follows the type
    /// rules ([`KeepOther`]); refused, it keeps the conflict, as it does
    /// whenever the other version cannot be written.
    pub fn sync_conflict_resolve(
        &self,
        conflict_id: Uuid,
        choice: ConflictChoice,
    ) -> Result<(), CoreError> {
        // A deletion of a kind the user deletes runs first as the user's own
        // (Recently deleted, a group's children move up); the copy goes
        // after it, so a failed deletion keeps the conflict.
        for _ in 0..2 {
            match self
                .with_session(|inner| self.conflict_resolve_pass(inner, conflict_id, choice))?
            {
                None => return Ok(()),
                Some(delete) => {
                    self.record_delete(delete)?;
                }
            }
        }
        Err(CoreError::Internal("conflict deletion repeated"))
    }

    /// One pass of [`Core::sync_conflict_resolve`]: everything in one
    /// transaction, or the deletion to run first.
    fn conflict_resolve_pass(
        &self,
        inner: &mut Inner,
        conflict_id: Uuid,
        choice: ConflictChoice,
    ) -> Result<Option<super::dto::RecordsRef>, CoreError> {
        let now = self.now();
        let now_text = format_timestamp(now);
        let session = inner.session.as_mut().ok_or(CoreError::Locked)?;
        let vault_id = session.ring.vault_id();
        let row = inner
            .store
            .record(&vault_id, &conflict_id)?
            .ok_or(CoreError::NotFound)?;
        let payload = open_payload(&mut session.ring, &row).ok_or(CoreError::NotFound)?;
        if payload.type_name != "conflict" || payload.deleted {
            return Err(CoreError::NotFound);
        }
        let data = payload.data.clone().unwrap_or_default();
        let mut rows = Vec::new();
        let mut writes = Vec::new();
        let mut trashed = Vec::new();
        let mut hidden = Vec::new();
        if choice == ConflictChoice::Copy {
            let original = data
                .get("original_record_id")
                .and_then(Value::as_str)
                .and_then(|id| Uuid::parse_str(id).ok())
                .ok_or(CoreError::NotFound)?;
            let type_name = data
                .get("original_type")
                .and_then(Value::as_str)
                .unwrap_or_default();
            // A type this app cannot write keeps its conflict (UI 13.6).
            let kind = RecordKind::from_name(type_name).ok_or(CoreError::ReadOnly)?;
            // A history or log entry may not be loaded yet (D10).
            session.ensure_class(&inner.store, kind.class())?;
            let losing_deleted = data
                .get("losing_deleted")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let losing = data.get("losing_data").and_then(Value::as_object).cloned();
            let previous = inner
                .store
                .record(&vault_id, &original)?
                .filter(|previous| !previous.tombstone);
            let current = previous
                .as_ref()
                .and_then(|previous| open_payload(&mut session.ring, previous))
                .filter(|current| !current.deleted);
            let current_data = current.as_ref().and_then(|current| current.data.as_ref());
            let keep = keep_other_of(
                type_name,
                unreadable(&mut session.ring, previous.as_ref()),
                current_data,
                losing_deleted,
                losing.as_ref(),
            );
            if !keep.allowed() {
                return Err(CoreError::ReadOnly);
            }
            if losing_deleted {
                if let Some(previous) = previous {
                    if session.index.get(&original).is_none() {
                        // Live here but not readable (a newer payload
                        // version, damaged): never dropped for a copy.
                        return Err(CoreError::ReadOnly);
                    }
                    if super::records::deletable(kind) {
                        return Ok(Some(super::dto::RecordsRef {
                            vault_id,
                            record_ids: vec![original],
                        }));
                    }
                    // Known hosts, identities, history and log entries and the
                    // bookkeeping types: tombstoned with the copy.
                    if trash::kept(kind) {
                        trashed.push(RecentlyDeletedRow {
                            vault_id,
                            record_id: original,
                            class: previous.class,
                            deleted_at: now_text.clone(),
                            origin: DeletedOrigin::Local,
                            envelope: previous.envelope.clone(),
                        });
                    }
                    rows.push(session.prepare_tombstone(kind, &previous, now)?);
                    hidden.push(original);
                }
            } else {
                let mut losing = losing.ok_or(CoreError::NotFound)?;
                if keep == KeepOther::KeepsRevocation
                    && let Some(current) = current_data
                {
                    keep_revocation(current, &mut losing);
                }
                writes.push(session.prepare_write(&inner.store, kind, original, losing, now)?);
            }
        }
        let hlc = session.hlc.tick(unix_ms(now));
        let tombstone = Payload::tombstone("conflict", payload.v, hlc, session.device_id);
        rows.push(vault::seal_row(
            &mut session.ring,
            row.class,
            row.record_id,
            &tombstone,
            Some(&row),
            &now_text,
        )?);
        inner.store.write(|tx| {
            for entry in &trashed {
                tx.recently_deleted_put(entry)?;
            }
            for row in &rows {
                tx.record_put(row)?;
            }
            for write in &writes {
                tx.record_put(&write.row)?;
                // The record is live again: no stale Recently deleted entry.
                tx.recently_deleted_delete(&vault_id, &write.record_id)?;
            }
            Ok(())
        })?;
        let mut changed: Vec<Uuid> = writes.iter().map(|write| write.record_id).collect();
        for id in &hidden {
            session.index.remove(id);
            changed.push(*id);
        }
        changed.push(conflict_id);
        session.commit(writes);
        self.emit(
            inner,
            CoreEventKind::RecordsChanged {
                vault_id,
                record_ids: changed,
                classes: vec![row.class],
            },
        );
        Ok(None)
    }

    pub fn sync_duplicates(&self) -> Result<Vec<DuplicateItem>, CoreError> {
        self.with_session(|inner| {
            let session = inner.session.as_mut().ok_or(CoreError::Locked)?;
            let pairs = live_pairs(&inner.store, session)?;
            let devices = device_names(session);
            let this_device = session.device_id;
            let side = |record: &IndexedRecord| duplicate_side(record, &devices, this_device);
            Ok(pairs
                .into_iter()
                .filter_map(|(a, b)| {
                    let record = session.index.get(&a)?;
                    let other = session.index.get(&b)?;
                    // UUIDv7 ids sort by creation time: the smaller is older.
                    let (left, right) = if b < a {
                        (other, record)
                    } else {
                        (record, other)
                    };
                    Some(DuplicateItem {
                        record_id: a,
                        other_record_id: b,
                        type_name: record.kind.name().to_owned(),
                        title: title_of(record.kind.name(), Some(record.data())),
                        fields: pair_fields(left.data(), right.data()),
                        left: side(left),
                        right: side(right),
                    })
                })
                .collect())
        })
    }

    /// Lists every two live records of the vault that share a natural key
    /// (data-model 18.6) as a duplicate pair, as a pull that brought one of
    /// them would; pairs already listed or kept stay as they are. Test
    /// support (feature `test-hooks`): for tests and the screenshot run
    /// only; on a device the pairs come from sync.
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn sync_duplicates_scan(&self) -> Result<usize, CoreError> {
        let now_text = format_timestamp(self.now());
        self.with_session(|inner| {
            let session = inner.session.as_ref().ok_or(CoreError::Locked)?;
            let vault_id = session.ring.vault_id();
            let known: HashSet<(Uuid, Uuid)> = inner
                .store
                .duplicate_pairs()?
                .into_iter()
                .filter(|pair| pair.vault_id == vault_id)
                .flat_map(|pair| {
                    [
                        (pair.record_id, pair.other_record_id),
                        (pair.other_record_id, pair.record_id),
                    ]
                })
                .collect();
            let mut by_key: HashMap<(RecordKind, String), Vec<Uuid>> = HashMap::new();
            let keys = natural::Keys::new(&session.index);
            for record in session.index.iter() {
                if let Some(key) = keys.key_of(record) {
                    by_key
                        .entry((record.kind, key))
                        .or_default()
                        .push(record.record_id);
                }
            }
            let mut pairs = Vec::new();
            for ids in by_key.values_mut() {
                ids.sort();
                for other in &ids[1..] {
                    if !known.contains(&(ids[0], *other)) {
                        pairs.push(DuplicatePairRow {
                            vault_id,
                            record_id: ids[0],
                            other_record_id: *other,
                            found_at: now_text.clone(),
                            kept: false,
                        });
                    }
                }
            }
            inner.store.write(|tx| {
                for pair in &pairs {
                    tx.duplicate_pair_put(pair)?;
                }
                Ok(())
            })?;
            self.emit_sync(inner);
            Ok(pairs.len())
        })
    }

    /// Test support (feature `test-hooks`): the next merge at sign-in stops
    /// after it pulled the account vault, as a lost connection would.
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn sync_merge_fail_once_for_test(&self) {
        self.inner().sync.fail_merge = true;
    }

    /// Test support (feature `test-hooks`): every vault row as (id, kind,
    /// status, bound).
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn vault_rows_for_test(&self) -> Result<Vec<(Uuid, String, String, bool)>, CoreError> {
        Ok(self
            .inner()
            .store
            .vaults()?
            .into_iter()
            .map(|vault| {
                (
                    vault.vault_id,
                    format!("{:?}", vault.kind).to_lowercase(),
                    format!("{:?}", vault.status).to_lowercase(),
                    vault.bound,
                )
            })
            .collect())
    }

    /// Test support (feature `test-hooks`): writes a copy of a live
    /// record's data under a new id, as the same object made on another
    /// device arrives by sync (UI 13.7; copies of one key arrive only that
    /// way, an import matches keys by fingerprint). Returns the new id.
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn sync_duplicate_copy_for_test(&self, record_id: Uuid) -> Result<Uuid, CoreError> {
        let now = self.now();
        let mut guard = self.inner();
        let mut parts = super::records::parts(&mut guard)?;
        let (kind, data) = parts
            .session
            .index
            .get(&record_id)
            .map(|record| (record.kind, record.data().clone()))
            .ok_or(CoreError::NotFound)?;
        let id = parts.ids.next(now);
        let mut plan = super::records::Plan::default();
        plan.set(kind, id, data);
        let writes = plan.prepare(&mut parts, now)?;
        self.commit_records(&mut parts, writes, Vec::new(), Vec::new())?;
        Ok(id)
    }

    /// "Keep both": the pair is not offered again.
    pub fn sync_duplicates_keep(
        &self,
        record_id: Uuid,
        other_record_id: Uuid,
    ) -> Result<(), CoreError> {
        let now_text = format_timestamp(self.now());
        self.with_session(|inner| {
            let vault_id = inner
                .session
                .as_ref()
                .map(|s| s.ring.vault_id())
                .ok_or(CoreError::Locked)?;
            inner.store.write(|tx| {
                tx.duplicate_pair_put(&crate::store::DuplicatePairRow {
                    vault_id,
                    record_id,
                    other_record_id,
                    found_at: now_text.clone(),
                    kept: true,
                })
            })?;
            Ok(())
        })
    }

    /// "Merge": references to `drop` move to `keep`, then `drop` is
    /// deleted (to Recently deleted). Two copies of one key follow the
    /// rule of identical keys whichever side was chosen (UI 13.7 and 7.2;
    /// data-model 18.3): the one with the smallest record id stays and
    /// takes `certificate`, `passphrase` and `fido2` where it lacks them.
    pub fn sync_duplicates_merge(&self, keep: Uuid, drop_id: Uuid) -> Result<(), CoreError> {
        let copies = self.with_session(|inner| {
            let session = inner.session.as_ref().ok_or(CoreError::Locked)?;
            if session.index.get(&keep).is_none() || session.index.get(&drop_id).is_none() {
                return Err(CoreError::NotFound);
            }
            Ok(key_copies(session, keep, drop_id).map(|ids| (session.ring.vault_id(), ids)))
        })?;
        if let Some((vault_id, ids)) = copies {
            let kept = self
                .keys_merge(super::dto::RecordRef {
                    vault_id,
                    record_id: keep,
                })?
                .record_id;
            // What the key rule does not rewrite (a credential binding's
            // target) moves as in any merge.
            let gone: Vec<Uuid> = ids.into_iter().filter(|id| *id != kept).collect();
            self.duplicate_links_move(kept, &gone)?;
            return Ok(());
        }
        self.duplicate_merge_into(keep, drop_id)
    }

    /// The merge of two records that are not copies of one key: the links
    /// to `drop_id` move to `keep`, its pairs are forgotten and it is
    /// removed, in one local transaction (data-model 18.3), so a failure
    /// leaves both records, their links and the pair as they were. Removed
    /// at once in local mode before its first sync, otherwise a tombstone;
    /// its last version goes to Recently deleted. A kind
    /// [`Core::record_delete`] takes gets its Undo as there; nothing else of
    /// that removal applies, as nothing links `drop_id` any more (its
    /// group's children, its package's snippets and the workspaces with it
    /// now name `keep`).
    fn duplicate_merge_into(&self, keep: Uuid, drop_id: Uuid) -> Result<(), CoreError> {
        let now = self.now();
        let mut guard = self.inner();
        let mut parts = super::records::parts(&mut guard)?;
        let writes = link_writes(parts.store, parts.session, keep, &[drop_id], now)?;
        let record = parts
            .session
            .index
            .get(&drop_id)
            .ok_or(CoreError::NotFound)?;
        let kind = record.kind;
        let undo = super::records::deletable(kind).then(|| record.payload.clone());
        let vault_id = parts.session.ring.vault_id();
        let bound = parts.store.vault(&vault_id)?.is_some_and(|row| row.bound);
        let previous = parts
            .store
            .record(&vault_id, &drop_id)?
            .ok_or(CoreError::NotFound)?;
        let (deletions, tombstones) = if !bound && previous.synced_revision == 0 {
            (vec![(drop_id, previous.class)], Vec::new())
        } else {
            let tombstone = parts.session.prepare_tombstone(kind, &previous, now)?;
            (Vec::new(), vec![tombstone])
        };
        self.commit_records_then(&mut parts, writes, deletions, tombstones, true, |tx| {
            tx.duplicate_pairs_forget(&vault_id, &drop_id)
        })?;
        if let Some(payload) = undo {
            parts.session.undo.put(drop_id, kind, payload, now);
        }
        tracing::info!("duplicate merged");
        Ok(())
    }

    /// Rewrites every reference to `gone` in the other live records to
    /// `keep` and forgets the pairs of `gone`, in one transaction.
    fn duplicate_links_move(&self, keep: Uuid, gone: &[Uuid]) -> Result<(), CoreError> {
        let now = self.now();
        let mut guard = self.inner();
        let mut parts = super::records::parts(&mut guard)?;
        let writes = link_writes(parts.store, parts.session, keep, gone, now)?;
        let vault_id = parts.session.ring.vault_id();
        self.commit_records_then(&mut parts, writes, Vec::new(), Vec::new(), true, |tx| {
            for id in gone {
                tx.duplicate_pairs_forget(&vault_id, id)?;
            }
            Ok(())
        })
    }

    pub fn sync_brought_back(&self) -> Result<Vec<BroughtBackItem>, CoreError> {
        self.with_session(|inner| {
            let session = inner.session.as_mut().ok_or(CoreError::Locked)?;
            let vault_id = session.ring.vault_id();
            Ok(inner
                .store
                .resync_returned()?
                .into_iter()
                .filter(|(vault, _, _)| *vault == vault_id)
                .filter_map(|(_, id, at)| {
                    let record = session.index.get(&id)?;
                    Some(BroughtBackItem {
                        record_id: id,
                        type_name: record.kind.name().to_owned(),
                        title: title_of(record.kind.name(), Some(record.data())),
                        returned_at: at,
                    })
                })
                .collect())
        })
    }

    /// "Keep": the records stay and leave the list.
    pub fn sync_brought_back_keep(&self, record_ids: &[Uuid]) -> Result<(), CoreError> {
        self.with_session(|inner| {
            let vault_id = inner
                .session
                .as_ref()
                .map(|s| s.ring.vault_id())
                .ok_or(CoreError::Locked)?;
            inner.store.write(|tx| {
                for id in record_ids {
                    tx.resync_returned_delete(&vault_id, id)?;
                }
                Ok(())
            })?;
            Ok(())
        })
    }

    /// "Delete again": the records are deleted (to Recently deleted).
    pub fn sync_brought_back_delete(&self, record_ids: &[Uuid]) -> Result<(), CoreError> {
        let vault_id = self.with_session(|inner| {
            inner
                .session
                .as_ref()
                .map(|s| s.ring.vault_id())
                .ok_or(CoreError::Locked)
        })?;
        self.record_delete(super::dto::RecordsRef {
            vault_id,
            record_ids: record_ids.to_vec(),
        })?;
        self.sync_brought_back_keep(record_ids)
    }
}

/// The export records of chosen Recently deleted entries (UI 14,
/// "Export…"): their last live versions, as `data_export` writes records.
pub(super) fn recently_deleted_export_list(
    store: &Store,
    session: &mut Session,
    record_ids: &[Uuid],
    counts: &mut std::collections::BTreeMap<String, usize>,
) -> Result<Vec<Value>, CoreError> {
    let vault_id = session.ring.vault_id();
    let mut list = Vec::new();
    for entry in store.recently_deleted()? {
        if entry.vault_id != vault_id || !record_ids.contains(&entry.record_id) {
            continue;
        }
        let Some(payload) = open_envelope_payload(&mut session.ring, &entry.envelope) else {
            continue;
        };
        let Some(data) = payload.data.clone() else {
            continue;
        };
        *counts.entry(payload.type_name.clone()).or_default() += 1;
        let revision = envelope::split(&entry.envelope)
            .map(|parts| parts.header.revision)
            .unwrap_or(1);
        list.push(serde_json::json!({
            "record_id": entry.record_id.to_string(),
            "revision": revision,
            "v": payload.v,
            "type": payload.type_name,
            "hlc": payload.hlc.to_text(),
            "device_id": payload.device_id.to_string(),
            "data": Value::Object(data),
        }));
    }
    Ok(list)
}

/// Members holding secrets (passwords, passphrases, private keys): masked
/// wherever a conflict is shown.
const SECRET_MEMBERS: [&str; 6] = [
    "password",
    "passphrase",
    "private_key",
    "secret",
    "token",
    "recovery_key",
];

const MASK: &str = "••••••";

/// Lists of `env_var` objects (`host.ssh.env`, data-model 7.3), whose
/// `value` is a secret (data-model 3.5: `secret(1024)`).
const ENV_LISTS: [&str; 1] = ["env"];

fn masked_value(name: &str, value: &Value) -> Value {
    if SECRET_MEMBERS.contains(&name) && !value.is_null() {
        return Value::String(MASK.to_owned());
    }
    if ENV_LISTS.contains(&name)
        && let Value::Array(items) = value
    {
        return Value::Array(items.iter().map(masked_env_var).collect());
    }
    match value {
        Value::Object(map) => Value::Object(masked(map)),
        Value::Array(items) => {
            Value::Array(items.iter().map(|item| masked_value(name, item)).collect())
        }
        other => other.clone(),
    }
}

/// An `env_var` with its `value` masked; its `name` stays readable.
fn masked_env_var(item: &Value) -> Value {
    match item {
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(name, value)| {
                    let value = if name == "value" && !value.is_null() {
                        Value::String(MASK.to_owned())
                    } else {
                        masked_value(name, value)
                    };
                    (name.clone(), value)
                })
                .collect(),
        ),
        other => masked_value("", other),
    }
}

fn masked(map: &Map<String, Value>) -> Map<String, Value> {
    map.iter()
        .map(|(name, value)| (name.clone(), masked_value(name, value)))
        .collect()
}

fn shown(value: Option<&Value>) -> String {
    match value {
        None | Some(Value::Null) => "—".to_owned(),
        Some(Value::String(text)) => text.clone(),
        Some(other) => other.to_string(),
    }
}

/// The members of a known host that make its key revoked (data-model 9.5).
const REVOCATION: [&str; 3] = ["marker", "key_type", "public_key"];

fn revoked(data: &Map<String, Value>) -> bool {
    data.get("marker").and_then(Value::as_str) == Some("revoked")
}

/// The original's row is live but this device cannot read it (it is left
/// out of the index: damaged, its epoch key missing, a newer payload
/// version).
fn unreadable(ring: &mut KeyRing, row: Option<&RecordRow>) -> bool {
    row.is_some_and(|row| {
        !row.tombstone && !matches!(vault::open_row(ring, row), vault::Opened::Live(..))
    })
}

/// What "Keep other" does with a conflict (UI 13.6): the type rules of
/// sync-protocol 6.7 hold, so a revoked known host key stays revoked.
/// `current` is `None` when the current version is deleted or missing;
/// `current_unreadable` when it is live here but cannot be read.
fn keep_other_of(
    type_name: &str,
    current_unreadable: bool,
    current: Option<&Map<String, Value>>,
    losing_deleted: bool,
    losing: Option<&Map<String, Value>>,
) -> KeepOther {
    // `member_pin`, `roster_entry`, `credential_binding` and types of a
    // newer client: this app never writes them.
    if RecordKind::from_name(type_name).is_none() {
        return KeepOther::Unsupported;
    }
    // Its rules (a revoked key stays revoked) cannot be checked against a
    // version this device cannot read: the copy never replaces it.
    if current_unreadable {
        return KeepOther::Unreadable;
    }
    if type_name == RecordKind::KnownHost.name()
        && let Some(current) = current.filter(|current| revoked(current))
    {
        if losing_deleted {
            return KeepOther::RevokedStays;
        }
        if losing.is_some_and(|losing| {
            REVOCATION
                .iter()
                .any(|name| losing.get(*name) != current.get(*name))
        }) {
            return KeepOther::KeepsRevocation;
        }
    }
    KeepOther::Replaces
}

/// The other version of a known host with the current one's revoked key
/// ([`KeepOther::KeepsRevocation`]).
fn keep_revocation(current: &Map<String, Value>, losing: &mut Map<String, Value>) {
    for name in REVOCATION {
        match current.get(name) {
            Some(value) => {
                losing.insert(name.to_owned(), value.clone());
            }
            None => {
                losing.remove(name);
            }
        }
    }
}

/// A value as the comparison sees it: `null`, `""`, `[]` and objects of
/// such members are absent, as the dialog shows none of them (UI 13.6).
fn present(value: Option<&Value>) -> Option<Value> {
    match value? {
        Value::Null => None,
        Value::String(text) if text.is_empty() => None,
        Value::Array(items) if items.is_empty() => None,
        Value::Object(map) => {
            let kept: Map<String, Value> = map
                .iter()
                .filter_map(|(name, value)| present(Some(value)).map(|value| (name.clone(), value)))
                .collect();
            (!kept.is_empty()).then_some(Value::Object(kept))
        }
        other => Some(other.clone()),
    }
}

/// The differing fields of two versions, one level of nesting flattened
/// (`ssh.port`), secrets masked. The list order (`order_key`) and empty
/// values against absent ones are no difference: the dialog shows
/// neither.
fn differences(
    current: Option<&Map<String, Value>>,
    other: Option<&Map<String, Value>>,
) -> Vec<FieldDifference> {
    // Compared as they are (a changed secret is a difference), shown masked.
    let empty = Map::new();
    let current = current.unwrap_or(&empty);
    let other = other.unwrap_or(&empty);
    let mut paths: Vec<(String, Option<Value>, Option<Value>)> = Vec::new();
    let mut names: Vec<&String> = current
        .keys()
        .chain(other.keys())
        .filter(|name| !UNCOMPARED.contains(&name.as_str()))
        .collect();
    names.sort();
    names.dedup();
    for name in names {
        match (current.get(name), other.get(name)) {
            (Some(Value::Object(a)), Some(Value::Object(b))) => {
                let mut inner: Vec<&String> = a.keys().chain(b.keys()).collect();
                inner.sort();
                inner.dedup();
                for key in inner {
                    paths.push((
                        format!("{name}.{key}"),
                        a.get(key).cloned(),
                        b.get(key).cloned(),
                    ));
                }
            }
            (a, b) => paths.push((name.clone(), a.cloned(), b.cloned())),
        }
    }
    paths
        .into_iter()
        .map(|(path, a, b)| (path, present(a.as_ref()), present(b.as_ref())))
        .filter(|(_, a, b)| a != b)
        .map(|(path, a, b)| {
            let name = path.rsplit('.').next().unwrap_or(&path).to_owned();
            let mask = |value: Option<Value>| value.map(|value| masked_value(&name, &value));
            FieldDifference {
                current: shown(mask(a).as_ref()),
                other: shown(mask(b).as_ref()),
                path,
            }
        })
        .collect()
}

/// The names of the account's devices by device id, from their
/// `device_info` records (UI 13.6 and 13.7: the device of each version).
fn device_names(session: &Session) -> HashMap<String, String> {
    session
        .index
        .of_kind(RecordKind::DeviceInfo)
        .filter_map(|record| {
            let data = record.data();
            let id = data.get("device_id").and_then(Value::as_str)?;
            let name = data.get("name").and_then(Value::as_str)?;
            Some((id.to_owned(), name.to_owned()))
        })
        .collect()
}

fn duplicate_side(
    record: &IndexedRecord,
    devices: &HashMap<String, String>,
    this_device: Uuid,
) -> DuplicateSide {
    let device_id = record.payload.device_id;
    let device = device_id.to_string();
    DuplicateSide {
        record_id: record.record_id,
        title: title_of(record.kind.name(), Some(record.data())),
        created_at: uuid_v7_timestamp(&record.record_id),
        modified_at: format_unix_ms(record.hlc().ms()),
        device_name: devices.get(&device).cloned(),
        device_id: device,
        this_device: device_id == this_device,
    }
}

/// Members of a record's data the comparison leaves out: list order only.
const UNCOMPARED: [&str; 1] = ["order_key"];

/// Every field of two records, one level of nesting flattened (`ssh.port`)
/// as in [`differences`], in path order, secrets masked.
fn pair_fields(left: &Map<String, Value>, right: &Map<String, Value>) -> Vec<PairField> {
    fn flatten(map: &Map<String, Value>) -> Vec<(String, &Value)> {
        let mut list = Vec::new();
        for (name, value) in map {
            if UNCOMPARED.contains(&name.as_str()) || value.is_null() {
                continue;
            }
            match value {
                Value::Object(inner) if !SECRET_MEMBERS.contains(&name.as_str()) => {
                    for (key, value) in inner {
                        if !value.is_null() {
                            list.push((format!("{name}.{key}"), value));
                        }
                    }
                }
                _ => list.push((name.clone(), value)),
            }
        }
        list
    }
    let left: HashMap<String, &Value> = flatten(left).into_iter().collect();
    let right: HashMap<String, &Value> = flatten(right).into_iter().collect();
    let mut paths: Vec<&String> = left.keys().chain(right.keys()).collect();
    paths.sort();
    paths.dedup();
    paths
        .into_iter()
        .map(|path| {
            let name = path.rsplit('.').next().unwrap_or(path);
            let show = |value: Option<&&Value>| match value.map(|value| masked_value(name, value)) {
                // A list of words (tags) as words.
                Some(Value::Array(items)) if items.iter().all(Value::is_string) => items
                    .iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join(", "),
                other => shown(other.as_ref()),
            };
            let (a, b) = (left.get(path), right.get(path));
            PairField {
                path: path.clone(),
                left: show(a),
                right: show(b),
                differs: a != b,
            }
        })
        .collect()
}

/// Rows the merge at sign-in writes: records, Recently deleted entries and
/// the duplicate pairs it raised.
type MergedRows = (
    Vec<RecordRow>,
    Vec<RecentlyDeletedRow>,
    Vec<DuplicatePairRow>,
);

/// After `sync.toggles` changed (section 6.10): classes now pulled that
/// were not, or `appearance` when one of its two types came back, get a
/// class-limited resync, so records the cursor passed while they were off
/// arrive. Only while bound.
pub(super) fn toggles_changed(store: &mut Store, before: &Selection) -> Result<(), CoreError> {
    let Some(vault_id) = store.local_auth()?.map(|row| row.personal_vault_id) else {
        return Ok(());
    };
    if store.server_account()?.is_none() {
        return Ok(());
    }
    let after = selection(store);
    let mut added: Vec<u8> = after
        .classes
        .iter()
        .copied()
        .filter(|class| !before.classes.contains(class))
        .collect();
    if before
        .skip_types
        .iter()
        .any(|name| !after.skip_types.contains(name))
        && !added.contains(&classes::APPEARANCE)
    {
        added.push(classes::APPEARANCE);
    }
    if added.is_empty() {
        return Ok(());
    }
    let mut state = store.sync_state(&vault_id)?;
    state.vault_id = vault_id;
    for class in added {
        if !state.resync_classes.contains(&class) {
            state.resync_classes.push(class);
        }
    }
    store.write(|tx| tx.sync_state_put(&state))?;
    Ok(())
}

/// Live conflict copies of the vault with their payloads.
fn conflict_rows(
    store: &Store,
    session: &mut Session,
) -> Result<Vec<(RecordRow, Payload)>, CoreError> {
    let vault_id = session.ring.vault_id();
    let mut rows = Vec::new();
    for row in store.records(&vault_id, None)? {
        // Conflict copies are never in the index (no record kind).
        if row.tombstone || session.index.get(&row.record_id).is_some() {
            continue;
        }
        if let Some(payload) = open_payload(&mut session.ring, &row)
            && payload.type_name == "conflict"
            && !payload.deleted
        {
            rows.push((row, payload));
        }
    }
    Ok(rows)
}

/// When `a` and `b` are live keys with one fingerprint (the key's natural
/// key, data-model 18.6): every live key with it, by record id.
fn key_copies(session: &Session, a: Uuid, b: Uuid) -> Option<Vec<Uuid>> {
    let print = |id: &Uuid| {
        session
            .index
            .get_kind(id, RecordKind::Key)
            .and_then(|record| crate::keys::record_fingerprint(record.data()))
    };
    let fingerprint = print(&a).filter(|print_a| print(&b).as_ref() == Some(print_a))?;
    let mut ids: Vec<Uuid> = session
        .index
        .of_kind(RecordKind::Key)
        .filter(|record| {
            crate::keys::record_fingerprint(record.data()).as_ref() == Some(&fingerprint)
        })
        .map(|record| record.record_id)
        .collect();
    ids.sort();
    Some(ids)
}

/// The next versions of the live records (other than `gone`) that link one
/// of `gone`, the links rewritten to `keep` (UI 13.7; data-model 18.3).
/// History and log entries link hosts too (`history_entry`,
/// `history_hint_state`, `connection_log_entry`): their lazy classes are
/// loaded first (D10), as the conflict resolution does.
fn link_writes(
    store: &Store,
    session: &mut Session,
    keep: Uuid,
    gone: &[Uuid],
    now: SystemTime,
) -> Result<Vec<super::session::PendingWrite>, CoreError> {
    for class in crate::model::types::LAZY_CLASSES {
        session.ensure_class(store, class)?;
    }
    let map: HashMap<String, String> = gone
        .iter()
        .map(|id| (id.to_string(), keep.to_string()))
        .collect();
    let targets: Vec<(Uuid, RecordKind, Map<String, Value>)> = session
        .index
        .iter()
        .filter(|record| !gone.contains(&record.record_id))
        .filter_map(|record| {
            let data = remap_data(record.data(), &map);
            (data != *record.data()).then_some((record.record_id, record.kind, data))
        })
        .collect();
    let mut writes = Vec::with_capacity(targets.len());
    for (id, kind, data) in targets {
        writes.push(session.prepare_write(store, kind, id, data, now)?);
    }
    Ok(writes)
}

/// Duplicate pairs not kept whose two records are both live, each once. A
/// database written before one pair had one row may hold it in both
/// orders: kept in either, it is kept.
fn live_pairs(store: &Store, session: &Session) -> Result<Vec<(Uuid, Uuid)>, CoreError> {
    let vault_id = session.ring.vault_id();
    let rows: Vec<DuplicatePairRow> = store
        .duplicate_pairs()?
        .into_iter()
        .filter(|pair| pair.vault_id == vault_id)
        .collect();
    let unordered = |a: Uuid, b: Uuid| if a < b { (a, b) } else { (b, a) };
    let mut seen: HashSet<(Uuid, Uuid)> = rows
        .iter()
        .filter(|pair| pair.kept)
        .map(|pair| unordered(pair.record_id, pair.other_record_id))
        .collect();
    Ok(rows
        .into_iter()
        .filter(|pair| {
            session.index.get(&pair.record_id).is_some()
                && session.index.get(&pair.other_record_id).is_some()
                && seen.insert(unordered(pair.record_id, pair.other_record_id))
        })
        .map(|pair| (pair.record_id, pair.other_record_id))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn conflict_differences_mask_secrets() {
        let current =
            json!({"address": "h", "label": "A", "ssh": {"port": 22, "password": "old-secret"}});
        let other =
            json!({"address": "h", "label": "B", "ssh": {"port": 2222, "password": "new-secret"}});
        let found = differences(current.as_object(), other.as_object());
        let paths: Vec<&str> = found.iter().map(|d| d.path.as_str()).collect();
        assert_eq!(paths, vec!["label", "ssh.password", "ssh.port"]);
        let password = &found[1];
        assert_eq!(password.current, "••••••");
        assert_eq!(password.other, "••••••");
        let masked_other = masked(other.as_object().unwrap());
        assert_eq!(masked_other["ssh"]["password"], json!("••••••"));
        assert!(!serde_json::to_string(&found).unwrap().contains("secret"));
    }

    #[test]
    fn pair_fields_list_both_sides_and_mask_secrets() {
        let left = json!({"address": "h", "order_key": "a0", "ssh": {"port": 22, "password": "left-secret"},
            "proxy": [{"password": "list-secret"}], "tags": ["a", "b"]});
        let right = json!({"address": "h", "order_key": "b0", "ssh": {"port": 2222, "password": "left-secret"},
            "notes": "n", "tags": ["a", "b"]});
        let fields = pair_fields(left.as_object().unwrap(), right.as_object().unwrap());
        let rows: Vec<(&str, &str, &str, bool)> = fields
            .iter()
            .map(|f| {
                (
                    f.path.as_str(),
                    f.left.as_str(),
                    f.right.as_str(),
                    f.differs,
                )
            })
            .collect();
        assert_eq!(
            rows,
            vec![
                ("address", "h", "h", false),
                ("notes", "—", "n", true),
                ("proxy", r#"[{"password":"••••••"}]"#, "—", true),
                ("ssh.password", "••••••", "••••••", false),
                ("ssh.port", "22", "2222", true),
                ("tags", "a, b", "a, b", false),
            ],
            "order_key is left out"
        );
        assert!(!serde_json::to_string(&fields).unwrap().contains("secret"));
    }

    #[test]
    fn remap_rewrites_values_and_keys() {
        let a = Uuid::now_v7().to_string();
        let b = Uuid::now_v7().to_string();
        let map = HashMap::from([(a.clone(), b.clone())]);
        let data =
            json!({"group_id": a, "ssh": {"key_id": a}, "list": [a, "x"], "by": {a.clone(): 1}});
        let out = remap_data(data.as_object().unwrap(), &map);
        assert_eq!(
            Value::Object(out),
            json!({"group_id": b, "ssh": {"key_id": b}, "list": [b, "x"], "by": {b.clone(): 1}})
        );
    }

    #[test]
    fn titles_fall_back_to_the_type() {
        assert_eq!(
            title_of("host", Some(json!({"address": "h"}).as_object().unwrap())),
            "h"
        );
        assert_eq!(
            title_of(
                "host",
                Some(json!({"label": "L", "address": "h"}).as_object().unwrap())
            ),
            "L"
        );
        assert_eq!(title_of("key", None), "key");
    }

    #[test]
    fn refresh_token_wrap_round_trip_and_tamper() {
        let vault_id = Uuid::now_v7();
        let device = Uuid::now_v7();
        let ring = KeyRing::new(vault_id, 1, Key32::generate().unwrap());
        let (blob, epoch) = wrap_refresh(&ring, &device, "https://x", "shellpp_rt_abc").unwrap();
        let mut account = ServerAccountRow {
            server_url: "https://x".into(),
            user_id: Uuid::now_v7(),
            email: "a@b".into(),
            refresh_token: Some(blob.clone()),
            secrets_epoch: epoch,
            bound_at: String::new(),
            server_generation: None,
        };
        assert_eq!(
            open_refresh(&ring, &device, &account)
                .as_deref()
                .map(String::as_str),
            Some("shellpp_rt_abc")
        );
        // Another device id (the AAD) or a flipped byte does not open.
        assert!(open_refresh(&ring, &Uuid::now_v7(), &account).is_none());
        // Nor another server address (D188): the token goes only to the
        // origin that issued it.
        let mut moved = account.clone();
        moved.server_url = "https://attacker.example".into();
        assert!(open_refresh(&ring, &device, &moved).is_none());
        let mut tampered = blob;
        let last = tampered.len() - 1;
        tampered[last] ^= 1;
        account.refresh_token = Some(tampered);
        assert!(open_refresh(&ring, &device, &account).is_none());
    }

    #[test]
    fn account_wraps_open_only_with_their_key_and_aad() {
        let vault_id = Uuid::now_v7();
        let kek = Key32::generate().unwrap();
        let vault_key = Key32::generate().unwrap();
        let row = vault::wrap_row(WrapKind::Kek, vault_id, 1, &kek, &vault_key).unwrap();
        let opened = open_account_wrap(vault_id, 1, &row.wrap, &kek, WrapKind::Kek).unwrap();
        assert!(opened.ct_eq(&vault_key));
        assert!(open_account_wrap(vault_id, 2, &row.wrap, &kek, WrapKind::Kek).is_none());
        assert!(open_account_wrap(Uuid::now_v7(), 1, &row.wrap, &kek, WrapKind::Kek).is_none());
        assert!(open_account_wrap(vault_id, 1, &row.wrap, &kek, WrapKind::Recovery).is_none());
        let other = Key32::generate().unwrap();
        assert!(open_account_wrap(vault_id, 1, &row.wrap, &other, WrapKind::Kek).is_none());
    }

    #[test]
    fn epoch_chains_open_down_to_epoch_one_and_refuse_a_swap() {
        let vault_id = Uuid::now_v7();
        let k1 = Key32::generate().unwrap();
        let k2 = Key32::generate().unwrap();
        let k3 = Key32::generate().unwrap();
        let c2 = vault::chain_row(vault_id, 2, &k2, &k1).unwrap();
        let c3 = vault::chain_row(vault_id, 3, &k3, &k2).unwrap();
        let info = |epoch, wrap: Vec<u8>| wire::EpochInfo {
            epoch,
            prev_key_wrap: Some(B64(wrap)),
            created_by: Uuid::nil(),
            created_at: String::new(),
        };
        let chain = vec![info(2, c2.wrap.clone()), info(3, c3.wrap.clone())];
        let (ring, rows) = ring_from_chain(vault_id, 3, k3.duplicate(), &chain).unwrap();
        assert_eq!(ring.current_epoch(), 3);
        assert!(ring.key(1).unwrap().ct_eq(&k1));
        assert_eq!(rows.len(), 2);
        // Swapped links do not open.
        let swapped = vec![info(2, c3.wrap), info(3, c2.wrap)];
        assert!(ring_from_chain(vault_id, 3, k3, &swapped).is_err());
    }

    /// A core with a vault, as at first run.
    fn unlocked_core() -> (tempfile::TempDir, Core) {
        use crate::platform::OsKind;
        use crate::platform::memory::MemoryPlatform;
        let dir = tempfile::tempdir().unwrap();
        let mem = MemoryPlatform::new();
        let core = Core::open(
            super::super::CoreConfig {
                data_dir: dir.path().join("data"),
                app_version: "0.1.0".into(),
            },
            mem.platform(OsKind::Linux, "TEST"),
        )
        .unwrap();
        core.first_run_setup(SecretString::from(format!("Pw-{}", random_text())))
            .unwrap();
        (dir, core)
    }

    fn random_text() -> String {
        crypto::random_array::<12>()
            .unwrap()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }

    fn save_host(core: &Core, label: &str, password: &str) -> Uuid {
        use crate::service::dto::{RecordSaveInput, RecordType};
        let vault_id = core.vaults().unwrap()[0].vault_id;
        core.record_save(RecordSaveInput {
            vault_id,
            record_id: None,
            record_type: RecordType::Host,
            data: json!({"address": "192.0.2.10", "label": label, "ssh": {"port": 22}})
                .as_object()
                .unwrap()
                .clone(),
            secrets: std::collections::BTreeMap::from([(
                "ssh.password".to_owned(),
                Some(SecretString::from(password.to_owned())),
            )]),
        })
        .unwrap()
        .record_id()
    }

    fn stored_data(core: &Core, id: Uuid) -> Map<String, Value> {
        let guard = core.inner();
        let session = guard.session.as_ref().unwrap();
        session.index.get(&id).unwrap().data().clone()
    }

    /// The comparison (UI 13.6) gets both versions: their data with secrets
    /// masked, their devices and times; a deletion on either side has no
    /// data. No secret leaves the core in a conflict.
    #[test]
    fn conflicts_carry_both_versions_masked() {
        let (_dir, core) = unlocked_core();
        let (current_secret, other_secret) = (random_text(), random_text());
        let host = save_host(&core, "web", &current_secret);
        let mut other = stored_data(&core, host);
        assert_eq!(other["ssh"]["password"], json!(current_secret.clone()));
        other.insert("label".into(), json!("web-2"));
        other["ssh"]["port"] = json!(2222);
        other["ssh"]["password"] = json!(other_secret.clone());
        let device = Uuid::now_v7();
        let at = SystemTime::UNIX_EPOCH + Duration::from_secs(1_790_000_000);
        let edited = core
            .sync_conflict_insert_for_test(host, Some(other), device, at)
            .unwrap();
        let deleted = core
            .sync_conflict_insert_for_test(host, None, device, at)
            .unwrap();
        assert_eq!(core.sync_status().unwrap().conflicts, 2);

        let items = core.sync_conflicts().unwrap();
        let text = serde_json::to_string(&items).unwrap();
        assert!(
            !text.contains(&current_secret),
            "the current secret is masked"
        );
        assert!(!text.contains(&other_secret), "the other secret is masked");
        let item = items.iter().find(|i| i.conflict_id == edited).unwrap();
        assert_eq!(item.original_record_id, host);
        assert_eq!(item.original_type, "host");
        assert_eq!(item.title, "web");
        assert!(!item.current_deleted && !item.losing_deleted);
        let current = item.current_data.as_ref().unwrap();
        let losing = item.losing_data.as_ref().unwrap();
        assert_eq!(current["label"], json!("web"));
        assert_eq!(losing["label"], json!("web-2"));
        assert_eq!(current["ssh"]["password"], json!(MASK));
        assert_eq!(losing["ssh"]["password"], json!(MASK));
        let paths: Vec<&str> = item.differences.iter().map(|d| d.path.as_str()).collect();
        assert_eq!(paths, ["label", "ssh.password", "ssh.port"]);
        assert!(item.current_this_device);
        assert!(!item.losing_this_device);
        assert_eq!(item.losing_device_id, Some(device.to_string()));
        assert!(item.current_device_id.is_some());
        assert_ne!(item.current_device_id, item.losing_device_id);
        assert_eq!(
            item.losing_changed_at.as_deref(),
            Some(format_timestamp(at).as_str())
        );
        let changed = item.current_changed_at.as_deref().unwrap();
        assert!(crate::model::time::is_timestamp(changed), "{changed}");

        let gone = items.iter().find(|i| i.conflict_id == deleted).unwrap();
        assert!(gone.losing_deleted);
        assert!(gone.losing_data.is_none());
        assert!(gone.current_data.is_some());

        // "Keep other" makes the copy's version the record's next one.
        core.sync_conflict_resolve(edited, ConflictChoice::Copy)
            .unwrap();
        let now = stored_data(&core, host);
        assert_eq!(now["label"], json!("web-2"));
        assert_eq!(now["ssh"]["password"], json!(other_secret));
        // "Keep this" only drops the copy.
        core.sync_conflict_resolve(deleted, ConflictChoice::Current)
            .unwrap();
        assert!(core.sync_conflicts().unwrap().is_empty());
        assert_eq!(stored_data(&core, host)["label"], json!("web-2"));
    }

    /// The current version deleted here: its side has no data.
    #[test]
    fn a_deleted_current_version_has_no_data() {
        let (_dir, core) = unlocked_core();
        let host = save_host(&core, "db", &random_text());
        let other = stored_data(&core, host);
        let conflict = core
            .sync_conflict_insert_for_test(host, Some(other), Uuid::now_v7(), SystemTime::now())
            .unwrap();
        let vault_id = core.vaults().unwrap()[0].vault_id;
        core.record_delete(crate::service::dto::RecordsRef {
            vault_id,
            record_ids: vec![host],
        })
        .unwrap();
        let items = core.sync_conflicts().unwrap();
        let item = items.iter().find(|i| i.conflict_id == conflict).unwrap();
        assert!(item.current_deleted);
        assert!(item.current_data.is_none());
        assert_eq!(item.title, "db", "named after the other version");
        assert!(item.losing_data.is_some());
    }

    /// A core with a vault and its master password (generated here).
    fn core_with_password() -> (tempfile::TempDir, Core, SecretString) {
        use crate::platform::OsKind;
        use crate::platform::memory::MemoryPlatform;
        let dir = tempfile::tempdir().unwrap();
        let core = Core::open(
            super::super::CoreConfig {
                data_dir: dir.path().join("data"),
                app_version: "0.1.0".into(),
            },
            MemoryPlatform::new().platform(OsKind::Linux, "TEST"),
        )
        .unwrap();
        let password = SecretString::from(format!("Pw-{}", random_text()));
        core.first_run_setup(password.clone()).unwrap();
        (dir, core, password)
    }

    fn item_of(core: &Core, conflict: Uuid) -> ConflictItem {
        core.sync_conflicts()
            .unwrap()
            .into_iter()
            .find(|item| item.conflict_id == conflict)
            .expect("the conflict is listed")
    }

    /// A random Ed25519 host key: base64 of its SSH wire form.
    fn host_key_blob() -> String {
        let mut blob = Vec::new();
        let key = crypto::random_array::<32>().unwrap();
        for part in [b"ssh-ed25519".as_slice(), key.as_slice()] {
            blob.extend_from_slice(&(part.len() as u32).to_be_bytes());
            blob.extend_from_slice(part);
        }
        data_encoding::BASE64.encode(&blob)
    }

    fn add_known_host(core: &Core, revoked: bool) -> Uuid {
        core.known_host_add(crate::service::dto::KnownHostAddInput {
            host: "192.0.2.20".into(),
            port: Some(22),
            key_type: "ssh-ed25519".into(),
            public_key: host_key_blob(),
            revoked,
            comment: None,
        })
        .unwrap()
        .record_id
    }

    fn row_of(core: &Core, id: Uuid) -> Option<RecordRow> {
        let guard = core.inner();
        let vault_id = guard.session.as_ref().unwrap().ring.vault_id();
        guard.store.record(&vault_id, &id).unwrap()
    }

    /// Sync-protocol 6.7 and UI 13.6: a revoked known host key stays
    /// revoked through "Keep other" too. An edit made elsewhere before the
    /// revocation replaces the rest of the entry and keeps the revoked key
    /// (also when it holds another key); a deletion of the revoked entry is
    /// refused and the conflict stays.
    #[test]
    fn keep_other_leaves_a_revoked_known_host_key_revoked() {
        let (_dir, core) = unlocked_core();
        let known = add_known_host(&core, true);
        let revoked = stored_data(&core, known);
        assert_eq!(revoked["marker"], json!("revoked"));
        let device = Uuid::now_v7();

        let mut edited = revoked.clone();
        edited.remove("marker");
        edited.insert("comment".into(), json!("edited elsewhere"));
        let conflict = core
            .sync_conflict_insert_for_test(known, Some(edited), device, SystemTime::now())
            .unwrap();
        assert_eq!(
            item_of(&core, conflict).keep_other,
            KeepOther::KeepsRevocation
        );
        core.sync_conflict_resolve(conflict, ConflictChoice::Copy)
            .unwrap();
        let now = stored_data(&core, known);
        assert_eq!(now["marker"], json!("revoked"), "the key stays revoked");
        assert_eq!(now["public_key"], revoked["public_key"]);
        assert_eq!(now["comment"], json!("edited elsewhere"));

        let mut replaced = revoked.clone();
        replaced.remove("marker");
        replaced.insert("public_key".into(), json!(host_key_blob()));
        let conflict = core
            .sync_conflict_insert_for_test(known, Some(replaced), device, SystemTime::now())
            .unwrap();
        assert_eq!(
            item_of(&core, conflict).keep_other,
            KeepOther::KeepsRevocation
        );
        core.sync_conflict_resolve(conflict, ConflictChoice::Copy)
            .unwrap();
        let now = stored_data(&core, known);
        assert_eq!(now["marker"], json!("revoked"));
        assert_eq!(now["public_key"], revoked["public_key"], "the revoked key");

        let deletion = core
            .sync_conflict_insert_for_test(known, None, device, SystemTime::now())
            .unwrap();
        assert_eq!(item_of(&core, deletion).keep_other, KeepOther::RevokedStays);
        assert!(matches!(
            core.sync_conflict_resolve(deletion, ConflictChoice::Copy),
            Err(CoreError::ReadOnly)
        ));
        assert!(
            item_of(&core, deletion).losing_deleted,
            "the conflict stays"
        );
        assert_eq!(stored_data(&core, known)["marker"], json!("revoked"));
        core.sync_conflict_resolve(deletion, ConflictChoice::Current)
            .unwrap();
        assert!(core.sync_conflicts().unwrap().is_empty());

        // Not revoked: the other version replaces it whole.
        let plain = add_known_host(&core, false);
        let mut other = stored_data(&core, plain);
        other.insert("marker".into(), json!("revoked"));
        let conflict = core
            .sync_conflict_insert_for_test(plain, Some(other), device, SystemTime::now())
            .unwrap();
        assert_eq!(item_of(&core, conflict).keep_other, KeepOther::Replaces);
    }

    /// A record of a type this app does not write (a newer client's
    /// `terminal_theme`, a `member_pin` confirmed by a fingerprint
    /// comparison): compared, but "Keep other" is refused and the conflict
    /// stays; "Keep this" resolves it.
    #[test]
    fn keep_other_is_refused_for_types_this_app_does_not_write() {
        let (_dir, core) = unlocked_core();
        for (type_name, class) in [
            ("terminal_theme", classes::APPEARANCE),
            ("member_pin", classes::VAULT),
        ] {
            let id = Uuid::now_v7();
            {
                let mut guard = core.inner();
                let inner = &mut *guard;
                let session = inner.session.as_mut().unwrap();
                let hlc = session.hlc.tick(unix_ms(SystemTime::now()));
                let payload = Payload::live(
                    type_name,
                    1,
                    hlc,
                    session.device_id,
                    json!({"name": "ours", "verified": true})
                        .as_object()
                        .cloned()
                        .unwrap(),
                );
                let row = vault::seal_row(
                    &mut session.ring,
                    class,
                    id,
                    &payload,
                    None,
                    &format_timestamp(SystemTime::now()),
                )
                .unwrap();
                inner.store.write(|tx| tx.record_put(&row)).unwrap();
            }
            let other = json!({"name": "theirs", "verified": false});
            let conflict = core
                .sync_conflict_insert_for_test(
                    id,
                    other.as_object().cloned(),
                    Uuid::now_v7(),
                    SystemTime::now(),
                )
                .unwrap();
            let item = item_of(&core, conflict);
            assert_eq!(item.keep_other, KeepOther::Unsupported, "{type_name}");
            assert_eq!(item.current_data.as_ref().unwrap()["name"], json!("ours"));
            assert!(matches!(
                core.sync_conflict_resolve(conflict, ConflictChoice::Copy),
                Err(CoreError::ReadOnly)
            ));
            assert_eq!(item_of(&core, conflict).keep_other, KeepOther::Unsupported);
            let row = row_of(&core, id).unwrap();
            assert_eq!(row.revision, 1, "{type_name} is not rewritten");
            core.sync_conflict_resolve(conflict, ConflictChoice::Current)
                .unwrap();
            assert!(core.sync_conflicts().unwrap().is_empty());
        }
    }

    /// "Keep other" with a deletion deletes a kind `record_delete` does not
    /// take (a known host) and a connection log entry whose lazy class was
    /// not loaded since the unlock (D10): the copy never goes alone.
    #[test]
    fn keep_other_deletes_known_hosts_and_entries_not_loaded_yet() {
        let (_dir, core, password) = core_with_password();
        let known = add_known_host(&core, false);
        let entry = core
            .connection_log_start(crate::service::host_api::ConnectionLogStart {
                host: None,
                host_label: None,
                address: "192.0.2.80".into(),
                port: 22,
                username: None,
                os: None,
            })
            .unwrap();
        core.lock().unwrap();
        core.unlock(password).unwrap();
        let loaded = core
            .inner()
            .session
            .as_ref()
            .unwrap()
            .lazy_loaded
            .contains(&classes::CONNECTION_LOG);
        assert!(!loaded, "the log is not loaded yet");
        let device = Uuid::now_v7();
        let known_gone = core
            .sync_conflict_insert_for_test(known, None, device, SystemTime::now())
            .unwrap();
        let entry_gone = core
            .sync_conflict_insert_for_test(entry, None, device, SystemTime::now())
            .unwrap();
        let item = item_of(&core, entry_gone);
        assert!(!item.current_deleted && !item.current_missing);
        assert!(item.current_data.is_some(), "read from its row");

        core.sync_conflict_resolve(entry_gone, ConflictChoice::Copy)
            .unwrap();
        core.sync_conflict_resolve(known_gone, ConflictChoice::Copy)
            .unwrap();
        assert!(core.sync_conflicts().unwrap().is_empty());
        for id in [known, entry] {
            assert!(row_of(&core, id).unwrap().tombstone, "deleted");
        }
        let log = core.connection_log_list().unwrap();
        assert!(log.iter().all(|view| view.record_id != entry));
        let known_hosts = core.known_hosts_list().unwrap();
        assert!(known_hosts.iter().all(|view| view.record_id != known));
        let trash = core.recently_deleted_list().unwrap();
        for id in [known, entry] {
            assert!(
                trash.iter().any(|item| item.record_id == id),
                "in the trash"
            );
        }
    }

    /// Data-model 15: an original no longer here (its row purged) is
    /// "missing", not "Deleted"; "Keep other" recreates it under its id.
    #[test]
    fn a_purged_original_is_missing_and_keep_other_recreates_it() {
        let (_dir, core) = unlocked_core();
        let host = save_host(&core, "db", &random_text());
        let mut other = stored_data(&core, host);
        other.insert("label".into(), json!("db-2"));
        let conflict = core
            .sync_conflict_insert_for_test(host, Some(other), Uuid::now_v7(), SystemTime::now())
            .unwrap();
        let vault_id = core.vaults().unwrap()[0].vault_id;
        // Not bound: the row goes at once; emptied from the trash, nothing
        // of it is left here.
        core.record_delete(crate::service::dto::RecordsRef {
            vault_id,
            record_ids: vec![host],
        })
        .unwrap();
        core.recently_deleted_delete(&[host]).unwrap();
        assert!(row_of(&core, host).is_none());

        let item = item_of(&core, conflict);
        assert!(item.current_missing, "missing");
        assert!(!item.current_deleted, "not a deletion");
        assert!(item.current_data.is_none());
        assert_eq!(item.title, "db-2", "named after the other version");
        assert_eq!(item.keep_other, KeepOther::Replaces);
        core.sync_conflict_resolve(conflict, ConflictChoice::Copy)
            .unwrap();
        assert_eq!(stored_data(&core, host)["label"], json!("db-2"));
    }

    /// Sync-protocol 6.7 and UI 13.6: an original still here that this
    /// device cannot read (its row damaged) is shown as missing, and "Keep
    /// other" is refused for it as for a deletion: its rules cannot be
    /// checked, so a revoked known host key is never replaced by the copy.
    /// The conflict stays; "Keep this" resolves it.
    #[test]
    fn keep_other_is_refused_when_the_current_version_cannot_be_read() {
        let (_dir, core) = unlocked_core();
        let known = add_known_host(&core, true);
        let mut edited = stored_data(&core, known);
        edited.remove("marker");
        edited.insert("comment".into(), json!("edited elsewhere"));
        let conflict = core
            .sync_conflict_insert_for_test(known, Some(edited), Uuid::now_v7(), SystemTime::now())
            .unwrap();
        // The envelope no longer opens (its tag changed).
        let mut damaged = row_of(&core, known).unwrap();
        *damaged.envelope.last_mut().unwrap() ^= 0x01;
        core.inner()
            .store
            .write(|tx| tx.record_put(&damaged))
            .unwrap();

        let item = item_of(&core, conflict);
        assert!(item.current_missing, "shown as missing");
        assert!(!item.current_deleted);
        assert_eq!(item.keep_other, KeepOther::Unreadable);
        assert!(!item.keep_other.allowed());
        assert!(matches!(
            core.sync_conflict_resolve(conflict, ConflictChoice::Copy),
            Err(CoreError::ReadOnly)
        ));
        let row = row_of(&core, known).unwrap();
        assert_eq!(row.revision, damaged.revision, "not rewritten");
        assert_eq!(row.envelope, damaged.envelope);
        assert_eq!(
            item_of(&core, conflict).keep_other,
            KeepOther::Unreadable,
            "the conflict stays"
        );
        core.sync_conflict_resolve(conflict, ConflictChoice::Current)
            .unwrap();
        assert!(core.sync_conflicts().unwrap().is_empty());
    }

    /// Security 9.1: a comparison's field and a brought-back record show no
    /// address, user name or command in `Debug`.
    #[test]
    fn debug_of_differences_and_brought_back_items_has_no_host_data() {
        let difference = FieldDifference {
            path: "address".into(),
            current: "192.0.2.10".into(),
            other: "deploy@198.51.100.7".into(),
        };
        let text = format!("{difference:?}");
        assert!(text.contains("address"), "{text}");
        for value in ["192.0.2.10", "198.51.100.7", "deploy"] {
            assert!(!text.contains(value), "{text}");
        }
        let item = BroughtBackItem {
            record_id: Uuid::now_v7(),
            type_name: "host".into(),
            title: "db.example.com".into(),
            returned_at: "2026-10-23T10:00:00Z".into(),
        };
        let text = format!("{item:?}");
        assert!(text.contains("host"), "{text}");
        assert!(!text.contains("db.example.com"), "{text}");
    }

    /// The comparison's differences are what the dialog can show: the list
    /// order and empty values against absent ones are none (UI 13.6).
    #[test]
    fn differences_leave_out_the_order_and_empty_values() {
        let current = json!({"address": "h", "order_key": "a0", "tags": [], "notes": "",
            "ssh": {"port": 22, "user": ""}, "proxy": {}});
        let other = json!({"address": "h", "order_key": "b0", "ssh": {"port": 22},
            "env": {"A": null}});
        assert_eq!(
            differences(current.as_object(), other.as_object()),
            Vec::new()
        );
        let other = json!({"address": "h", "tags": ["x"], "ssh": {"port": 22, "user": "u"}});
        let found = differences(current.as_object(), other.as_object());
        let rows: Vec<(&str, &str, &str)> = found
            .iter()
            .map(|d| (d.path.as_str(), d.current.as_str(), d.other.as_str()))
            .collect();
        assert_eq!(rows, [("ssh.user", "—", "u"), ("tags", "—", "[\"x\"]")]);
    }

    /// The devices of a conflict's versions are named by their
    /// `device_info` records (UI 13.6).
    #[test]
    fn conflicts_name_the_devices_from_device_info() {
        let (_dir, core) = unlocked_core();
        let host = save_host(&core, "web", &random_text());
        let other_device = Uuid::now_v7();
        {
            let mut guard = core.inner();
            let inner = &mut *guard;
            let session = inner.session.as_mut().unwrap();
            let data = json!({"device_id": other_device.to_string(), "name": "Office laptop",
                "platform": "linux", "app_version": "0.1.0"});
            let write = session
                .prepare_write(
                    &inner.store,
                    RecordKind::DeviceInfo,
                    Uuid::now_v7(),
                    data.as_object().cloned().unwrap(),
                    SystemTime::now(),
                )
                .unwrap();
            inner.store.write(|tx| tx.record_put(&write.row)).unwrap();
            session.commit(vec![write]);
        }
        let other = stored_data(&core, host);
        let conflict = core
            .sync_conflict_insert_for_test(host, Some(other), other_device, SystemTime::now())
            .unwrap();
        let item = item_of(&core, conflict);
        assert_eq!(item.losing_device_name.as_deref(), Some("Office laptop"));
        assert_eq!(item.losing_device_id, Some(other_device.to_string()));
        assert!(!item.losing_this_device);
        // This device's own record names the current version.
        assert_eq!(item.current_device_name.as_deref(), Some("TEST"));
        assert!(item.current_this_device);
    }

    #[test]
    fn stale_key_epoch_keeps_the_item_pending_and_not_refused() {
        // Run 7 finding 19 (section 6.5): `stale_key_epoch` asks for the
        // keys and another push, it does not park the item; an accepted
        // deletion is reported, so the quota's refusals go again.
        let (_dir, core) = unlocked_core();
        let host = save_host(&core, "Stale", &random_text());
        let mut guard = core.inner();
        let inner = &mut *guard;
        let session = inner.session.as_mut().unwrap();
        let now = core.now();
        let batch = engine::build_push(
            &mut inner.store,
            session,
            &Selection::all(),
            &HashSet::new(),
            now,
        )
        .unwrap();
        assert!(batch.sent.iter().any(|row| row.record_id == host));
        let results: Vec<wire::PushResult> = batch
            .sent
            .iter()
            .map(|row| wire::PushResult {
                record_id: row.record_id,
                status: if row.record_id == host {
                    wire::ItemStatus::StaleKeyEpoch
                } else {
                    wire::ItemStatus::QuotaExceeded
                },
                seq: None,
                current: None,
            })
            .collect();
        let effects = engine::apply_push_results(
            &mut inner.store,
            session,
            &batch.sent,
            &results,
            &Selection::all(),
            now,
        )
        .unwrap();
        assert_eq!(effects.stale_key_epoch, vec![host]);
        assert!(!effects.newer_epoch, "the keys decide, not the answer");
        assert!(
            effects
                .failed
                .iter()
                .all(|(id, status)| *id != host && status == "quota_exceeded"),
            "{:?}",
            effects.failed
        );
        assert!(!effects.pushed_deletion);
        let row = inner
            .store
            .record(&session.ring.vault_id(), &host)
            .unwrap()
            .unwrap();
        assert_ne!(row.revision, row.synced_revision, "still pending");
    }

    #[test]
    fn selection_follows_the_toggles() {
        let store = Store::open_in_memory().unwrap();
        let sel = selection(&store);
        assert!(sel.classes.contains(&classes::HOSTS));
        assert!(!sel.classes.contains(&classes::HISTORY));
        assert!(sel.skip_types.is_empty());
    }
}
