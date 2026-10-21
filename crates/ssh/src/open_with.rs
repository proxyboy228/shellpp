//! Open and Open with for a remote file (night plan, stage 4, item 6;
//! UI 5.4; night decision D49), without the program launch and the
//! notifications, which belong to the app.
//!
//! - [`OpenedCopy::open`] downloads the file into a folder of its own under
//!   `root` (the app passes a folder inside its data folder), under its own
//!   name (made safe for Windows), so the program sees the right extension.
//!   On Unix the folder is created with mode 0700: the copy is not
//!   encrypted. The server's state is read before and after the download;
//!   a file that changed meanwhile is downloaded again.
//! - [`OpenedCopy::changed_locally`]: the program saved it (its size or
//!   modification time is not the one written).
//! - [`OpenedCopy::upload_back`] uploads a changed copy. When the file on
//!   the server changed since the copy was made (its size or modification
//!   time; for a file of up to [`COMPARED_BYTES`] also its content), nothing
//!   is written and [`UploadOutcome::RemoteChanged`] asks the user first;
//!   `force` overwrites (and recreates a file deleted on the server). What
//!   is recorded as uploaded is the copy as it was when the upload started,
//!   so a save during the upload is uploaded next time.
//! - [`OpenedCopy::discard`] removes the copy and its folder when the file
//!   is closed, but never a copy with changes that are not uploaded, unless
//!   forced. Each folder holds a small note of what was written, so after a
//!   crash [`discard_all`] removes the copies that hold nothing new and
//!   returns the others ([`Leftover`]) for the user to upload or drop.
//!
//! Limits: the check and the upload are not one step on the server, so a
//! change made on the server during the upload of a large file is
//! overwritten; a file over [`COMPARED_BYTES`] changed on the server with
//! the same size within the same second is not noticed.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tokio::io::AsyncReadExt as _;

use crate::sftp::{EntryKind, RemoteEntry, Sftp, SftpError};

/// Files up to this size are also compared by content before an upload.
pub const COMPARED_BYTES: u64 = 1024 * 1024;
/// The note of a copy, next to it in its folder.
const NOTE: &str = ".shellpp-open";
/// Downloads tried while the server's file keeps changing.
const DOWNLOAD_TRIES: usize = 3;

/// What [`OpenedCopy::upload_back`] did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UploadOutcome {
    /// The copy is as it was written: nothing to upload.
    Unchanged,
    /// Uploaded; the notification of UI 1.5 follows.
    Uploaded,
    /// The file on the server changed since the copy was made: nothing was
    /// written. Asking the user, then `force`, overwrites it.
    RemoteChanged,
}

/// The server's state a copy is based on.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Baseline {
    size: u64,
    modified: Option<u64>,
    /// The content, for a file of up to [`COMPARED_BYTES`].
    content: Option<Vec<u8>>,
}

/// A remote file opened in a local program.
#[derive(Debug)]
pub struct OpenedCopy {
    remote: String,
    local: PathBuf,
    folder: PathBuf,
    baseline: Baseline,
    /// The copy as written (or uploaded): length and modification time.
    written: (u64, Option<SystemTime>),
}

fn local_io(error: std::io::Error) -> SftpError {
    SftpError::Local(error.kind().to_string())
}

/// Whether `name` is a folder name [`new_folder`] makes.
fn is_copy_folder(name: &str) -> bool {
    let mut parts = name.split('-');
    let ok = |part: Option<&str>, hex: bool| {
        part.is_some_and(|part| {
            !part.is_empty()
                && part.chars().all(|c| {
                    if hex {
                        c.is_ascii_hexdigit()
                    } else {
                        c.is_ascii_digit()
                    }
                })
        })
    };
    ok(parts.next(), false)
        && ok(parts.next(), false)
        && ok(parts.next(), true)
        && parts.next().is_none()
}

/// A folder of its own for one copy: `<root>/<process>-<n>-<time>`.
fn new_folder(root: &Path) -> std::io::Result<PathBuf> {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    std::fs::create_dir_all(root)?;
    loop {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| since.subsec_nanos());
        let folder = root.join(format!("{}-{n}-{nanos:x}", std::process::id()));
        let builder = {
            #[cfg_attr(not(unix), allow(unused_mut))]
            let mut builder = std::fs::DirBuilder::new();
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt as _;
                builder.mode(0o700);
            }
            builder
        };
        match builder.create(&folder) {
            Ok(()) => return Ok(folder),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
}

fn local_state(path: &Path) -> std::io::Result<(u64, Option<SystemTime>)> {
    let meta = std::fs::metadata(path)?;
    Ok((meta.len(), meta.modified().ok()))
}

fn nanos(time: Option<SystemTime>) -> String {
    time.and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map_or_else(|| "-".to_owned(), |since| since.as_nanos().to_string())
}

fn from_nanos(text: &str) -> Option<SystemTime> {
    let nanos: u128 = text.parse().ok()?;
    let secs = u64::try_from(nanos / 1_000_000_000).ok()?;
    let rest = u32::try_from(nanos % 1_000_000_000).ok()?;
    Some(UNIX_EPOCH + Duration::new(secs, rest))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn unhex(text: &str) -> Option<String> {
    let bytes: Option<Vec<u8>> = (0..text.len())
        .step_by(2)
        .map(|at| u8::from_str_radix(text.get(at..at + 2)?, 16).ok())
        .collect();
    String::from_utf8(bytes?).ok()
}

/// The note: the remote path (hex), the copy's file name (hex) and what
/// was written (length, modification time in nanoseconds).
fn write_note(folder: &Path, remote: &str, local: &Path, written: (u64, Option<SystemTime>)) {
    let name = local
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let text = format!(
        "{}\n{}\n{}\n{}\n",
        hex(remote.as_bytes()),
        hex(name.as_bytes()),
        written.0,
        nanos(written.1)
    );
    // Best effort: without it a leftover copy is kept, never wiped.
    let _ = std::fs::write(folder.join(NOTE), text);
}

/// What the server holds at `remote` now; `None` when there is nothing.
async fn server_state(sftp: &Sftp, remote: &str) -> Result<Option<RemoteEntry>, SftpError> {
    match sftp.stat_target(remote).await {
        Ok(entry) => Ok(Some(entry)),
        Err(SftpError::NotFound) => Ok(None),
        Err(error) => Err(error),
    }
}

async fn read_small(sftp: &Sftp, remote: &str, size: u64) -> Result<Option<Vec<u8>>, SftpError> {
    if size > COMPARED_BYTES {
        return Ok(None);
    }
    let mut file = sftp.open_read(remote).await?;
    let mut content = Vec::with_capacity(size as usize);
    file.read_to_end(&mut content)
        .await
        .map_err(|error| sftp.file_error(error))?;
    Ok(Some(content))
}

impl OpenedCopy {
    /// Downloads `remote` into a new folder under `root`.
    pub async fn open(sftp: &Sftp, remote: &str, root: &Path) -> Result<Self, SftpError> {
        let entry = sftp.stat(remote).await?;
        let remote = match entry.kind {
            EntryKind::File => remote.to_owned(),
            EntryKind::Symlink { to_dir: false, .. } => {
                // The copy is of the file the link points to, and an upload
                // goes there: the link stays a link. A server whose realpath
                // does not follow links leaves a link: refused.
                let target = sftp.canonicalize(remote).await?;
                if sftp.stat(&target).await?.kind != EntryKind::File {
                    return Err(SftpError::NotAFile);
                }
                target
            }
            _ => return Err(SftpError::NotAFile),
        };
        let name = remote.rsplit('/').next().unwrap_or(&remote);
        let name = if cfg!(windows) {
            crate::local_names::windows_safe_name(name).unwrap_or_else(|| name.to_owned())
        } else {
            name.to_owned()
        };
        let folder = new_folder(root).map_err(local_io)?;
        let local = crate::local_names::long_path(&folder.join(&name));
        let made = async {
            for _ in 0..DOWNLOAD_TRIES {
                let before = sftp.stat_target(&remote).await?;
                sftp.download(&remote, &local, false, &mut |_| {}).await?;
                let after = sftp.stat_target(&remote).await?;
                if before.size == after.size && before.modified == after.modified {
                    let content = read_small(sftp, &remote, after.size).await?;
                    // The content read back must be the one downloaded.
                    if let Some(content) = &content
                        && std::fs::read(&local).map_err(local_io)? != *content
                    {
                        continue;
                    }
                    let written = local_state(&local).map_err(local_io)?;
                    return Ok((
                        Baseline {
                            size: after.size,
                            modified: after.modified,
                            content,
                        },
                        written,
                    ));
                }
            }
            Err(SftpError::Changed)
        };
        match made.await {
            Ok((baseline, written)) => {
                write_note(&folder, &remote, &local, written);
                Ok(Self {
                    remote,
                    local,
                    folder,
                    baseline,
                    written,
                })
            }
            Err(error) => {
                let _ = std::fs::remove_dir_all(&folder);
                Err(error)
            }
        }
    }

    /// The copy the program opens.
    pub fn local_path(&self) -> &Path {
        &self.local
    }

    /// The remote file of the copy.
    pub fn remote_path(&self) -> &str {
        &self.remote
    }

    /// Whether the program saved the copy since it was written (or
    /// uploaded).
    pub fn changed_locally(&self) -> bool {
        match local_state(&self.local) {
            Ok(state) => state != self.written,
            Err(_) => false,
        }
    }

    /// Uploads a changed copy, unless the server's file changed since the
    /// copy was made and `force` is off.
    pub async fn upload_back(
        &mut self,
        sftp: &Sftp,
        force: bool,
    ) -> Result<UploadOutcome, SftpError> {
        if !self.changed_locally() {
            return Ok(UploadOutcome::Unchanged);
        }
        match server_state(sftp, &self.remote).await? {
            Some(now) if now.kind == EntryKind::Dir => return Err(SftpError::NotAFile),
            Some(now) if !force => {
                let moved =
                    now.size != self.baseline.size || now.modified != self.baseline.modified;
                let rewritten = match &self.baseline.content {
                    Some(content) if !moved => {
                        read_small(sftp, &self.remote, now.size).await?.as_ref() != Some(content)
                    }
                    _ => false,
                };
                if moved || rewritten {
                    return Ok(UploadOutcome::RemoteChanged);
                }
            }
            // Deleted on the server: only an explicit overwrite recreates it.
            None if !force => return Ok(UploadOutcome::RemoteChanged),
            _ => {}
        }
        // The copy as it is now is what goes up: a save during the upload
        // stays "changed" and goes up next time.
        let sent = local_state(&self.local).map_err(local_io)?;
        let content = if sent.0 <= COMPARED_BYTES {
            Some(std::fs::read(&self.local).map_err(local_io)?)
        } else {
            None
        };
        sftp.upload(&self.local, &self.remote, false, &mut |_| {})
            .await?;
        let after = sftp.stat_target(&self.remote).await?;
        self.baseline = Baseline {
            size: after.size,
            modified: after.modified,
            // What we sent, if the server holds that much.
            content: content.filter(|content| content.len() as u64 == after.size),
        };
        self.written = sent;
        write_note(&self.folder, &self.remote, &self.local, self.written);
        Ok(UploadOutcome::Uploaded)
    }

    /// Removes the copy and its folder when the file is closed. A copy with
    /// changes that are not uploaded is kept (and returned) unless `force`;
    /// so is one whose folder cannot be removed now.
    pub fn discard(self, force: bool) -> Result<(), Box<Self>> {
        if !force && self.changed_locally() {
            return Err(Box::new(self));
        }
        match std::fs::remove_dir_all(&self.folder) {
            Ok(()) => Ok(()),
            Err(_) => Err(Box::new(self)),
        }
    }
}

/// A copy left in the root (the app ended without closing it) whose file
/// changed after it was written: the user uploads it or drops it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Leftover {
    pub folder: PathBuf,
    pub local: PathBuf,
    /// The remote path, when the note could be read.
    pub remote: Option<String>,
}

/// At exit, or at the next start after a crash: removes every copy in
/// `root` that holds nothing new (only folders this module made, never the
/// root itself or anything else in it) and returns the others.
pub fn discard_all(root: &Path) -> std::io::Result<Vec<Leftover>> {
    let entries = match std::fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    let mut kept = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !is_copy_folder(&name) || !entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            continue;
        }
        let folder = entry.path();
        let note = std::fs::read_to_string(folder.join(NOTE)).ok();
        let mut lines = note.as_deref().unwrap_or_default().lines();
        let remote = lines.next().and_then(unhex);
        let file = lines.next().and_then(unhex);
        let written = (
            lines.next().and_then(|len| len.parse::<u64>().ok()),
            lines.next().map(from_nanos),
        );
        let Some(local) = file.map(|file| folder.join(file)) else {
            // No note: kept for the user, never wiped on a guess.
            kept.push(Leftover {
                local: folder.clone(),
                folder,
                remote: None,
            });
            continue;
        };
        let unchanged = match (written, local_state(&local)) {
            ((Some(len), Some(at)), Ok(now)) => now == (len, at),
            // The copy is gone: nothing to keep.
            (_, Err(_)) => true,
            _ => false,
        };
        if unchanged {
            std::fs::remove_dir_all(&folder)?;
        } else {
            kept.push(Leftover {
                folder,
                local,
                remote,
            });
        }
    }
    Ok(kept)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_folders_made_here_are_copy_folders() {
        assert!(is_copy_folder("1234-0-1a2b3c"));
        assert!(!is_copy_folder("1234-0"));
        assert!(!is_copy_folder("Documents"));
        assert!(!is_copy_folder("12-3-zz"));
        assert!(!is_copy_folder("1-2-3-4"));
    }

    #[test]
    fn notes_read_back() {
        assert_eq!(
            unhex(&hex("/home/u/отчёт.txt".as_bytes())).as_deref(),
            Some("/home/u/отчёт.txt")
        );
        let time = UNIX_EPOCH + Duration::new(1_700_000_000, 123_456_789);
        assert_eq!(from_nanos(&nanos(Some(time))), Some(time));
        assert_eq!(from_nanos("-"), None);
    }

    #[test]
    fn leftovers_with_changes_are_kept_and_the_rest_removed() {
        let root = tempfile::tempdir().unwrap();
        // Something of the user's in the root is never touched.
        std::fs::create_dir(root.path().join("Documents")).unwrap();
        let unchanged = new_folder(root.path()).unwrap();
        let file = unchanged.join("a.txt");
        std::fs::write(&file, "a").unwrap();
        write_note(&unchanged, "/srv/a.txt", &file, local_state(&file).unwrap());
        let changed = new_folder(root.path()).unwrap();
        let edited = changed.join("b.txt");
        std::fs::write(&edited, "b").unwrap();
        write_note(&changed, "/srv/b.txt", &edited, (99, None));
        let kept = discard_all(root.path()).unwrap();
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].remote.as_deref(), Some("/srv/b.txt"));
        assert_eq!(kept[0].local, edited);
        assert!(!unchanged.exists());
        assert!(root.path().join("Documents").exists());
        assert!(
            discard_all(&root.path().join("missing"))
                .unwrap()
                .is_empty()
        );
    }
}
