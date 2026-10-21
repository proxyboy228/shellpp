//! Open and Open with for a remote file (night plan, stage 4, item 6;
//! UI 5.4; night decision D49), without the program launch and the
//! notifications, which belong to the app.
//!
//! - [`OpenedCopy::open`] downloads the file into a folder of its own under
//!   `root` (the app passes a folder inside its data folder), under its own
//!   name (made safe for Windows), so the program sees the right extension.
//!   On Unix the folder has mode 0700: the copy is not encrypted.
//! - [`OpenedCopy::changed_locally`]: the program saved it (its size or
//!   modification time is not the one written).
//! - [`OpenedCopy::upload_back`] uploads a changed copy. When the file on
//!   the server changed since the copy was made (its size or modification
//!   time), nothing is written and [`UploadOutcome::RemoteChanged`] asks the
//!   user first; `force` overwrites.
//! - [`OpenedCopy::discard`] removes the copy and its folder (tab closed,
//!   program done); [`discard_all`] removes every copy left in `root` (app
//!   exit, or the next start after a crash).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::SystemTime;

use crate::sftp::{EntryKind, Sftp, SftpError};

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

/// A remote file opened in a local program.
#[derive(Debug)]
pub struct OpenedCopy {
    remote: String,
    local: PathBuf,
    folder: PathBuf,
    /// The server's size and modification time the copy is based on.
    remote_size: u64,
    remote_modified: Option<u64>,
    /// The copy as written by us: length and modification time.
    written_len: u64,
    written_at: Option<SystemTime>,
}

fn local_io(error: std::io::Error) -> SftpError {
    SftpError::Local(error.kind().to_string())
}

/// A folder of its own for one copy: `<root>/<process>-<n>-<time>`.
fn new_folder(root: &Path) -> std::io::Result<PathBuf> {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    std::fs::create_dir_all(root)?;
    loop {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let nanos = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map_or(0, |since| since.subsec_nanos());
        let folder = root.join(format!("{}-{n}-{nanos:x}", std::process::id()));
        match std::fs::create_dir(&folder) {
            Ok(()) => {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt as _;
                    std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o700))?;
                }
                return Ok(folder);
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
}

fn local_state(path: &Path) -> std::io::Result<(u64, Option<SystemTime>)> {
    let meta = std::fs::metadata(path)?;
    Ok((meta.len(), meta.modified().ok()))
}

impl OpenedCopy {
    /// Downloads `remote` into a new folder under `root`.
    pub async fn open(sftp: &Sftp, remote: &str, root: &Path) -> Result<Self, SftpError> {
        let entry = sftp.stat(remote).await?;
        let is_file = match &entry.kind {
            EntryKind::File => true,
            EntryKind::Symlink { to_dir, .. } => !to_dir,
            _ => false,
        };
        if !is_file {
            return Err(SftpError::NotAFile);
        }
        // A link: the copy is of the file it points to, and an upload goes
        // there (it never replaces the link with a file).
        let remote = match entry.kind {
            EntryKind::Symlink { .. } => sftp.canonicalize(remote).await?,
            _ => remote.to_owned(),
        };
        let remote = remote.as_str();
        let name = remote.rsplit('/').next().unwrap_or(remote);
        let name = if cfg!(windows) {
            crate::local_names::windows_safe_name(name).unwrap_or_else(|| name.to_owned())
        } else {
            name.to_owned()
        };
        let folder = new_folder(root).map_err(local_io)?;
        let local = folder.join(&name);
        if let Err(error) = sftp.download(remote, &local, false, &mut |_| {}).await {
            let _ = std::fs::remove_dir_all(&folder);
            return Err(error);
        }
        let target = sftp.stat_target(remote).await?;
        let (written_len, written_at) = local_state(&local).map_err(local_io)?;
        Ok(Self {
            remote: remote.to_owned(),
            local,
            folder,
            remote_size: target.size,
            remote_modified: target.modified,
            written_len,
            written_at,
        })
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
            Ok((len, at)) => len != self.written_len || at != self.written_at,
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
        let now = sftp.stat_target(&self.remote).await?;
        if !force && (now.size != self.remote_size || now.modified != self.remote_modified) {
            return Ok(UploadOutcome::RemoteChanged);
        }
        sftp.upload(&self.local, &self.remote, false, &mut |_| {})
            .await?;
        let after = sftp.stat_target(&self.remote).await?;
        self.remote_size = after.size;
        self.remote_modified = after.modified;
        let (len, at) = local_state(&self.local).map_err(local_io)?;
        self.written_len = len;
        self.written_at = at;
        Ok(UploadOutcome::Uploaded)
    }

    /// Removes the copy and its folder.
    pub fn discard(self) -> std::io::Result<()> {
        std::fs::remove_dir_all(&self.folder)
    }
}

/// Removes every copy left in `root` (app exit; the next start after a
/// crash). A missing `root` is fine.
pub fn discard_all(root: &Path) -> std::io::Result<()> {
    match std::fs::remove_dir_all(root) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        other => other,
    }
}
