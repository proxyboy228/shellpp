//! Known hosts import and export (UI 10.1, D21; docs/data-model.md,
//! sections 9.5 and 18.4; docs/storage-format.md, section 13.6): the file
//! format is in [`crate::known_hosts::openssh`].
//!
//! Import: the file is read outside the core mutex; its entries are matched
//! against the live entries by their natural key (host, port, key type, key,
//! marker), so a re-import, or a line repeated in the file, adds nothing and
//! counts as a duplicate. Existing entries are never changed. The new
//! entries are written with `source = import` in batches of
//! [`openssh::IMPORT_BATCH`], one transaction each, and the core mutex is
//! let go between batches, so sessions can check host keys during a large
//! import (decision of stage 2b). A failure stops the import; the batches
//! written before it stay. Logs carry only the four numbers.

use std::collections::HashSet;

use super::Core;
use super::dto::KnownHostsImportSummary;
use super::records::parts;
use crate::error::{CoreError, ValidationReason};
use crate::index::Index;
use crate::known_hosts::{decode_blob, openssh};
use crate::model::known_host::{self, Entry};
use crate::model::types::RecordKind;

/// The natural key of data-model 18.6, with the blob decoded and the port
/// effective.
type NaturalKey = (String, u16, String, bool, Vec<u8>);

fn natural_key(entry: &Entry) -> NaturalKey {
    (
        entry.host.clone(),
        entry.port.unwrap_or(22),
        entry.key_type.clone(),
        entry.revoked,
        decode_blob(&entry.public_key).unwrap_or_default(),
    )
}

/// The readable known host entries.
fn entries(index: &Index) -> Vec<Entry> {
    index
        .of_kind(RecordKind::KnownHost)
        .filter_map(|record| known_host::read(record.data()))
        .collect()
}

impl Core {
    pub(super) fn known_hosts_import_impl(
        &self,
        input: &[u8],
    ) -> Result<KnownHostsImportSummary, CoreError> {
        if input.len() > openssh::MAX_IMPORT_BYTES {
            return Err(CoreError::field("file", ValidationReason::TooLong));
        }
        {
            // Locked: nothing is parsed.
            let inner = self.inner();
            Self::require_session(&inner)?;
        }
        let file = openssh::parse(input);
        let mut summary = KnownHostsImportSummary {
            skipped: file.skipped,
            invalid: file.invalid,
            duplicates: file.duplicates,
            ..Default::default()
        };
        let mut known: Option<HashSet<NaturalKey>> = None;
        for batch in file.entries.chunks(openssh::IMPORT_BATCH) {
            let now = self.now();
            let mut guard = self.inner();
            let mut parts = parts(&mut guard)?;
            let known = known.get_or_insert_with(|| {
                entries(&parts.session.index)
                    .iter()
                    .map(natural_key)
                    .collect()
            });
            let mut writes = Vec::new();
            for entry in batch {
                if !known.insert(natural_key(entry)) {
                    summary.duplicates += 1;
                    continue;
                }
                let record_id = parts.ids.next(now);
                writes.push(parts.session.prepare_write(
                    parts.store,
                    RecordKind::KnownHost,
                    record_id,
                    known_host::to_data(entry),
                    now,
                )?);
                summary.imported += 1;
            }
            if !writes.is_empty() {
                self.commit_records(&mut parts, writes, Vec::new(), Vec::new())?;
            }
            // The mutex is let go here, before the next batch.
        }
        tracing::info!(
            imported = summary.imported,
            duplicates = summary.duplicates,
            skipped = summary.skipped,
            invalid = summary.invalid,
            "known_hosts imported"
        );
        Ok(summary)
    }

    pub(super) fn known_hosts_export_impl(&self) -> Result<String, CoreError> {
        let inner = self.inner();
        let session = Self::require_session(&inner)?;
        let entries = entries(&session.index);
        tracing::info!(entries = entries.len(), "known_hosts exported");
        Ok(openssh::write(&entries))
    }
}
