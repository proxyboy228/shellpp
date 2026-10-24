//! The record side of sync (docs/sync-protocol.md, sections 6.4-6.9): applying
//! pulled records with their checks, conflict resolution, building a push
//! and handling its results, the full resync and the server restore.
//!
//! Every function here runs under the core's lock on the store and the
//! session; the network calls happen between them. A page of a pull is one
//! local transaction with its cursor (section 6.4, step 7).
//!
//! Principle 3: nothing here deletes a record because the server lacks it.
//! Only an authenticated tombstone, or the user's own action, deletes; a
//! deleted record's last live version goes to Recently deleted.

use std::collections::{HashMap, HashSet};
use std::time::SystemTime;

use serde_json::{Map, Value};
use shellpp_sync_api::api::{B64, ItemStatus, PushResult, RecordWrite, SeqEnvelope};
use shellpp_sync_api::envelope;
use uuid::Uuid;

use crate::error::CoreError;
use crate::index::IndexedRecord;
use crate::model::hlc::Hlc;
use crate::model::payload::Payload;
use crate::model::time::format_timestamp;
use crate::model::types::RecordKind;
use crate::service::session::Session;
use crate::store::{
    DeletedOrigin, DuplicatePairRow, RecentlyDeletedRow, RecordRow, Store, SyncStateRow,
};
use crate::vault::{self, Opened};

use super::natural;
use super::trash;

/// The batch limits of a push (section 6.5).
pub const MAX_PUSH_ITEMS: usize = 500;
/// Envelope bytes per push (base64 grows them by a third; the body limit is
/// 16 MiB).
pub const MAX_PUSH_ENVELOPE_BYTES: usize = 11 * 1024 * 1024;

/// What the personal vault syncs on this device (D15; data-model 17).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Selection {
    /// Classes pulled and pushed (class 1 always).
    pub classes: Vec<u8>,
    /// Type names never applied nor pushed although their class syncs (one
    /// of the two `appearance` toggles off).
    pub skip_types: Vec<String>,
}

impl Selection {
    /// Everything (tests, team vaults).
    pub fn all() -> Self {
        Self {
            classes: shellpp_sync_api::classes::ALL.to_vec(),
            skip_types: Vec::new(),
        }
    }

    pub fn class(&self, class: u8) -> bool {
        class == shellpp_sync_api::classes::VAULT || self.classes.contains(&class)
    }

    fn type_name(&self, name: &str) -> bool {
        // The last session is device-local whatever the toggles (16.3).
        name != "last_session" && !self.skip_types.iter().any(|skip| skip == name)
    }
}

/// The effects of an engine step, applied to the index after the commit.
#[derive(Debug, Default)]
pub struct Effects {
    puts: Vec<RecordRow>,
    index_insert: Vec<(Uuid, RecordKind, u64, u32, Payload)>,
    index_remove: Vec<Uuid>,
    trash_put: Vec<RecentlyDeletedRow>,
    trash_remove: Vec<Uuid>,
    duplicates: Vec<DuplicatePairRow>,
    /// Records changed here (for `records:changed`).
    pub changed: Vec<Uuid>,
    pub classes: Vec<u8>,
    /// Records refused as a server or storage fault (section 6.4, step 4).
    pub integrity_warnings: usize,
    /// Conflict copies created.
    pub conflicts: usize,
    /// A record of an epoch newer than this device holds arrived: the
    /// device must sign in again to learn the key (section 5.1).
    pub newer_epoch: bool,
    /// Items the server refused that must not be retried automatically,
    /// with their status (section 6.5).
    pub failed: Vec<(Uuid, String)>,
    /// Items that need another push round in this cycle.
    pub repush: bool,
    /// Items the server answered `stale_key_epoch` for: the keys, then the
    /// push again (section 6.5); they stay pending.
    pub stale_key_epoch: Vec<Uuid>,
    /// A deletion was accepted: items refused for the quota go again
    /// (section 6.5).
    pub pushed_deletion: bool,
}

impl Effects {
    fn put(&mut self, row: RecordRow) {
        if !self.classes.contains(&row.class) {
            self.classes.push(row.class);
        }
        self.changed.push(row.record_id);
        self.puts.retain(|other| other.record_id != row.record_id);
        self.puts.push(row);
    }

    fn show(&mut self, row: &RecordRow, kind: RecordKind, payload: Payload) {
        self.index_remove.retain(|id| *id != row.record_id);
        self.index_insert.retain(|entry| entry.0 != row.record_id);
        self.index_insert
            .push((row.record_id, kind, row.revision, row.key_epoch, payload));
        self.trash_remove.push(row.record_id);
    }

    fn hide(&mut self, record_id: Uuid) {
        self.index_insert.retain(|entry| entry.0 != record_id);
        self.index_remove.push(record_id);
    }

    /// Writes the rows, the trash and the duplicate pairs, and `state` when
    /// given, in one transaction; then updates the index.
    pub fn commit(
        mut self,
        store: &mut Store,
        session: &mut Session,
        state: Option<&SyncStateRow>,
    ) -> Result<Effects, CoreError> {
        let vault_id = session.ring.vault_id();
        store.write(|tx| {
            for row in &self.puts {
                tx.record_put(row)?;
            }
            for id in &self.trash_remove {
                tx.recently_deleted_delete(&vault_id, id)?;
            }
            for entry in &self.trash_put {
                tx.recently_deleted_put(entry)?;
            }
            for id in &self.index_remove {
                tx.duplicate_pairs_forget(&vault_id, id)?;
            }
            for pair in &self.duplicates {
                tx.duplicate_pair_put(pair)?;
            }
            if let Some(state) = state {
                tx.sync_state_put(state)?;
            }
            Ok(())
        })?;
        for id in self.index_remove.drain(..) {
            session.index.remove(&id);
        }
        for (id, kind, revision, epoch, payload) in self.index_insert.drain(..) {
            session
                .index
                .insert(IndexedRecord::new(id, kind, revision, epoch, payload));
        }
        self.puts.clear();
        Ok(self)
    }
}

/// A row for an envelope as the server holds it (synced at its revision).
fn row_from_envelope(
    vault_id: Uuid,
    bytes: &[u8],
    seq: Option<u64>,
    now_text: &str,
) -> Option<RecordRow> {
    let parts = envelope::split(bytes).ok()?;
    let header = parts.header;
    if header.vault_id != *vault_id.as_bytes() {
        return None;
    }
    Some(RecordRow {
        vault_id,
        record_id: Uuid::from_bytes(header.record_id),
        class: header.class,
        revision: header.revision,
        synced_revision: header.revision,
        key_epoch: header.key_epoch,
        tombstone: header.tombstone,
        tombstoned_at: header.tombstone.then(|| now_text.to_owned()),
        server_seq: seq.map(|seq| seq as i64),
        envelope: bytes.to_vec(),
        base_envelope: None,
    })
}

/// The payload of any version, live or tombstone, of a known or unknown
/// type; `None` when it does not open.
fn payload_of(session: &mut Session, row: &RecordRow) -> Option<Payload> {
    let record_key = session.ring.record_key(row.key_epoch)?;
    let (_, bytes) = crate::crypto::open_envelope(record_key, &row.envelope).ok()?;
    Payload::from_json_bytes(&bytes).ok()
}

/// The epoch of the version this device last synced for a record: the
/// local envelope when synced, the base envelope when pending.
fn synced_epoch(local: &RecordRow) -> Option<u32> {
    if local.revision == local.synced_revision {
        return Some(local.key_epoch);
    }
    let base = local.base_envelope.as_ref()?;
    envelope::split(base)
        .ok()
        .map(|parts| parts.header.key_epoch)
}

/// `data` equality after dropping `null` members (data-model 1.2).
pub fn data_equal(a: Option<&Map<String, Value>>, b: Option<&Map<String, Value>>) -> bool {
    fn strip(value: &Value) -> Value {
        match value {
            Value::Object(map) => Value::Object(
                map.iter()
                    .filter(|(_, v)| !v.is_null())
                    .map(|(k, v)| (k.clone(), strip(v)))
                    .collect(),
            ),
            Value::Array(items) => Value::Array(items.iter().map(strip).collect()),
            other => other.clone(),
        }
    }
    let a = a.map(|map| strip(&Value::Object(map.clone())));
    let b = b.map(|map| strip(&Value::Object(map.clone())));
    a == b
}

fn stamp(payload: &Payload) -> (Hlc, [u8; 16]) {
    payload.stamp()
}

/// Type rule of D93 under rule 5: `Some(true)` when the local version wins
/// whatever the stamps, `Some(false)` when the server's does.
fn type_rule(local: &Payload, server: &Payload) -> Option<bool> {
    let revoked = |p: &Payload| {
        p.data
            .as_ref()
            .and_then(|data| data.get("marker"))
            .and_then(Value::as_str)
            == Some("revoked")
    };
    if local.type_name == "known_host" && server.type_name == "known_host" {
        match (revoked(local), revoked(server)) {
            (true, false) => return Some(true),
            (false, true) => return Some(false),
            _ => {}
        }
    }
    if local.type_name == "member_pin" && server.type_name == "member_pin" {
        let key = |p: &Payload| {
            p.data
                .as_ref()
                .and_then(|data| data.get("x25519_public_key"))
                .cloned()
        };
        let verified = |p: &Payload| {
            p.data
                .as_ref()
                .and_then(|data| data.get("verified"))
                .and_then(Value::as_bool)
                == Some(true)
        };
        if key(local) == key(server) {
            match (verified(local), verified(server)) {
                (true, false) => return Some(true),
                (false, true) => return Some(false),
                _ => {}
            }
        }
    }
    None
}

/// The data of a conflict copy (data-model 15).
pub(crate) fn conflict_data(original: &RecordRow, losing: &Payload) -> Map<String, Value> {
    let mut data = Map::new();
    data.insert(
        "original_record_id".into(),
        Value::String(original.record_id.to_string()),
    );
    data.insert(
        "original_type".into(),
        Value::String(losing.type_name.clone()),
    );
    data.insert("original_revision".into(), Value::from(original.revision));
    data.insert("losing_hlc".into(), Value::String(losing.hlc.to_text()));
    data.insert(
        "losing_device_id".into(),
        Value::String(losing.device_id.to_string()),
    );
    data.insert("losing_deleted".into(), Value::Bool(losing.deleted));
    if let Some(losing_data) = &losing.data {
        data.insert("losing_data".into(), Value::Object(losing_data.clone()));
    }
    data
}

/// Shows or hides a version in the index (and the trash) as it becomes the
/// local one.
fn adopt_version(
    effects: &mut Effects,
    previous_live: Option<(RecordKind, &Payload)>,
    row: &RecordRow,
    opened: Opened,
    now_text: &str,
    previous_envelope: Option<&[u8]>,
    selection: &Selection,
) {
    match opened {
        Opened::Live(kind, payload) => {
            if selection.type_name(kind.name()) {
                effects.show(row, kind, payload);
            }
        }
        Opened::Tombstone(_) => {
            effects.hide(row.record_id);
            if let (Some((kind, _)), Some(envelope)) = (previous_live, previous_envelope)
                && trash::kept(kind)
            {
                effects.trash_put.push(RecentlyDeletedRow {
                    vault_id: row.vault_id,
                    record_id: row.record_id,
                    class: row.class,
                    deleted_at: now_text.to_owned(),
                    origin: DeletedOrigin::Sync,
                    envelope: envelope.to_vec(),
                });
            }
        }
        Opened::Unsupported | Opened::Damaged(_) => effects.hide(row.record_id),
    }
}

/// The stamp of the version this device last synced for `local`: its own
/// envelope when synced, its base envelope when pending (`None` when it was
/// never synced or does not open).
fn synced_stamp(
    session: &mut Session,
    local: &RecordRow,
    now_text: &str,
) -> Option<(Hlc, [u8; 16])> {
    if local.revision == local.synced_revision {
        return payload_of(session, local).map(|payload| stamp(&payload));
    }
    let base = local
        .base_envelope
        .as_ref()
        .and_then(|bytes| row_from_envelope(local.vault_id, bytes, None, now_text))?;
    payload_of(session, &base).map(|payload| stamp(&payload))
}

/// D184 (rule 0 of section 6.7; section 6.9): a server version below the
/// revision this device synced but written after the version it synced
/// (its stamp is greater) is an edit made on a server restored from an
/// older backup, which this device's newer version goes over. It never
/// wins, and it is never lost either: it is kept as a conflict copy, pushed
/// and synced like every copy. A stale version whose stamp is not greater
/// (a version this device superseded, a replay) adds nothing and gets no
/// copy, nor does one equal to the local version or a conflict copy.
#[allow(clippy::too_many_arguments)]
fn keep_overwritten(
    session: &mut Session,
    effects: &mut Effects,
    local: &RecordRow,
    local_payload: &Payload,
    synced: Option<(Hlc, [u8; 16])>,
    server_row: &RecordRow,
    server_payload: &Payload,
    now: SystemTime,
) -> Result<(), CoreError> {
    if local_payload.type_name == "conflict" || server_payload.type_name == "conflict" {
        return Ok(());
    }
    if local_payload.deleted == server_payload.deleted
        && data_equal(local_payload.data.as_ref(), server_payload.data.as_ref())
    {
        return Ok(());
    }
    if synced.is_some_and(|synced| stamp(server_payload) <= synced) {
        return Ok(());
    }
    let now_text = format_timestamp(now);
    let hlc = session.hlc.tick(crate::model::time::unix_ms(now));
    let payload = Payload::live(
        "conflict",
        1,
        hlc,
        session.device_id,
        conflict_data(server_row, server_payload),
    );
    let row = vault::seal_row(
        &mut session.ring,
        local.class,
        Uuid::now_v7(),
        &payload,
        None,
        &now_text,
    )?;
    effects.put(row);
    effects.conflicts += 1;
    effects.repush = true;
    Ok(())
}

/// A conflict copy of `losing` (by its record, stamp and deletion) is held
/// here already: a repeated resync or restore, or a replay, makes none.
fn has_copy(
    store: &Store,
    session: &mut Session,
    local: &RecordRow,
    losing: &Payload,
) -> Result<bool, CoreError> {
    let original = local.record_id.to_string();
    let hlc = losing.hlc.to_text();
    let device = losing.device_id.to_string();
    for row in store.records(&local.vault_id, Some(&[local.class]))? {
        if row.tombstone || session.index.get(&row.record_id).is_some() {
            continue;
        }
        let Some(payload) = payload_of(session, &row) else {
            continue;
        };
        if payload.type_name != "conflict" {
            continue;
        }
        let field = |name: &str| {
            payload
                .data
                .as_ref()
                .and_then(|data| data.get(name))
                .and_then(Value::as_str)
                .map(str::to_owned)
        };
        if field("original_record_id").as_deref() == Some(original.as_str())
            && field("losing_hlc").as_deref() == Some(hlc.as_str())
            && field("losing_device_id").as_deref() == Some(device.as_str())
        {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Resolves a conflict between the local pending version and the server's
/// (section 6.7). `server` is `None` when the record is absent there.
/// `no_copy` suppresses the copy of rule 5 (a server restore's stale copy).
#[allow(clippy::too_many_arguments)]
pub fn resolve(
    store: &Store,
    session: &mut Session,
    effects: &mut Effects,
    local: &RecordRow,
    server: Option<&RecordRow>,
    selection: &Selection,
    no_copy: bool,
    now: SystemTime,
) -> Result<(), CoreError> {
    let now_text = format_timestamp(now);
    let Some(server) = server else {
        // Rule 1: absent on the server.
        let mut row = local.clone();
        if local.tombstone {
            row.synced_revision = row.revision;
            row.base_envelope = None;
        } else {
            row.synced_revision = 0;
            row.base_envelope = None;
            row.server_seq = None;
            effects.repush = true;
        }
        effects.put(row);
        return Ok(());
    };
    let (Some(l), Some(s)) = (payload_of(session, local), payload_of(session, server)) else {
        // A version that does not open: keep the local one.
        effects.integrity_warnings += 1;
        return Ok(());
    };
    let base_row = local
        .base_envelope
        .as_ref()
        .and_then(|bytes| row_from_envelope(local.vault_id, bytes, None, &now_text));
    let base_revision = base_row.as_ref().map_or(0, |row| row.revision);
    let base = base_row
        .as_ref()
        .and_then(|row| payload_of(session, row))
        .map(|payload| stamp(&payload));
    // A server version older than the one this device synced (a record
    // pushed back by a resync or a restore keeps it as its base) is stale:
    // it never replaces the local version, whatever rules 2-5 would say
    // (principle 3; section 6.9, step 3: no conflict copy for it).
    // Another version at the revision of the base (other bytes) is a fork
    // of the server's history, as a record a restore or a resync marked
    // meets it (synced below its base): the local version wins as over a
    // stale one, and the server's is kept as a copy whatever its stamp
    // (D184, as in the pull).
    let fork = base_row
        .as_ref()
        .is_some_and(|base| base.revision == server.revision && base.envelope != server.envelope);
    if fork || server.revision < local.synced_revision.max(base_revision) {
        // A later server sequence: written after a restore (D198), or a
        // record pushed back after a purge; either way the copy costs
        // nothing, as the local version wins anyway.
        let synced = if fork || later_seq(server, local) {
            None
        } else {
            base
        };
        if !has_copy(store, session, local, &s)? {
            keep_overwritten(session, effects, local, &l, synced, server, &s, now)?;
        }
        let previous = RecordRow {
            revision: local.revision.max(server.revision),
            synced_revision: server.revision,
            server_seq: server.server_seq,
            envelope: server.envelope.clone(),
            base_envelope: Some(server.envelope.clone()),
            ..local.clone()
        };
        let row = vault::seal_row(
            &mut session.ring,
            local.class,
            local.record_id,
            &l,
            Some(&previous),
            &now_text,
        )?;
        effects.put(row);
        effects.repush = true;
        return Ok(());
    }
    let local_live = (!l.deleted)
        .then(|| RecordKind::from_name(&l.type_name))
        .flatten();
    let adopt_server = |session: &mut Session, effects: &mut Effects| {
        let opened = vault::open_row(&mut session.ring, server);
        adopt_version(
            effects,
            local_live.map(|kind| (kind, &l)),
            server,
            opened,
            &now_text,
            Some(&local.envelope),
            selection,
        );
        effects.put(server.clone());
    };
    // Rule 2: the same content.
    if l.deleted == s.deleted && data_equal(l.data.as_ref(), s.data.as_ref()) {
        adopt_server(session, effects);
        return Ok(());
    }
    let local_wins = if base == Some(stamp(&s)) {
        true // Rule 3: S only re-encrypted the base.
    } else if base == Some(stamp(&l)) {
        false // Rule 4: L only re-encrypted the base.
    } else {
        // Rule 5, with the type rules of D93.
        let wins = type_rule(&l, &s).unwrap_or(stamp(&l) > stamp(&s));
        let is_copy = l.type_name == "conflict" || s.type_name == "conflict";
        if !no_copy && !is_copy {
            let loser = if wins { &s } else { &l };
            let loser_row = if wins { server } else { local };
            let copy_id = Uuid::now_v7();
            let hlc = session.hlc.tick(crate::model::time::unix_ms(now));
            let payload = Payload::live(
                "conflict",
                1,
                hlc,
                session.device_id,
                conflict_data(loser_row, loser),
            );
            let row = vault::seal_row(
                &mut session.ring,
                local.class,
                copy_id,
                &payload,
                None,
                &now_text,
            )?;
            effects.put(row);
            effects.conflicts += 1;
            effects.repush = true;
        }
        wins
    };
    if local_wins {
        // Re-seal L above both revisions; S becomes its base.
        let previous = RecordRow {
            revision: local.revision.max(server.revision),
            synced_revision: server.revision,
            server_seq: server.server_seq,
            envelope: server.envelope.clone(),
            base_envelope: Some(server.envelope.clone()),
            ..local.clone()
        };
        let row = vault::seal_row(
            &mut session.ring,
            local.class,
            local.record_id,
            &l,
            Some(&previous),
            &now_text,
        )?;
        effects.put(row);
        effects.repush = true;
    } else {
        adopt_server(session, effects);
    }
    Ok(())
}

/// What a pull page left.
#[derive(Debug, Default)]
pub struct PageResult {
    pub effects: Effects,
    /// Every record id of the page (a full resync collects them), with the
    /// revision the server holds.
    pub received: Vec<(Uuid, u64)>,
    /// The page stopped before its end (a newer epoch): the cursor stays.
    pub stopped: bool,
}

/// Applies one pull page (section 6.4) and stores it with `cursor`.
#[allow(clippy::too_many_arguments)]
pub fn apply_page(
    store: &mut Store,
    session: &mut Session,
    records: &[SeqEnvelope],
    next_cursor: u64,
    state: &SyncStateRow,
    selection: &Selection,
    no_copy_below: &HashMap<Uuid, u64>,
    refuse_newer: bool,
    now: SystemTime,
) -> Result<PageResult, CoreError> {
    let vault_id = session.ring.vault_id();
    let now_text = format_timestamp(now);
    let mut result = PageResult::default();
    // New live records, checked for duplicates once the page is read.
    let mut inserted: Vec<(RecordKind, Uuid)> = Vec::new();
    for record in records {
        let bytes = record.envelope.as_bytes();
        // Step 1: the vault, and an id that belongs to no other vault here.
        let Some(row) = row_from_envelope(vault_id, bytes, Some(record.seq), &now_text) else {
            result.effects.integrity_warnings += 1;
            continue;
        };
        if let Some(other) = store.record_anywhere(&row.record_id)?
            && other.vault_id != vault_id
        {
            result.effects.integrity_warnings += 1;
            continue;
        }
        // Step 2: an epoch this device does not hold yet. After the user's
        // "Reset sync state" (section 6.9, a header epoch the server does
        // not hold), it is refused like a failure of steps 1-3: never
        // accepted, the cursor advances.
        if row.key_epoch > session.ring.current_epoch() {
            if refuse_newer {
                result.effects.integrity_warnings += 1;
                continue;
            }
            result.effects.newer_epoch = true;
            result.stopped = true;
            break;
        }
        // Step 3: open it (AAD, padding, payload, class, tombstone flag).
        let opened = vault::open_row(&mut session.ring, &row);
        if let Opened::Damaged(_) = opened {
            result.effects.integrity_warnings += 1;
            continue;
        }
        // Only authenticated versions count as held by the server (a full
        // resync and a restore build on this list).
        result.received.push((row.record_id, row.revision));
        let type_name = match &opened {
            Opened::Live(kind, _) => Some(kind.name().to_owned()),
            Opened::Tombstone(payload) => Some(payload.type_name.clone()),
            _ => payload_of(session, &row).map(|payload| payload.type_name.clone()),
        };
        if type_name
            .as_deref()
            .is_some_and(|name| !selection.type_name(name))
        {
            continue;
        }
        // Step 5: merge the clock.
        if let Some(payload) = payload_of(session, &row) {
            session
                .hlc
                .observe(payload.hlc, crate::model::time::unix_ms(now));
        }
        let local = store.record(&vault_id, &row.record_id)?;
        let Some(local) = local else {
            // A new record: insert.
            if let Opened::Live(kind, payload) = &opened
                && selection.type_name(kind.name())
                && payload.data.is_some()
            {
                inserted.push((*kind, row.record_id));
            }
            adopt_version(
                &mut result.effects,
                None,
                &row,
                opened,
                &now_text,
                None,
                selection,
            );
            result.effects.put(row);
            continue;
        };
        if row.revision <= local.synced_revision {
            // An echo, a duplicate or a server rollback of this record. A
            // version below the synced one written after it (an edit on a
            // restored server) is kept as a conflict copy (D184): the
            // restore's mark pushes the local version over it.
            // Another version at the synced revision is a fork of the
            // server's history (a restore): this device's version goes over
            // it as over a stale one (rule 0), and it is kept as a copy.
            // The version this device holds at the synced revision: its own
            // when nothing is pending, else its base, but only when the base
            // is at that revision (a record a restore marked keeps a newer
            // base, which says nothing about the server's version there;
            // review of run 10).
            let synced_envelope = if local.revision == local.synced_revision {
                Some(&local.envelope)
            } else {
                local.base_envelope.as_ref().filter(|bytes| {
                    row_from_envelope(vault_id, bytes, None, &now_text)
                        .is_some_and(|base| base.revision == local.synced_revision)
                })
            };
            let fork = row.revision == local.synced_revision
                && synced_envelope.is_some_and(|envelope| *envelope != row.envelope);
            // Below the epoch this device synced (D93): refused, no copy.
            let old_epoch = synced_epoch(&local).is_some_and(|epoch| row.key_epoch < epoch);
            if (row.revision < local.synced_revision || fork) && old_epoch {
                result.effects.integrity_warnings += 1;
                continue;
            }
            if (row.revision < local.synced_revision || fork)
                && let (Some(server_payload), Some(local_payload)) =
                    (payload_of(session, &row), payload_of(session, &local))
            {
                // A version at the synced revision is never one this device
                // made (a write always raises the revision), so a fork is
                // kept whatever its stamp. In a server restore, so is a
                // version whose server sequence is above that of the version
                // this device synced: written after the restore, even by a
                // device whose clock is behind (D198; D184, residual 1).
                // Only there: a record pushed back after its tombstone was
                // purged also has a lower revision at a higher sequence.
                let later_seq = !no_copy_below.is_empty() && later_seq(&row, &local);
                let synced = if fork || later_seq {
                    None
                } else {
                    synced_stamp(session, &local, &now_text)
                };
                let copies = result.effects.conflicts;
                if !has_copy(store, session, &local, &server_payload)? {
                    keep_overwritten(
                        session,
                        &mut result.effects,
                        &local,
                        &local_payload,
                        synced,
                        &row,
                        &server_payload,
                        now,
                    )?;
                }
                // The local version goes over the server's, as rule 0 has
                // it: at a fork, and below the synced revision when that
                // version was an edit made after this device's (kept as a
                // copy just now), also outside a resync (cross review of
                // run 9). A replay of a version this device superseded
                // changes nothing.
                if fork || result.effects.conflicts > copies {
                    let previous = RecordRow {
                        revision: local.revision.max(row.revision),
                        synced_revision: row.revision,
                        server_seq: row.server_seq,
                        envelope: row.envelope.clone(),
                        base_envelope: Some(row.envelope.clone()),
                        ..local.clone()
                    };
                    let resealed = vault::seal_row(
                        &mut session.ring,
                        local.class,
                        local.record_id,
                        &local_payload,
                        Some(&previous),
                        &now_text,
                    )?;
                    result.effects.put(resealed);
                    result.effects.repush = true;
                }
            }
            continue;
        }
        // D93: a version below the key epoch this device synced is refused.
        // Records pushed back by a resync or a restore (synced revision 0)
        // keep their synced envelope as the base, so the check covers them.
        if let Some(epoch) = synced_epoch(&local)
            && row.key_epoch < epoch
        {
            result.effects.integrity_warnings += 1;
            continue;
        }
        let pending = local.revision != local.synced_revision;
        // In a server restore (section 6.9; `no_copy_below` holds the
        // synced revisions of before): a newer server version replaces a
        // version this device synced with the server of before, which the
        // restored server may never have had (lost with the writes after
        // its backup). That version is kept as a conflict copy, so the
        // replace loses nothing (cross review of run 9, D184).
        if !pending
            && no_copy_below
                .get(&row.record_id)
                .is_some_and(|held| *held > 0)
            && let (Some(server_payload), Some(local_payload)) =
                (payload_of(session, &row), payload_of(session, &local))
            && !has_copy(store, session, &local, &local_payload)?
        {
            keep_overwritten(
                session,
                &mut result.effects,
                &row,
                &server_payload,
                None,
                &local,
                &local_payload,
                now,
            )?;
        }
        if !pending {
            let previous_live = if local.tombstone {
                None
            } else {
                match vault::open_row(&mut session.ring, &local) {
                    Opened::Live(kind, payload) => Some((kind, payload)),
                    _ => None,
                }
            };
            adopt_version(
                &mut result.effects,
                previous_live
                    .as_ref()
                    .map(|(kind, payload)| (*kind, payload)),
                &row,
                opened,
                &now_text,
                Some(&local.envelope),
                selection,
            );
            result.effects.put(row);
        } else {
            let no_copy = no_copy_below
                .get(&row.record_id)
                .is_some_and(|held| row.revision < *held);
            resolve(
                store,
                session,
                &mut result.effects,
                &local,
                Some(&row),
                selection,
                no_copy,
                now,
            )?;
        }
    }
    let mut state = state.clone();
    if !result.stopped {
        state.cursor = next_cursor;
    }
    let effects = std::mem::take(&mut result.effects);
    result.effects = effects.commit(store, session, Some(&state))?;
    // The pairs are raised against the index with the page in it, so a
    // parent that came in the same page counts (its natural key, a host's
    // inherited port). They are hints the scan raises again (UI 13.7).
    let pairs = duplicates(session, &inserted, vault_id, &now_text);
    if !pairs.is_empty() {
        store.write(|tx| {
            for pair in &pairs {
                tx.duplicate_pair_put(pair)?;
            }
            Ok(())
        })?;
    }
    Ok(result)
}

/// Raises a duplicate pair when a pulled insert has the natural key of a
/// live record of the same type (data-model 18.3; not history or logs).
/// The keys are those of data-model 18.6 in this vault (parents by their
/// own natural keys, a host's effective port and username), built once per
/// page and kind after the page is stored, so a first pull of thousands of
/// records stays linear. A record the page also removed again is skipped.
fn duplicates(
    session: &Session,
    inserted: &[(RecordKind, Uuid)],
    vault_id: Uuid,
    now_text: &str,
) -> Vec<DuplicatePairRow> {
    let mut pairs = Vec::new();
    if inserted.is_empty() {
        return pairs;
    }
    let keys = natural::Keys::new(&session.index);
    let mut cache: HashMap<RecordKind, HashMap<String, Vec<Uuid>>> = HashMap::new();
    for (kind, record_id) in inserted {
        let Some(record) = session.index.get_kind(record_id, *kind) else {
            continue;
        };
        let Some(key) = keys.key_of(record) else {
            continue;
        };
        let by_key = cache.entry(*kind).or_insert_with(|| {
            let mut map: HashMap<String, Vec<Uuid>> = HashMap::new();
            for other in session.index.of_kind(*kind) {
                if let Some(key) = keys.key_of(other) {
                    map.entry(key).or_default().push(other.record_id);
                }
            }
            map
        });
        for other in by_key.get(&key).into_iter().flatten() {
            // Pairs among the page's own records are raised once, from the
            // later one.
            let earlier = inserted.iter().position(|entry| entry.1 == *other);
            let this = inserted.iter().position(|entry| entry.1 == *record_id);
            if other != record_id && earlier.is_none_or(|earlier| Some(earlier) < this) {
                pairs.push(DuplicatePairRow {
                    vault_id,
                    record_id: *other,
                    other_record_id: *record_id,
                    found_at: now_text.to_owned(),
                    kept: false,
                });
            }
        }
    }
    pairs
}

/// A push batch: the items and the rows they were built from.
#[derive(Debug, Default)]
pub struct PushBatch {
    pub items: Vec<RecordWrite>,
    pub sent: Vec<RecordRow>,
    /// Pending rows left for the next batch.
    pub more: bool,
}

/// Builds the next push batch of the vault (section 6.5): pending rows of
/// the selection, re-sealed under the current epoch first where needed; a
/// tombstone that was never pushed is marked synced instead. `skip`: items
/// the server refused, not retried automatically.
pub fn build_push(
    store: &mut Store,
    session: &mut Session,
    selection: &Selection,
    skip: &HashSet<Uuid>,
    now: SystemTime,
) -> Result<PushBatch, CoreError> {
    let vault_id = session.ring.vault_id();
    let now_text = format_timestamp(now);
    let current = session.ring.current_epoch();
    let mut local_only = Effects::default();
    let mut batch = PushBatch::default();
    let mut bytes = 0usize;
    for row in store.records_pending(&vault_id)? {
        if !selection.class(row.class) || skip.contains(&row.record_id) {
            continue;
        }
        let payload = payload_of(session, &row);
        if payload
            .as_ref()
            .is_some_and(|payload| !selection.type_name(&payload.type_name))
        {
            continue;
        }
        if row.tombstone && row.synced_revision == 0 {
            let mut done = row.clone();
            done.synced_revision = done.revision;
            local_only.put(done);
            continue;
        }
        let row = if row.key_epoch < current {
            let Some(payload) = payload else {
                continue;
            };
            let resealed = vault::seal_row(
                &mut session.ring,
                row.class,
                row.record_id,
                &payload,
                Some(&row),
                &now_text,
            )?;
            local_only.put(resealed.clone());
            resealed
        } else {
            row
        };
        if batch.items.len() >= MAX_PUSH_ITEMS
            || (bytes + row.envelope.len() > MAX_PUSH_ENVELOPE_BYTES && !batch.items.is_empty())
        {
            batch.more = true;
            break;
        }
        bytes += row.envelope.len();
        batch.items.push(RecordWrite {
            base_revision: row.synced_revision,
            envelope: B64(row.envelope.clone()),
        });
        batch.sent.push(row);
    }
    // Re-sealed rows keep their index entries' payloads; only revisions move.
    let resealed: Vec<(Uuid, u64, u32)> = local_only
        .puts
        .iter()
        .map(|row| (row.record_id, row.revision, row.key_epoch))
        .collect();
    local_only.commit(store, session, None)?;
    for (id, revision, epoch) in resealed {
        if let Some(mut record) = session.index.remove(&id) {
            record.revision = revision;
            record.key_epoch = epoch;
            session.index.insert(record);
        }
    }
    Ok(batch)
}

/// Handles the results of a push (section 6.5).
pub fn apply_push_results(
    store: &mut Store,
    session: &mut Session,
    sent: &[RecordRow],
    results: &[PushResult],
    selection: &Selection,
    now: SystemTime,
) -> Result<Effects, CoreError> {
    let vault_id = session.ring.vault_id();
    let now_text = format_timestamp(now);
    let mut effects = Effects::default();
    for result in results {
        let Some(pushed) = sent.iter().find(|row| row.record_id == result.record_id) else {
            continue;
        };
        let Some(local) = store.record(&vault_id, &pushed.record_id)? else {
            continue;
        };
        match result.status {
            ItemStatus::Ok => {
                let seq = result.seq;
                effects.pushed_deletion |= pushed.tombstone;
                let mut row = local.clone();
                if local.revision == pushed.revision {
                    row.synced_revision = row.revision;
                    row.base_envelope = None;
                } else {
                    // Changed meanwhile: still pending, on top of the pushed version.
                    row.synced_revision = pushed.revision;
                    row.base_envelope = Some(pushed.envelope.clone());
                }
                row.server_seq = seq.map(|seq| seq as i64);
                effects.put(row);
            }
            ItemStatus::Conflict => {
                let current = match &result.current {
                    Some(Some(current)) => {
                        match row_from_envelope(
                            vault_id,
                            current.envelope.as_bytes(),
                            Some(current.seq),
                            &now_text,
                        ) {
                            Some(row) if row.record_id == local.record_id => Some(row),
                            _ => {
                                effects.integrity_warnings += 1;
                                effects.failed.push((local.record_id, "integrity".into()));
                                continue;
                            }
                        }
                    }
                    Some(None) => None,
                    // A `conflict` without `current` is not what section 6.5
                    // defines: nothing is assumed about the server's copy.
                    None => {
                        effects.integrity_warnings += 1;
                        effects.failed.push((local.record_id, "integrity".into()));
                        continue;
                    }
                };
                // D93: a `current` below the epoch this device synced loses.
                if let (Some(current), Some(epoch)) = (&current, synced_epoch(&local))
                    && current.key_epoch < epoch
                {
                    effects.integrity_warnings += 1;
                    effects.failed.push((local.record_id, "integrity".into()));
                    continue;
                }
                // A `current` above this device's epoch: the cycle pushes
                // only after a pull whose `current_epoch` is not above it,
                // so the server holds no such key (D183): refused like a
                // damaged version, not retried automatically.
                if current
                    .as_ref()
                    .is_some_and(|row| row.key_epoch > session.ring.current_epoch())
                {
                    effects.integrity_warnings += 1;
                    effects.failed.push((local.record_id, "integrity".into()));
                    continue;
                }
                // A `current` that does not open is refused like a damaged
                // version (section 6.5), not met again in every batch.
                if current
                    .as_ref()
                    .is_some_and(|row| payload_of(session, row).is_none())
                {
                    effects.integrity_warnings += 1;
                    effects.failed.push((local.record_id, "integrity".into()));
                    continue;
                }
                resolve(
                    store,
                    session,
                    &mut effects,
                    &local,
                    current.as_ref(),
                    selection,
                    false,
                    now,
                )?;
            }
            ItemStatus::StaleKeyEpoch => {
                // Not refused for good: the keys, then the push again.
                effects.stale_key_epoch.push(local.record_id);
            }
            ItemStatus::QuotaExceeded => {
                effects
                    .failed
                    .push((local.record_id, "quota_exceeded".into()));
            }
            ItemStatus::InvalidEnvelope => {
                effects
                    .failed
                    .push((local.record_id, "invalid_envelope".into()));
            }
            ItemStatus::InsufficientRole => {
                effects
                    .failed
                    .push((local.record_id, "insufficient_role".into()));
            }
            ItemStatus::ClassNotAllowed => {
                effects
                    .failed
                    .push((local.record_id, "class_not_allowed".into()));
            }
            ItemStatus::PayloadTooLarge => {
                effects
                    .failed
                    .push((local.record_id, "payload_too_large".into()));
            }
            ItemStatus::Unknown => {}
        }
    }
    effects.commit(store, session, None)
}

/// After a full resync (section 6.9, steps 2 and 5): synced live records of
/// the covered classes the snapshot lacked are kept and become pending with
/// `base_revision = 0`; their synced envelope stays as the base. The
/// deletable ones are listed in "Brought back" when `list` (a resync, not a
/// server restore). Returns the ids pushed back.
pub fn push_back_missing(
    store: &mut Store,
    session: &mut Session,
    received: &HashSet<Uuid>,
    classes: &[u8],
    list: bool,
    now: SystemTime,
) -> Result<Vec<Uuid>, CoreError> {
    let vault_id = session.ring.vault_id();
    let now_text = format_timestamp(now);
    let mut rows = Vec::new();
    let mut listed = Vec::new();
    for row in store.records(&vault_id, Some(classes))? {
        if received.contains(&row.record_id)
            || row.tombstone
            || row.synced_revision == 0
            || row.revision != row.synced_revision
        {
            continue;
        }
        let type_name = payload_of(session, &row).map(|payload| payload.type_name.clone());
        if type_name.as_deref() == Some("last_session") {
            continue;
        }
        let mut back = row.clone();
        back.base_envelope = Some(row.envelope.clone());
        back.synced_revision = 0;
        back.server_seq = None;
        if list && type_name.as_deref().is_some_and(trash::deletable_type) {
            listed.push(row.record_id);
        }
        rows.push(back);
    }
    let ids: Vec<Uuid> = rows.iter().map(|row| row.record_id).collect();
    store.write(|tx| {
        for row in &rows {
            tx.record_put(row)?;
        }
        for id in &listed {
            tx.resync_returned_put(&vault_id, id, &now_text)?;
        }
        Ok(())
    })?;
    Ok(ids)
}

/// `true` when the server gave `server` a sequence above the one of the
/// version `local` synced (D198).
fn later_seq(server: &RecordRow, local: &RecordRow) -> bool {
    matches!(
        (server.server_seq, local.server_seq),
        (Some(server), Some(synced)) if server > synced
    )
}

/// The server restore (section 6.9, step 2): every local record's synced
/// revision becomes the server's (or 0 when it has none), so whatever is
/// newer here, or missing there, is pushed again. Returns, per record, the
/// synced revision held before, for step 3. With `list`, the deletable live
/// records the server lacks go to "Brought back" (a rebind that is not sure
/// a restore happened, D197: they may be deletions whose tombstones were
/// purged).
///
/// A synced revision is only ever lowered: a server revision above it is a
/// version the pull received but did not apply (a type that is off, a
/// refused key epoch, a payload that did not open), and marking it would
/// take a pending local edit out of the push (review finding; section 6.10).
pub fn restore_mark(
    store: &mut Store,
    session: &mut Session,
    server: &HashMap<Uuid, u64>,
    classes: &[u8],
    list: bool,
    now: SystemTime,
) -> Result<HashMap<Uuid, u64>, CoreError> {
    let vault_id = session.ring.vault_id();
    let now_text = format_timestamp(now);
    let mut held = HashMap::new();
    let mut rows = Vec::new();
    let mut listed = Vec::new();
    for row in store.records(&vault_id, Some(classes))? {
        let server_revision = server.get(&row.record_id).copied().unwrap_or(0);
        if row.synced_revision <= server_revision {
            continue;
        }
        held.insert(row.record_id, row.synced_revision);
        let mut changed = row.clone();
        if row.tombstone && server_revision == 0 {
            // A deletion the server lost: nothing to push back.
            continue;
        }
        if row.revision == row.synced_revision && server_revision < row.synced_revision {
            changed.base_envelope = Some(row.envelope.clone());
        }
        changed.synced_revision = server_revision.min(row.revision);
        if server_revision == 0 {
            changed.server_seq = None;
            if list
                && !row.tombstone
                && payload_of(session, &row)
                    .is_some_and(|payload| trash::deletable_type(&payload.type_name))
            {
                listed.push(row.record_id);
            }
        }
        rows.push(changed);
    }
    store.write(|tx| {
        for row in &rows {
            tx.record_put(row)?;
        }
        for id in &listed {
            tx.resync_returned_put(&vault_id, id, &now_text)?;
        }
        Ok(())
    })?;
    Ok(held)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn data_equality_ignores_nulls_and_order() {
        let a = json!({"address": "h", "label": null, "ssh": {"port": 22, "x": null}});
        let b = json!({"ssh": {"port": 22}, "address": "h"});
        assert!(data_equal(a.as_object(), b.as_object()));
        let c = json!({"address": "other"});
        assert!(!data_equal(a.as_object(), c.as_object()));
        assert!(data_equal(None, None));
        assert!(!data_equal(None, a.as_object()));
    }

    #[test]
    fn revoked_known_hosts_and_verified_pins_win() {
        let device = Uuid::now_v7();
        let known = |marker: Option<&str>, ms: u64| {
            let mut data = json!({"host": "h", "key_type": "ssh-ed25519", "public_key": "AAAA"});
            if let Some(marker) = marker {
                data["marker"] = json!(marker);
            }
            Payload::live(
                "known_host",
                1,
                Hlc::new(ms, 0),
                device,
                data.as_object().cloned().unwrap(),
            )
        };
        assert_eq!(
            type_rule(&known(Some("revoked"), 1), &known(None, 9)),
            Some(true)
        );
        assert_eq!(
            type_rule(&known(None, 9), &known(Some("revoked"), 1)),
            Some(false)
        );
        assert_eq!(type_rule(&known(None, 9), &known(None, 1)), None);
        let pin = |verified: bool, key: &str| {
            Payload::live(
                "member_pin",
                1,
                Hlc::new(1, 0),
                device,
                json!({"x25519_public_key": key, "verified": verified})
                    .as_object()
                    .cloned()
                    .unwrap(),
            )
        };
        assert_eq!(type_rule(&pin(true, "k"), &pin(false, "k")), Some(true));
        assert_eq!(type_rule(&pin(false, "k"), &pin(true, "k")), Some(false));
        assert_eq!(type_rule(&pin(true, "k"), &pin(false, "other")), None);
    }

    #[test]
    fn selection_rules() {
        let selection = Selection {
            classes: vec![2, 3],
            skip_types: vec!["terminal_theme".into()],
        };
        assert!(selection.class(1), "class 1 always");
        assert!(selection.class(2));
        assert!(!selection.class(6));
        assert!(!selection.type_name("terminal_theme"));
        assert!(!selection.type_name("last_session"));
        assert!(selection.type_name("host"));
    }
}
