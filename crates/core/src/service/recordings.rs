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
use crate::vault::KeyRing;

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

// -------------------------------------------------------------------------
// Re-encryption into another personal vault (storage-format.md 8.4)
// -------------------------------------------------------------------------

/// The file name extension of a recording's pending re-encryption.
const REKEY_EXTENSION: &str = "rekey";
/// Magic of a `.rekey` file.
const REKEY_MAGIC: &[u8; 7] = b"shpprky";
/// Bytes of a `.rekey` file before its wrap.
const REKEY_HEAD: usize = 44;

fn recordings_dir(store: &Store) -> Option<PathBuf> {
    store.data_dir().map(|dir| dir.join(recording::DIR_NAME))
}

/// A `.rekey` file: the key a recording is sealed under now, wrapped for
/// the personal vault it moves into (storage-format.md 8.4).
struct Rekey {
    head: [u8; REKEY_HEAD],
    file_id: Uuid,
    vault_id: Uuid,
    epoch: u32,
    wrap: Vec<u8>,
}

impl Rekey {
    fn head(file_id: Uuid, vault_id: Uuid, epoch: u32) -> [u8; REKEY_HEAD] {
        let mut head = [0u8; REKEY_HEAD];
        head[..7].copy_from_slice(REKEY_MAGIC);
        head[7] = 1;
        head[8..24].copy_from_slice(file_id.as_bytes());
        head[24..40].copy_from_slice(vault_id.as_bytes());
        head[40..44].copy_from_slice(&epoch.to_be_bytes());
        head
    }

    /// The wrapping key: HKDF of the file's `recording_key` in the new
    /// vault, so that key itself seals only the file's chunks.
    fn wrapping(vault_key: &crypto::Key32, file_id: Uuid) -> crypto::Key32 {
        let recording_key = crypto::derive_recording_key(vault_key, &file_id);
        crypto::hkdf32(&recording_key, crypto::labels::WRAP_RECORDING_REKEY, &[])
    }

    fn aad(head: &[u8; REKEY_HEAD]) -> Vec<u8> {
        let mut aad = crypto::labels::WRAP_RECORDING_REKEY.to_vec();
        aad.extend_from_slice(head);
        aad
    }

    /// Wraps `key` (the file's key now) under the file's key in `ring`'s
    /// current epoch.
    fn seal(file_id: Uuid, ring: &KeyRing, key: &crypto::Key32) -> Result<Vec<u8>, CoreError> {
        let head = Self::head(file_id, ring.vault_id(), ring.current_epoch());
        let wrapping = Self::wrapping(ring.current_key(), file_id);
        let wrap = crypto::wrap_key(&wrapping, &Self::aad(&head), key)
            .map_err(|_| CoreError::Internal("recording rekey"))?;
        let mut bytes = head.to_vec();
        bytes.extend_from_slice(&wrap);
        Ok(bytes)
    }

    fn parse(bytes: &[u8]) -> Option<Self> {
        let head: [u8; REKEY_HEAD] = bytes.get(..REKEY_HEAD)?.try_into().ok()?;
        if &head[..7] != REKEY_MAGIC || head[7] != 1 {
            return None;
        }
        let id = |range: std::ops::Range<usize>| {
            <[u8; 16]>::try_from(&head[range])
                .ok()
                .map(Uuid::from_bytes)
        };
        Some(Self {
            file_id: id(8..24)?,
            vault_id: id(24..40)?,
            epoch: u32::from_be_bytes(head[40..44].try_into().ok()?),
            wrap: bytes[REKEY_HEAD..].to_vec(),
            head,
        })
    }

    /// The file's key now, when `ring` is the vault it moves into.
    fn open(&self, ring: &KeyRing) -> Option<crypto::Key32> {
        if self.vault_id != ring.vault_id() {
            return None;
        }
        let wrapping = Self::wrapping(ring.key(self.epoch)?, self.file_id);
        crypto::unwrap_key(&wrapping, &Self::aad(&self.head), &self.wrap).ok()
    }
}

/// `<file_id>.<vault_id>.rekey`: one per vault the file moves into, so a
/// merge that does not commit never replaces the one an earlier merge left.
fn rekey_path(dir: &Path, file_id: Uuid, vault_id: Uuid) -> PathBuf {
    dir.join(format!("{file_id}.{vault_id}.{REKEY_EXTENSION}"))
}

fn read_rekey(dir: &Path, file_id: Uuid, vault_id: Uuid) -> Option<Rekey> {
    std::fs::read(rekey_path(dir, file_id, vault_id))
        .ok()
        .and_then(|bytes| Rekey::parse(&bytes))
        .filter(|rekey| rekey.file_id == file_id && rekey.vault_id == vault_id)
}

/// The header of the recording at `path` (its first 63 bytes only).
fn header_of(path: &Path) -> Option<Header> {
    use std::io::Read as _;
    let mut head = [0u8; recording::HEADER_LEN];
    std::fs::File::open(path).ok()?.read_exact(&mut head).ok()?;
    Header::parse(&head).ok()
}

/// Writes `bytes` beside `path`, syncs and renames it over `path`.
fn write_synced(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write as _;
    let temp = path.with_extension("rekey-new");
    let _ = std::fs::remove_file(&temp);
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let written = (|| {
        let mut file = options.open(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&temp, path)?;
        match path.parent() {
            Some(dir) => recording::sync_dir(dir),
            None => Ok(()),
        }
    })();
    if written.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    written
}

/// The merge at sign-in, before its transaction (storage-format 6.6, 8.4):
/// every recording still sealed under `old` gets a `.rekey` file with its
/// key wrapped for `new`, and so does every `.rekey` file that names
/// `old`. The files themselves are re-encrypted after the commit
/// ([`Core::recordings_resume`]); until then, and after a failure or a
/// crash, the `.rekey` file keeps them readable and the re-encryption
/// continues at the next unlock. An error here stops the merge before
/// anything changed.
pub(super) fn prepare_rekeys(store: &Store, old: &KeyRing, new: &KeyRing) -> Result<(), CoreError> {
    let Some(dir) = recordings_dir(store) else {
        return Ok(());
    };
    let entries = match std::fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(io(error)),
    };
    for entry in entries {
        let path = entry.map_err(io)?.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some(recording::EXTENSION) {
            continue;
        }
        let Some(header) = header_of(&path) else {
            continue; // Not a recording this app can read; left as it is.
        };
        let file_id = header.file_id;
        let key = if header.vault_id == old.vault_id() {
            old.key(header.key_epoch)
                .map(|vault_key| crypto::derive_recording_key(vault_key, &file_id))
        } else {
            // A re-encryption into `old` that has not run yet.
            read_rekey(&dir, file_id, old.vault_id()).and_then(|rekey| rekey.open(old))
        };
        let Some(key) = key else {
            continue;
        };
        let bytes = Rekey::seal(file_id, new, &key)?;
        write_synced(&rekey_path(&dir, file_id, new.vault_id()), &bytes).map_err(io)?;
    }
    Ok(())
}

/// One recording to re-encrypt.
struct Job {
    path: PathBuf,
    rekey: PathBuf,
    old_key: crypto::Key32,
    header: Header,
    new_key: crypto::Key32,
}

/// The `.rekey` files and what to do with them under the current personal
/// vault `ring`: jobs for the recordings to re-encrypt, and the files to
/// remove (done already, the recording gone, or left by a merge that did
/// not commit).
fn rekey_jobs(dir: &Path, ring: &KeyRing) -> (Vec<Job>, Vec<PathBuf>) {
    let mut jobs = Vec::new();
    let mut stale = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return (jobs, stale);
    };
    for entry in entries.flatten() {
        let rekey_file = entry.path();
        let extension = rekey_file.extension().and_then(|ext| ext.to_str());
        if extension.is_some_and(|ext| {
            ext == "rekey-new" || ext.starts_with(&format!("{}-", recording::TEMP_EXTENSION))
        }) {
            // A temporary file a crash left (no other re-encryption runs).
            stale.push(rekey_file);
            continue;
        }
        if extension != Some(REKEY_EXTENSION) {
            continue;
        }
        let Some(rekey) = std::fs::read(&rekey_file)
            .ok()
            .and_then(|bytes| Rekey::parse(&bytes))
        else {
            continue; // Unreadable: kept, never acted on.
        };
        let path = dir.join(format!("{}.{}", rekey.file_id, recording::EXTENSION));
        let header = header_of(&path);
        let done = match &header {
            Some(header) => header.vault_id == ring.vault_id(),
            // Gone (deleted with its log entry); a file that is there but
            // does not parse keeps its key.
            None => matches!(path.try_exists(), Ok(false)),
        };
        if done {
            stale.push(rekey_file);
            continue;
        }
        if rekey.vault_id != ring.vault_id() || header.is_none() {
            // For another vault (a merge that did not commit): kept until
            // the file is done.
            continue;
        }
        let Some(old_key) = rekey.open(ring) else {
            continue;
        };
        let Ok(new_header) = Header::new(rekey.file_id, ring.vault_id(), ring.current_epoch())
        else {
            continue;
        };
        jobs.push(Job {
            path,
            rekey: rekey_file,
            old_key,
            header: new_header,
            new_key: crypto::derive_recording_key(ring.current_key(), &rekey.file_id),
        });
    }
    (jobs, stale)
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
    /// Re-encrypts the recordings that a merge at sign-in left under an
    /// earlier personal vault's key (storage-format.md 8.4): after the merge
    /// and at every unlock, so an interrupted re-encryption continues.
    /// Files still being written are left for the next run. Only the list
    /// of jobs is made under the core's lock; the files are re-encrypted
    /// without it, one run at a time, and a lock stops the run.
    pub(super) fn recordings_resume(&self) {
        let Ok(_running) = self.rekeying.try_lock() else {
            return; // Another run goes through the same files.
        };
        let (jobs, stale) = {
            let mut guard = self.inner();
            let Ok(parts) = parts(&mut guard) else {
                return;
            };
            let Some(dir) = recordings_dir(parts.store) else {
                return;
            };
            rekey_jobs(&dir, &parts.session.ring)
        };
        for path in stale {
            let _ = std::fs::remove_file(path);
        }
        let mut done = 0usize;
        for job in jobs {
            if self.inner().session.is_none() {
                break; // Locked: the keys are gone from the session too.
            }
            if self.open_recordings.contains(&job.header.file_id) {
                continue;
            }
            match recording::rekey_file(&job.path, &job.old_key, &job.header, &job.new_key) {
                Ok(()) => {
                    done += 1;
                    let _ = std::fs::remove_file(&job.rekey);
                }
                Err(error) => tracing::warn!(%error, "recording not re-encrypted yet"),
            }
        }
        if done > 0 {
            tracing::info!(done, "recordings re-encrypted");
        }
    }

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
        let mut writer =
            RecordingWriter::create(&path, header, key, size, unix).map_err(recording_error)?;
        writer.track(&self.open_recordings);
        tracing::info!("session recording started");
        Ok(writer)
    }

    /// The recording of log entry `log_id`, opened (UI 11.2): `None` when
    /// the session was not recorded. A file that is not on this device is
    /// [`CoreError::NotFound`] ("recording not available on this device").
    pub fn recording_read(&self, log_id: Uuid) -> Result<Option<Recording>, CoreError> {
        // The core is locked only to look up the file and its key: reading
        // and decrypting a long recording would hold up every other call.
        let (file_id, path) = {
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
            (file_id, path)
        };
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Err(CoreError::NotFound);
            }
            Err(error) => return Err(io(error)),
        };
        let header = Header::parse(&bytes).map_err(recording_error)?;
        let key = {
            let mut guard = self.inner();
            let parts = parts(&mut guard)?;
            let ring = &parts.session.ring;
            if header.file_id != file_id {
                None
            } else if header.vault_id == ring.vault_id() {
                ring.key(header.key_epoch)
                    .map(|vault_key| crypto::derive_recording_key(vault_key, &file_id))
            } else {
                // Not re-encrypted into this vault yet (storage 8.4).
                recordings_dir(parts.store)
                    .and_then(|dir| read_rekey(&dir, file_id, ring.vault_id()))
                    .and_then(|rekey| rekey.open(ring))
            }
        };
        let recording = recording::read(&bytes, |_| key).map_err(recording_error)?;
        Ok(Some(recording))
    }

    /// "Export to .txt" (UI 11.2): the recording of `log_id` as plain text
    /// (escape sequences stripped) written to `path`. The file is not
    /// encrypted: the interface warns first (D16).
    pub fn recording_export_txt(&self, log_id: Uuid, path: &Path) -> Result<(), CoreError> {
        let recording = self.recording_read(log_id)?.ok_or(CoreError::NotFound)?;
        let text = zeroize::Zeroizing::new(recording.plain_text());
        shellpp_private_file::write(
            path,
            text.as_bytes(),
            shellpp_private_file::Existing::Replace,
        )
        .map_err(io)
    }
}
