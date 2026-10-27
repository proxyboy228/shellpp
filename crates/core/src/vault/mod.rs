//! Vault keys in memory (the key ring), personal vault creation, key wraps
//! (`kek`, `recovery`, `device`, `chain`), personal vault rotation and the
//! sealing and opening of record envelopes (docs/security.md, sections 5.2-5.3
//! and 5.9; docs/storage-format.md, sections 4.6, 5 and 6). Personal vault
//! only in stage 1.
//!
//! Owner: W2.

use std::collections::BTreeMap;
use std::fmt;

use shellpp_sync_api::envelope::{EnvelopeHeader, VERSION_1};
use uuid::Uuid;

use crate::crypto::{self, CryptoError, Key32, WrapPurpose};
use crate::error::CoreError;
use crate::model::payload::Payload;
use crate::model::types::{RecordKind, class_of_type_name};
use crate::service::dto::{VaultKind, VaultStatus};
use crate::store::{KeyWrapRow, RecordRow, Store, StoreError, VaultRow, WrapKind, WriteTx};

/// Vault keys of one vault, by epoch, plus cached record keys. Every key is a
/// [`Key32`], zeroized when the ring is dropped (lock).
pub struct KeyRing {
    vault_id: Uuid,
    current_epoch: u32,
    keys: BTreeMap<u32, Key32>,
    record_keys: BTreeMap<u32, Key32>,
}

impl KeyRing {
    /// A ring holding the current key only.
    pub fn new(vault_id: Uuid, current_epoch: u32, current_key: Key32) -> Self {
        let mut keys = BTreeMap::new();
        keys.insert(current_epoch, current_key);
        Self {
            vault_id,
            current_epoch,
            keys,
            record_keys: BTreeMap::new(),
        }
    }

    pub fn vault_id(&self) -> Uuid {
        self.vault_id
    }

    pub fn current_epoch(&self) -> u32 {
        self.current_epoch
    }

    /// The vault key of the current epoch.
    pub fn current_key(&self) -> &Key32 {
        // The current key is inserted by `new` and `push_epoch` and never removed
        // without moving `current_epoch` back to an epoch that has a key.
        &self.keys[&self.current_epoch]
    }

    pub fn key(&self, epoch: u32) -> Option<&Key32> {
        self.keys.get(&epoch)
    }

    /// Opens the older epochs through the `chain` rows, newest first
    /// (docs/storage-format.md, section 6.2). Stops at the first missing or
    /// failing row; records of unreachable epochs are reported as damaged.
    /// Returns the oldest epoch reached.
    pub fn open_chain(&mut self, store: &Store) -> Result<u32, CoreError> {
        let mut epoch = self.current_epoch;
        while epoch > 1 {
            if self.keys.contains_key(&(epoch - 1)) {
                epoch -= 1;
                continue;
            }
            let Some(row) = store.key_wrap(&self.vault_id, epoch, WrapKind::Chain)? else {
                tracing::warn!(vault_id = %self.vault_id, epoch, "epoch chain row missing");
                break;
            };
            let chain_key = crypto::derive_chain_key(&self.keys[&epoch], &self.vault_id, epoch);
            let aad = crypto::vault_wrap_aad(WrapPurpose::EpochChain, &self.vault_id, epoch - 1);
            match crypto::unwrap_key(&chain_key, &aad, &row.wrap) {
                Ok(older) => {
                    self.keys.insert(epoch - 1, older);
                    epoch -= 1;
                }
                Err(_) => {
                    tracing::warn!(vault_id = %self.vault_id, epoch, "epoch chain row does not open");
                    break;
                }
            }
        }
        Ok(epoch)
    }

    /// Makes `epoch` (with `key`) the current epoch; used for a rotation just
    /// before its transaction. [`KeyRing::pop_epoch`] undoes it.
    pub fn push_epoch(&mut self, epoch: u32, key: Key32) {
        self.keys.insert(epoch, key);
        self.current_epoch = epoch;
    }

    /// Removes the keys above `epoch` and makes it current again (a rotation
    /// whose transaction failed).
    pub fn pop_epoch(&mut self, epoch: u32) {
        self.keys.retain(|held, _| *held <= epoch);
        self.record_keys.retain(|held, _| *held <= epoch);
        self.current_epoch = epoch;
    }

    pub(crate) fn record_key(&mut self, epoch: u32) -> Option<&Key32> {
        if !self.record_keys.contains_key(&epoch) {
            let vault_key = self.keys.get(&epoch)?;
            let record_key = crypto::derive_record_key(vault_key, &self.vault_id, epoch);
            self.record_keys.insert(epoch, record_key);
        }
        self.record_keys.get(&epoch)
    }
}

impl fmt::Debug for KeyRing {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("KeyRing")
            .field("vault_id", &self.vault_id)
            .field("current_epoch", &self.current_epoch)
            .field("epochs", &self.keys.keys().collect::<Vec<_>>())
            .finish()
    }
}

fn purpose(kind: WrapKind) -> Option<WrapPurpose> {
    match kind {
        WrapKind::Kek => Some(WrapPurpose::Kek),
        WrapKind::Recovery => Some(WrapPurpose::Recovery),
        WrapKind::Device => Some(WrapPurpose::Device),
        WrapKind::Chain => Some(WrapPurpose::EpochChain),
        WrapKind::Hpke => None,
    }
}

/// A `kek`, `recovery` or `device` row: `vault_key[v, e]` wrapped by `wrapping_key`.
pub fn wrap_row(
    kind: WrapKind,
    vault_id: Uuid,
    epoch: u32,
    wrapping_key: &Key32,
    vault_key: &Key32,
) -> Result<KeyWrapRow, CoreError> {
    let purpose = purpose(kind)
        .filter(|purpose| *purpose != WrapPurpose::EpochChain)
        .ok_or(CoreError::Internal("unsupported wrap kind"))?;
    let aad = crypto::vault_wrap_aad(purpose, &vault_id, epoch);
    Ok(KeyWrapRow {
        vault_id,
        epoch,
        kind,
        sender_user_id: None,
        hpke_enc: None,
        wrap: crypto::wrap_key(wrapping_key, &aad, vault_key)?,
    })
}

/// Opens a `kek`, `recovery` or `device` row.
pub fn open_wrap(row: &KeyWrapRow, wrapping_key: &Key32) -> Result<Key32, CryptoError> {
    let purpose = match row.kind {
        WrapKind::Kek => WrapPurpose::Kek,
        WrapKind::Recovery => WrapPurpose::Recovery,
        WrapKind::Device => WrapPurpose::Device,
        _ => {
            return Err(CryptoError::Malformed(
                "not a password, recovery or device wrap",
            ));
        }
    };
    let aad = crypto::vault_wrap_aad(purpose, &row.vault_id, row.epoch);
    crypto::unwrap_key(wrapping_key, &aad, &row.wrap)
}

/// The `chain` row of epoch `epoch`: `vault_key[v, epoch - 1]` wrapped by
/// `chain_key[v, epoch]`.
pub fn chain_row(
    vault_id: Uuid,
    epoch: u32,
    key: &Key32,
    previous: &Key32,
) -> Result<KeyWrapRow, CoreError> {
    let chain_key = crypto::derive_chain_key(key, &vault_id, epoch);
    let aad = crypto::vault_wrap_aad(WrapPurpose::EpochChain, &vault_id, epoch - 1);
    Ok(KeyWrapRow {
        vault_id,
        epoch,
        kind: WrapKind::Chain,
        sender_user_id: None,
        hpke_enc: None,
        wrap: crypto::wrap_key(&chain_key, &aad, previous)?,
    })
}

/// The new personal vault of a first run.
pub struct NewVault {
    pub row: VaultRow,
    pub key: Key32,
}

impl fmt::Debug for NewVault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NewVault")
            .field("row", &self.row)
            .finish_non_exhaustive()
    }
}

/// A new personal vault: random `vault_key[v, 1]`, `vaults` row (personal,
/// active, role NULL, epoch 1, not bound).
pub fn create_personal_vault(vault_id: Uuid, created_at: String) -> Result<NewVault, CoreError> {
    Ok(NewVault {
        row: VaultRow {
            vault_id,
            kind: VaultKind::Personal,
            status: VaultStatus::Active,
            role: None,
            current_epoch: 1,
            bound: false,
            created_at,
        },
        key: Key32::generate()?,
    })
}

/// A prepared personal vault rotation (docs/security.md, section 5.9): epoch
/// `e + 1` with a new random key, its `chain` row, `kek`, `recovery` and
/// optionally `device` rows, and the deletion of the epoch-`e` rows of those
/// kinds. Written by [`Rotation::write`] in one transaction.
pub struct Rotation {
    pub old_epoch: u32,
    pub new_epoch: u32,
    new_key: Key32,
    rows: Vec<KeyWrapRow>,
    vault_row: VaultRow,
}

impl Rotation {
    pub fn prepare(
        ring: &KeyRing,
        vault_row: &VaultRow,
        kek: &Key32,
        recovery_kek: &Key32,
        device_wrap_key: Option<&Key32>,
    ) -> Result<Self, CoreError> {
        let old_epoch = ring.current_epoch();
        let new_epoch = old_epoch
            .checked_add(1)
            .ok_or(CoreError::Internal("epoch overflow"))?;
        let vault_id = ring.vault_id();
        let new_key = Key32::generate()?;
        let mut rows = vec![
            chain_row(vault_id, new_epoch, &new_key, ring.current_key())?,
            wrap_row(WrapKind::Kek, vault_id, new_epoch, kek, &new_key)?,
            wrap_row(
                WrapKind::Recovery,
                vault_id,
                new_epoch,
                recovery_kek,
                &new_key,
            )?,
        ];
        if let Some(device_wrap_key) = device_wrap_key {
            rows.push(wrap_row(
                WrapKind::Device,
                vault_id,
                new_epoch,
                device_wrap_key,
                &new_key,
            )?);
        }
        let mut vault_row = vault_row.clone();
        vault_row.current_epoch = new_epoch;
        Ok(Self {
            old_epoch,
            new_epoch,
            new_key,
            rows,
            vault_row,
        })
    }

    /// The prepared row of `kind` (`chain`, `kek`, `recovery`, `device`).
    pub fn row(&self, kind: WrapKind) -> Option<&KeyWrapRow> {
        self.rows.iter().find(|row| row.kind == kind)
    }

    /// A copy of the new vault key (for [`KeyRing::push_epoch`]).
    pub fn new_key(&self) -> Key32 {
        self.new_key.duplicate()
    }

    pub fn write(&self, tx: &mut WriteTx<'_>) -> Result<(), StoreError> {
        for row in &self.rows {
            tx.key_wrap_put(row)?;
        }
        let vault_id = self.vault_row.vault_id;
        for kind in [WrapKind::Kek, WrapKind::Recovery, WrapKind::Device] {
            tx.key_wrap_delete(&vault_id, self.old_epoch, kind)?;
        }
        tx.vault_put(&self.vault_row)
    }
}

impl fmt::Debug for Rotation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Rotation")
            .field("old_epoch", &self.old_epoch)
            .field("new_epoch", &self.new_epoch)
            .finish_non_exhaustive()
    }
}

/// Seals `payload` as the next version of a record (docs/storage-format.md,
/// section 4.6): `revision + 1` (1 for a new row), the current epoch, a fresh
/// nonce; sync bookkeeping is carried over from `previous`.
pub fn seal_row(
    ring: &mut KeyRing,
    class: u8,
    record_id: Uuid,
    payload: &Payload,
    previous: Option<&RecordRow>,
    now_text: &str,
) -> Result<RecordRow, CoreError> {
    seal_bytes(
        ring,
        class,
        record_id,
        payload.deleted,
        &payload.to_json_bytes(),
        previous,
        payload.deleted.then(|| now_text.to_owned()),
    )
}

/// Re-seals `previous` under the ring's current epoch with exactly the
/// plaintext it holds, members of a later version included: re-encrypting a
/// pending row after a rotation never rewrites what it carries
/// (storage-format 5.6; run 17). `None` when its epoch key is missing, its
/// envelope does not open, or it fails a check of section 5.4 (a payload
/// that does not parse, a header that disagrees with its columns).
pub fn reseal_row(
    ring: &mut KeyRing,
    previous: &RecordRow,
    now_text: &str,
) -> Result<Option<RecordRow>, CoreError> {
    if let Opened::Damaged(_) = open_row(ring, previous) {
        return Ok(None);
    }
    let opened = match ring.record_key(previous.key_epoch) {
        Some(key) => crypto::open_envelope(key, &previous.envelope).ok(),
        None => None,
    };
    let Some((header, bytes)) = opened else {
        return Ok(None);
    };
    let tombstoned_at = previous
        .tombstoned_at
        .clone()
        .or_else(|| header.tombstone.then(|| now_text.to_owned()));
    seal_bytes(
        ring,
        previous.class,
        previous.record_id,
        header.tombstone,
        &bytes,
        Some(previous),
        tombstoned_at,
    )
    .map(Some)
}

/// Seals `bytes` as the next revision after `previous` (or the first one)
/// under the ring's current epoch.
fn seal_bytes(
    ring: &mut KeyRing,
    class: u8,
    record_id: Uuid,
    tombstone: bool,
    bytes: &[u8],
    previous: Option<&RecordRow>,
    tombstoned_at: Option<String>,
) -> Result<RecordRow, CoreError> {
    let (revision, synced_revision, server_seq, base_envelope) = match previous {
        None => (1, 0, None, None),
        Some(previous) => {
            // A record pushed back by a resync keeps its synced envelope as
            // the base until the server holds a version again.
            let base = if previous.synced_revision == 0 {
                previous.base_envelope.clone()
            } else if previous.revision == previous.synced_revision {
                Some(previous.envelope.clone())
            } else {
                previous.base_envelope.clone()
            };
            (
                previous
                    .revision
                    .checked_add(1)
                    .ok_or(CoreError::Internal("revision overflow"))?,
                previous.synced_revision,
                previous.server_seq,
                base,
            )
        }
    };
    let epoch = ring.current_epoch();
    let vault_id = ring.vault_id();
    let header = EnvelopeHeader {
        class,
        tombstone,
        vault_id: *vault_id.as_bytes(),
        record_id: *record_id.as_bytes(),
        revision,
        key_epoch: epoch,
    };
    let record_key = ring
        .record_key(epoch)
        .ok_or(CoreError::Internal("current epoch key missing"))?;
    let envelope =
        crypto::seal_envelope(record_key, &header, bytes).map_err(|error| match error {
            CryptoError::TooLarge => CoreError::TooLarge,
            other => CoreError::Crypto(other),
        })?;
    Ok(RecordRow {
        vault_id,
        record_id,
        class,
        revision,
        synced_revision,
        key_epoch: epoch,
        tombstone,
        tombstoned_at,
        server_seq,
        envelope,
        base_envelope,
    })
}

/// The result of opening a stored row.
#[derive(Debug)]
pub enum Opened {
    /// A live record of a stage-1 type with a supported payload version.
    Live(RecordKind, Payload),
    /// A tombstone.
    Tombstone(Payload),
    /// A newer envelope version, an unknown type or a newer payload version:
    /// kept, excluded from the index, never rewritten.
    Unsupported,
    /// Failed one of the checks of docs/storage-format.md, section 5.4.
    Damaged(&'static str),
}

/// Opens a row with checks 1-8 of docs/storage-format.md, section 5.4.
pub fn open_row(ring: &mut KeyRing, row: &RecordRow) -> Opened {
    match row.envelope.first() {
        None => return Opened::Damaged("empty envelope"),
        Some(version) if *version != VERSION_1 => return Opened::Unsupported,
        Some(_) => {}
    }
    if row.vault_id != ring.vault_id() {
        return Opened::Damaged("vault mismatch");
    }
    let Some(record_key) = ring.record_key(row.key_epoch) else {
        return Opened::Damaged("epoch key unavailable");
    };
    let (header, bytes) = match crypto::open_envelope(record_key, &row.envelope) {
        Ok(opened) => opened,
        Err(_) => return Opened::Damaged("envelope does not open"),
    };
    if header.vault_id != *row.vault_id.as_bytes()
        || header.record_id != *row.record_id.as_bytes()
        || header.revision != row.revision
        || header.key_epoch != row.key_epoch
        || header.tombstone != row.tombstone
        || header.class != row.class
    {
        return Opened::Damaged("header and columns disagree");
    }
    let payload = match Payload::from_json_bytes(&bytes) {
        Ok(payload) => payload,
        Err(_) => return Opened::Damaged("payload malformed"),
    };
    match class_of_type_name(&payload.type_name) {
        Some(class) if class != header.class => return Opened::Damaged("type and class disagree"),
        Some(_) => {}
        None => return Opened::Unsupported,
    }
    if payload.deleted != header.tombstone {
        return Opened::Damaged("tombstone flag and payload disagree");
    }
    if payload.deleted {
        return Opened::Tombstone(payload);
    }
    match RecordKind::from_name(&payload.type_name) {
        Some(kind) if payload.v <= kind.current_version() => Opened::Live(kind, payload),
        _ => Opened::Unsupported,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::hlc::Hlc;
    use serde_json::{Map, json};

    fn payload(kind: RecordKind, data: serde_json::Value) -> Payload {
        Payload::live(
            kind.name(),
            1,
            Hlc::new(1000, 0),
            Uuid::now_v7(),
            data.as_object().cloned().unwrap_or_default(),
        )
    }

    #[test]
    fn seal_and_open_round_trip() {
        let vault_id = Uuid::now_v7();
        let mut ring = KeyRing::new(vault_id, 1, Key32::generate().unwrap());
        let id = Uuid::now_v7();
        let first = payload(RecordKind::Host, json!({"address": "h.example.com"}));
        let row = seal_row(&mut ring, 0x02, id, &first, None, "t").unwrap();
        assert_eq!(row.revision, 1);
        assert_eq!(row.key_epoch, 1);
        match open_row(&mut ring, &row) {
            Opened::Live(RecordKind::Host, opened) => assert_eq!(opened, first),
            other => panic!("unexpected {other:?}"),
        }
        let second = seal_row(&mut ring, 0x02, id, &first, Some(&row), "t").unwrap();
        assert_eq!(second.revision, 2);
        assert_ne!(second.envelope, row.envelope);

        // Tombstone.
        let tomb = Payload::tombstone("host", 1, Hlc::new(1001, 0), Uuid::now_v7());
        let tomb_row = seal_row(
            &mut ring,
            0x02,
            id,
            &tomb,
            Some(&second),
            "2026-10-18T22:31:05.123Z",
        )
        .unwrap();
        assert!(tomb_row.tombstone);
        assert_eq!(
            tomb_row.tombstoned_at.as_deref(),
            Some("2026-10-18T22:31:05.123Z")
        );
        assert!(matches!(
            open_row(&mut ring, &tomb_row),
            Opened::Tombstone(_)
        ));
    }

    #[test]
    fn checks_detect_tampering_and_unsupported_records() {
        let vault_id = Uuid::now_v7();
        let mut ring = KeyRing::new(vault_id, 1, Key32::generate().unwrap());
        let id = Uuid::now_v7();
        let row = seal_row(
            &mut ring,
            0x02,
            id,
            &payload(RecordKind::Host, json!({})),
            None,
            "t",
        )
        .unwrap();

        let mut wrong_revision = row.clone();
        wrong_revision.revision = 7;
        assert!(matches!(
            open_row(&mut ring, &wrong_revision),
            Opened::Damaged(_)
        ));

        let mut flipped = row.clone();
        let last = flipped.envelope.len() - 1;
        flipped.envelope[last] ^= 1;
        assert!(matches!(open_row(&mut ring, &flipped), Opened::Damaged(_)));

        let mut newer = row.clone();
        newer.envelope[0] = 0x02;
        assert!(matches!(open_row(&mut ring, &newer), Opened::Unsupported));

        // Type in the wrong class.
        let wrong_class = seal_row(
            &mut ring,
            0x01,
            Uuid::now_v7(),
            &payload(RecordKind::Host, json!({})),
            None,
            "t",
        )
        .unwrap();
        assert!(matches!(
            open_row(&mut ring, &wrong_class),
            Opened::Damaged(_)
        ));

        // Unknown type and newer payload version.
        let mut unknown = payload(RecordKind::Host, json!({}));
        unknown.type_name = "future_type".into();
        let unknown_row = seal_row(&mut ring, 0x02, Uuid::now_v7(), &unknown, None, "t").unwrap();
        assert!(matches!(
            open_row(&mut ring, &unknown_row),
            Opened::Unsupported
        ));
        let mut newer_v = payload(RecordKind::Host, json!({}));
        newer_v.v = 2;
        let newer_row = seal_row(&mut ring, 0x02, Uuid::now_v7(), &newer_v, None, "t").unwrap();
        assert!(matches!(
            open_row(&mut ring, &newer_row),
            Opened::Unsupported
        ));

        // A custom terminal theme (a later stage) in its own class is kept
        // but not indexed.
        let mut later = payload(RecordKind::Host, json!({}));
        later.type_name = "terminal_theme".into();
        let later_row = seal_row(&mut ring, 0x09, Uuid::now_v7(), &later, None, "t").unwrap();
        assert!(matches!(
            open_row(&mut ring, &later_row),
            Opened::Unsupported
        ));

        // Another ring cannot open it.
        let mut other = KeyRing::new(vault_id, 1, Key32::generate().unwrap());
        assert!(matches!(open_row(&mut other, &row), Opened::Damaged(_)));
        let _ = Map::<String, serde_json::Value>::new();
    }

    #[test]
    fn wraps_bind_kind_vault_and_epoch() {
        let vault_id = Uuid::now_v7();
        let vault_key = Key32::generate().unwrap();
        let kek = Key32::generate().unwrap();
        let row = wrap_row(WrapKind::Kek, vault_id, 3, &kek, &vault_key).unwrap();
        assert_eq!(open_wrap(&row, &kek).unwrap().expose(), vault_key.expose());
        let mut moved = row.clone();
        moved.epoch = 4;
        assert!(open_wrap(&moved, &kek).is_err());
        let mut relabeled = row.clone();
        relabeled.kind = WrapKind::Recovery;
        assert!(open_wrap(&relabeled, &kek).is_err());
        assert!(open_wrap(&row, &Key32::generate().unwrap()).is_err());
    }

    #[test]
    fn rotation_prepares_chain_and_wraps() {
        let vault_id = Uuid::now_v7();
        let old_key = Key32::generate().unwrap();
        let old_copy = old_key.duplicate();
        let mut ring = KeyRing::new(vault_id, 1, old_key);
        let vault = create_personal_vault(vault_id, "t".into()).unwrap();
        let kek = Key32::generate().unwrap();
        let recovery = Key32::generate().unwrap();
        let rotation = Rotation::prepare(&ring, &vault.row, &kek, &recovery, None).unwrap();
        assert_eq!(rotation.new_epoch, 2);
        assert_eq!(rotation.rows.len(), 3);
        let chain = &rotation.rows[0];
        assert_eq!(chain.kind, WrapKind::Chain);
        assert_eq!(chain.epoch, 2);
        // The chain row opens the old key with the new key's chain key.
        let new_key = rotation.new_key();
        let chain_key = crypto::derive_chain_key(&new_key, &vault_id, 2);
        let aad = crypto::vault_wrap_aad(WrapPurpose::EpochChain, &vault_id, 1);
        assert_eq!(
            crypto::unwrap_key(&chain_key, &aad, &chain.wrap)
                .unwrap()
                .expose(),
            old_copy.expose()
        );
        ring.push_epoch(2, rotation.new_key());
        assert_eq!(ring.current_epoch(), 2);
        assert!(ring.key(1).is_some());
        ring.pop_epoch(1);
        assert_eq!(ring.current_epoch(), 1);
        assert!(ring.key(2).is_none());
    }
}
