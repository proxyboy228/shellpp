//! The unlocked session: key ring, index, HLC clock and undo buffer. Dropped
//! (and thereby zeroized) on lock.
//!
//! Stage 2: rows of the lazy classes (`history`, `connection_log`; D10,
//! docs/storage-format.md, section 4.6) are not decrypted at unlock but on
//! first use ([`Session::ensure_class`]), and wiped on lock with the rest of
//! the session. The last recorded command of each terminal session (the
//! history duplicate rule) lives here too, so a lock wipes it.

use std::collections::HashMap;
use std::fmt;
use std::time::SystemTime;

use serde_json::{Map, Value};
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::error::CoreError;
use crate::index::{Index, IndexedRecord};
use crate::lock::UNDO_WINDOW;
use crate::model::hlc::HlcClock;
use crate::model::payload::Payload;
use crate::model::time::{format_timestamp, unix_ms};
use crate::model::types::{LAZY_CLASSES, RecordKind};
use crate::store::{RecordRow, Store};
use crate::vault::{self, KeyRing, Opened};

/// One removed record kept for undo (plan decision 14).
pub(crate) struct UndoEntry {
    pub kind: RecordKind,
    /// The last live payload, secrets included (wiped on drop).
    pub payload: Payload,
    pub deadline: SystemTime,
}

/// Removed records kept for 60 seconds, memory only.
#[derive(Default)]
pub(crate) struct UndoBuffer {
    entries: HashMap<Uuid, UndoEntry>,
}

impl UndoBuffer {
    pub fn put(&mut self, record_id: Uuid, kind: RecordKind, payload: Payload, now: SystemTime) {
        let deadline = now.checked_add(UNDO_WINDOW).unwrap_or(now);
        self.entries.insert(
            record_id,
            UndoEntry {
                kind,
                payload,
                deadline,
            },
        );
    }

    /// Drops entries whose undo window has passed.
    pub fn purge(&mut self, now: SystemTime) {
        self.entries.retain(|_, entry| now < entry.deadline);
    }

    pub fn get(&self, record_id: &Uuid) -> Option<&UndoEntry> {
        self.entries.get(record_id)
    }

    pub fn remove(&mut self, record_id: &Uuid) -> Option<UndoEntry> {
        self.entries.remove(record_id)
    }
}

/// A sealed record version waiting for its transaction.
pub(crate) struct PendingWrite {
    pub kind: RecordKind,
    pub record_id: Uuid,
    pub payload: Payload,
    pub row: RecordRow,
}

pub(crate) struct Session {
    pub ring: KeyRing,
    pub index: Index,
    pub hlc: HlcClock,
    pub undo: UndoBuffer,
    pub db_id: Uuid,
    pub device_id: Uuid,
    /// Lazy classes whose rows are in the index.
    pub lazy_loaded: Vec<u8>,
    /// Rows of lazy classes decrypted so far in this session (tests and logs).
    pub lazy_opened: usize,
    /// The last recorded history command per terminal session (memory only).
    pub history_last: HashMap<Uuid, Zeroizing<String>>,
}

impl fmt::Debug for Session {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Session")
            .field("ring", &self.ring)
            .field("index", &self.index)
            .field("lazy_loaded", &self.lazy_loaded)
            .finish_non_exhaustive()
    }
}

impl Session {
    /// An empty session (first run): the lazy classes have no rows yet.
    pub fn new(ring: KeyRing, db_id: Uuid, device_id: Uuid, hlc: HlcClock) -> Self {
        let index = Index::new(ring.vault_id());
        Self {
            ring,
            index,
            hlc,
            undo: UndoBuffer::default(),
            db_id,
            device_id,
            lazy_loaded: LAZY_CLASSES.to_vec(),
            lazy_opened: 0,
            history_last: HashMap::new(),
        }
    }

    /// Decrypts every row of the vault into a new index (unlock), except rows
    /// of the lazy classes. Rows that fail the checks are kept and reported
    /// by id; records of unknown types or newer versions are kept unchanged.
    pub fn load(
        store: &Store,
        mut ring: KeyRing,
        db_id: Uuid,
        device_id: Uuid,
        now: SystemTime,
    ) -> Result<Self, CoreError> {
        let vault_id = ring.vault_id();
        let rows = store.records_for_load(&vault_id)?;
        let mut index = Index::new(vault_id);
        for row in &rows {
            let row = match row {
                Ok(row) => row,
                // A row whose columns cannot be read is kept in the database
                // and left out of the index (docs/storage-format.md, section 4.6).
                Err(unreadable) => {
                    let reason = unreadable.error.code();
                    match unreadable.record_id {
                        Some(record_id) => {
                            tracing::warn!(record_id = %record_id, reason, "unreadable record left out of the index");
                            index.mark_damaged(record_id);
                        }
                        None => tracing::warn!(reason, "unreadable record row without an id"),
                    }
                    continue;
                }
            };
            if LAZY_CLASSES.contains(&row.class) {
                continue;
            }
            match vault::open_row(&mut ring, row) {
                Opened::Live(kind, payload) => {
                    index.insert(IndexedRecord::new(
                        row.record_id,
                        kind,
                        row.revision,
                        row.key_epoch,
                        payload,
                    ));
                }
                Opened::Tombstone(_) => {}
                Opened::Unsupported => index.mark_unsupported(row.record_id),
                Opened::Damaged(reason) => {
                    tracing::warn!(record_id = %row.record_id, reason, "damaged record left out of the index");
                    index.mark_damaged(row.record_id);
                }
            }
        }
        let hlc = HlcClock::start(index.max_hlc(), unix_ms(now));
        tracing::info!(
            records = index.len(),
            unsupported = index.unsupported().len(),
            damaged = index.damaged().len(),
            "index loaded"
        );
        Ok(Self {
            ring,
            index,
            hlc,
            undo: UndoBuffer::default(),
            db_id,
            device_id,
            lazy_loaded: Vec::new(),
            lazy_opened: 0,
            history_last: HashMap::new(),
        })
    }

    /// Decrypts the rows of a lazy class into the index on its first use in
    /// this session. Returns `true` when it loaded the class now. Records
    /// written in this session before the load are read back from their
    /// rows, so nothing is counted twice.
    pub fn ensure_class(&mut self, store: &Store, class: u8) -> Result<bool, CoreError> {
        if !LAZY_CLASSES.contains(&class) || self.lazy_loaded.contains(&class) {
            return Ok(false);
        }
        let rows = store.records(&self.ring.vault_id(), Some(&[class]))?;
        let mut loaded = 0usize;
        for row in &rows {
            self.lazy_opened += 1;
            match vault::open_row(&mut self.ring, row) {
                Opened::Live(kind, payload) => {
                    loaded += 1;
                    self.index.insert(IndexedRecord::new(
                        row.record_id,
                        kind,
                        row.revision,
                        row.key_epoch,
                        payload,
                    ));
                }
                Opened::Tombstone(_) => {}
                Opened::Unsupported => self.index.mark_unsupported(row.record_id),
                Opened::Damaged(reason) => {
                    tracing::warn!(record_id = %row.record_id, reason, "damaged record left out of the index");
                    self.index.mark_damaged(row.record_id);
                }
            }
        }
        self.lazy_loaded.push(class);
        tracing::debug!(class, records = loaded, "lazy class loaded");
        Ok(true)
    }

    /// Seals a new semantic version of a record (fresh HLC, this device).
    pub fn prepare_write(
        &mut self,
        store: &Store,
        kind: RecordKind,
        record_id: Uuid,
        data: Map<String, Value>,
        now: SystemTime,
    ) -> Result<PendingWrite, CoreError> {
        let hlc = self.hlc.tick(unix_ms(now));
        let payload = Payload::live(
            kind.name(),
            kind.current_version(),
            hlc,
            self.device_id,
            data,
        );
        let previous = store.record(&self.ring.vault_id(), &record_id)?;
        let row = vault::seal_row(
            &mut self.ring,
            kind.class(),
            record_id,
            &payload,
            previous.as_ref(),
            &format_timestamp(now),
        )?;
        Ok(PendingWrite {
            kind,
            record_id,
            payload,
            row,
        })
    }

    /// The tombstone row of a removed record.
    pub fn prepare_tombstone(
        &mut self,
        kind: RecordKind,
        previous: &RecordRow,
        now: SystemTime,
    ) -> Result<RecordRow, CoreError> {
        let hlc = self.hlc.tick(unix_ms(now));
        let payload = Payload::tombstone(kind.name(), kind.current_version(), hlc, self.device_id);
        vault::seal_row(
            &mut self.ring,
            kind.class(),
            previous.record_id,
            &payload,
            Some(previous),
            &format_timestamp(now),
        )
    }

    /// Puts committed writes into the index.
    pub fn commit(&mut self, writes: Vec<PendingWrite>) {
        for write in writes {
            let PendingWrite {
                kind,
                record_id,
                payload,
                row,
            } = write;
            self.index.insert(IndexedRecord::new(
                record_id,
                kind,
                row.revision,
                row.key_epoch,
                payload,
            ));
        }
    }

    /// Re-seals every live indexed record whose envelope uses an older epoch:
    /// `revision + 1`, the current epoch, the same payload and HLC
    /// (docs/sync-protocol.md, section 7.9). One transaction. Returns the ids.
    pub fn reseal_older(
        &mut self,
        store: &mut Store,
        now: SystemTime,
    ) -> Result<Vec<Uuid>, CoreError> {
        let current = self.ring.current_epoch();
        let vault_id = self.ring.vault_id();
        let stale: Vec<Uuid> = self
            .index
            .iter()
            .filter(|record| record.key_epoch < current)
            .map(|record| record.record_id)
            .collect();
        if stale.is_empty() {
            return Ok(Vec::new());
        }
        let now_text = format_timestamp(now);
        let mut rows: Vec<RecordRow> = Vec::with_capacity(stale.len());
        for id in &stale {
            let Some(previous) = store.record(&vault_id, id)? else {
                continue;
            };
            let Some(record) = self.index.get(id) else {
                continue;
            };
            rows.push(vault::seal_row(
                &mut self.ring,
                previous.class,
                *id,
                &record.payload,
                Some(&previous),
                &now_text,
            )?);
        }
        store.write(|tx| {
            for row in &rows {
                tx.record_put(row)?;
            }
            Ok(())
        })?;
        let mut ids = Vec::with_capacity(rows.len());
        for row in rows {
            if let Some(mut record) = self.index.remove(&row.record_id) {
                record.revision = row.revision;
                record.key_epoch = row.key_epoch;
                self.index.insert(record);
            }
            ids.push(row.record_id);
        }
        tracing::info!(count = ids.len(), epoch = current, "records re-sealed");
        Ok(ids)
    }
}
