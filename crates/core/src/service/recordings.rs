//! Session recordings in the service (UI 11.2, 12.6; storage-format.md,
//! section 8.4; D16, D20, D75): whether a session is recorded, the start
//! of its file (named in its connection log entry), reading it back, and
//! the files that go with their entries.

use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde_json::{Map, Value, json};
use uuid::Uuid;

use super::Core;
use super::dto::RecordRef;
use super::records::parts;
use crate::crypto;
use crate::error::CoreError;
use crate::model::types::RecordKind;
use crate::recording::{self, Header, Recording, RecordingWriter};
use crate::store::{Store, StoreError};

/// `recording_file_id` of a log entry.
pub(super) fn file_id_of(data: &Map<String, Value>) -> Option<Uuid> {
    data.get("recording_file_id")
        .and_then(Value::as_str)
        .and_then(|id| Uuid::parse_str(id).ok())
}

/// The file of `file_id` in the data directory of `store`.
fn file_path(store: &Store, file_id: Uuid) -> Option<PathBuf> {
    store.data_dir().map(|dir| {
        dir.join(recording::DIR_NAME)
            .join(format!("{file_id}.{}", recording::EXTENSION))
    })
}

/// Deletes the files of `file_ids` (their entries are gone already,
/// storage-format.md 8.4: the entry first, the file second). A file that
/// is not there is no error.
pub(super) fn delete_files(store: &Store, file_ids: &[Uuid]) {
    for file_id in file_ids {
        let Some(path) = file_path(store, *file_id) else {
            continue;
        };
        match std::fs::remove_file(&path) {
            Ok(()) => tracing::info!("recording removed"),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => tracing::warn!(%error, "recording not removed"),
        }
    }
}

fn io(error: std::io::Error) -> CoreError {
    CoreError::Store(StoreError::from(error))
}

fn recording_error(error: recording::RecordingError) -> CoreError {
    match error {
        recording::RecordingError::Io(error) => io(error),
        recording::RecordingError::NoKey => CoreError::Locked,
        recording::RecordingError::Format | recording::RecordingError::Damaged => {
            CoreError::Damaged
        }
    }
}

/// `record_output` of a host: its override unless "As in settings".
fn record_output_of(data: &Map<String, Value>, setting: bool) -> bool {
    match data.get("record_output").and_then(Value::as_str) {
        Some("on") => true,
        Some("off") => false,
        _ => setting,
    }
}

impl Core {
    /// Whether the output of a session to `host` is recorded (UI 12.6, 3.1;
    /// data-model 8.6): a host of the personal vault decides with its
    /// "Record output", else `logs.record_output` (off by default, D16).
    pub fn record_output_for(&self, host: Option<RecordRef>) -> bool {
        let mut guard = self.inner();
        let setting = guard
            .settings
            .get("logs.record_output")
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
        let Ok(parts) = parts(&mut guard) else {
            return false;
        };
        let personal = parts.session.ring.vault_id();
        match host {
            Some(host) if host.vault_id == personal => parts
                .session
                .index
                .get_kind(&host.record_id, RecordKind::Host)
                .is_some_and(|record| record_output_of(record.data(), setting)),
            _ => setting,
        }
    }

    /// Starts the recording of the session of log entry `log_id`: the entry
    /// names its file first, then the file is created (storage-format.md
    /// 8.4). The writer keeps its own key, so output that arrives while the
    /// app is locked is still recorded; the key opens only that file.
    pub fn recording_start(
        &self,
        log_id: Uuid,
        size: (u16, u16),
    ) -> Result<RecordingWriter, CoreError> {
        let now = self.now();
        let mut guard = self.inner();
        let mut parts = parts(&mut guard)?;
        let record = parts
            .session
            .index
            .get_kind(&log_id, RecordKind::ConnectionLogEntry)
            .ok_or(CoreError::NotFound)?;
        let dir = parts
            .store
            .data_dir()
            .map(|dir| dir.join(recording::DIR_NAME))
            .ok_or(CoreError::Unavailable)?;
        let file_id = parts.ids.next(now);
        let mut data = record.data().clone();
        data.insert("recording_file_id".into(), json!(file_id.to_string()));
        self.write_record(
            &mut parts,
            RecordKind::ConnectionLogEntry,
            log_id,
            data,
            now,
        )?;
        let ring = &parts.session.ring;
        let key = crypto::derive_recording_key(ring.current_key(), &file_id);
        let header =
            Header::new(file_id, ring.vault_id(), ring.current_epoch()).map_err(recording_error)?;
        std::fs::create_dir_all(&dir).map_err(io)?;
        crate::store::restrict_dir(&dir)?;
        let unix = now
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_secs());
        let path = dir.join(format!("{file_id}.{}", recording::EXTENSION));
        let writer =
            RecordingWriter::create(&path, header, key, size, unix).map_err(recording_error)?;
        tracing::info!("session recording started");
        Ok(writer)
    }

    /// The recording of log entry `log_id`, opened (UI 11.2): `None` when
    /// the session was not recorded. A file that is not on this device is
    /// [`CoreError::NotFound`] ("recording not available on this device").
    pub fn recording_read(&self, log_id: Uuid) -> Result<Option<Recording>, CoreError> {
        let mut guard = self.inner();
        let parts = parts(&mut guard)?;
        let record = parts
            .session
            .index
            .get_kind(&log_id, RecordKind::ConnectionLogEntry)
            .ok_or(CoreError::NotFound)?;
        let Some(file_id) = file_id_of(record.data()) else {
            return Ok(None);
        };
        let path = file_path(parts.store, file_id).ok_or(CoreError::NotFound)?;
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Err(CoreError::NotFound);
            }
            Err(error) => return Err(io(error)),
        };
        let ring = &parts.session.ring;
        let recording = recording::read(&bytes, |header| {
            (header.file_id == file_id && header.vault_id == ring.vault_id())
                .then(|| ring.key(header.key_epoch))
                .flatten()
                .map(|vault_key| crypto::derive_recording_key(vault_key, &file_id))
        })
        .map_err(recording_error)?;
        Ok(Some(recording))
    }

    /// "Export to .txt" (UI 11.2): the recording of `log_id` as plain text
    /// (escape sequences stripped) written to `path`. The file is not
    /// encrypted: the interface warns first (D16).
    pub fn recording_export_txt(&self, log_id: Uuid, path: &Path) -> Result<(), CoreError> {
        let recording = self.recording_read(log_id)?.ok_or(CoreError::NotFound)?;
        shellpp_private_file::write(
            path,
            recording.plain_text().as_bytes(),
            shellpp_private_file::Existing::Replace,
        )
        .map_err(io)
    }
}
