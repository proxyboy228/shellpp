//! SFTP over an open connection (night plan, stage 4, item 1; UI 5; night
//! decision D45).
//!
//! [`crate::Connection::open_sftp`] starts the `sftp` subsystem on a new
//! channel of a connection that a terminal tab may already use, so a second
//! login is never needed; the session keeps that connection alive while it
//! is open. About 2 MiB are in flight per file each way (russh-sftp's
//! pipelining: 8 reads of up to 255 KiB, 64 writes of 32 KiB): without that
//! SFTP is slow on a high ping, with more a slow link times out.
//!
//! - [`Sftp::list`]: the entries of a folder, `.` and `..` left out; a
//!   symbolic link is reported as a link with its target and whether that
//!   is a folder, never as what it points to.
//! - [`Sftp::download`] and [`Sftp::upload`]: into `<name>.part` first,
//!   renamed when complete; with `resume`, a `.part` left by an interrupted
//!   transfer is continued from its length once its last block matches the
//!   source (else the transfer starts over). The modification time is kept
//!   where the server allows; a new upload's permissions follow the server's
//!   umask, a replaced file keeps its own.
//! - [`Sftp::remove_tree`]: a recursive delete that removes a symbolic link
//!   itself and never what it points to.
//! - A name that is not UTF-8 on the server is shown with U+FFFD (russh-sftp
//!   decodes names lossily); such a file is listed but cannot be opened.
//!
//! Nothing here logs paths or file content.

use std::io::SeekFrom;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, UNIX_EPOCH};

use russh_sftp::client::SftpSession;
use russh_sftp::client::error::Error as LibError;
use russh_sftp::client::fs::File as RemoteFile;
use russh_sftp::protocol::{FileAttributes, OpenFlags, StatusCode};
use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncSeekExt as _, AsyncWrite, AsyncWriteExt as _};

use crate::client::ConnCore;

/// The buffer of one copy step (several SFTP requests: russh-sftp splits it
/// and keeps them in flight).
const COPY_BUFFER: usize = 256 * 1024;
/// Reads in flight per file. A read asks for as much as the server allows
/// (OpenSSH: 255 KiB), so 8 keep about 2 MiB on the way.
const READS_IN_FLIGHT: usize = 8;
/// Writes in flight per file, 32 KiB each: about 2 MiB, as OpenSSH's
/// `sftp -R 64` (russh-sftp's default of 16 halves an upload over a 50 ms
/// link).
const WRITES_IN_FLIGHT: usize = 64;
/// How long one request may wait for its answer. Requests are answered in
/// order, so a request waits behind the 2 MiB in flight: 60 s still lets a
/// 300 kbit/s link through, for one transfer per session (parallel
/// transfers each take their own session, D46). A dead link ends sooner
/// through the connection's keepalive.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);
/// Links resolved at once while listing a folder.
const LINKS_AT_ONCE: usize = 16;
/// The block a resumed transfer compares before it goes on.
const RESUME_CHECK: u64 = 64 * 1024;

fn session_config() -> russh_sftp::client::Config {
    russh_sftp::client::Config {
        max_concurrent_reads: READS_IN_FLIGHT,
        max_concurrent_writes: WRITES_IN_FLIGHT,
        request_timeout_secs: REQUEST_TIMEOUT.as_secs(),
        ..russh_sftp::client::Config::default()
    }
}

/// The suffix of a file while it is transferred.
pub const PART_SUFFIX: &str = ".part";

/// Why an SFTP operation failed (UI 5: clear errors).
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SftpError {
    #[error("no such file or folder")]
    NotFound,
    #[error("permission denied")]
    PermissionDenied,
    /// Any other refusal of the server (SFTP v3 has no own code for a full
    /// disk: it is a failure with the server's message).
    #[error("the server refused: {0}")]
    Failure(String),
    #[error("the connection was lost")]
    ConnectionLost,
    #[error("the server did not answer in time")]
    Timeout,
    /// A local file could not be read or written.
    #[error("local file: {0}")]
    Local(String),
    /// The path is a folder where a file is needed, or the other way round.
    #[error("not a file")]
    NotAFile,
    /// The file changed size while it was copied.
    #[error("the file changed during the transfer")]
    Changed,
}

impl From<&LibError> for SftpError {
    fn from(error: &LibError) -> Self {
        match error {
            LibError::Status(status) => match status.status_code {
                StatusCode::NoSuchFile => SftpError::NotFound,
                StatusCode::PermissionDenied => SftpError::PermissionDenied,
                StatusCode::NoConnection | StatusCode::ConnectionLost => SftpError::ConnectionLost,
                _ => SftpError::Failure(status.error_message.clone()),
            },
            LibError::Timeout => SftpError::Timeout,
            LibError::IO(_) => SftpError::ConnectionLost,
            // The session's channel is gone (russh-sftp's words for it).
            LibError::UnexpectedBehavior(message)
                if ["closed", "dropped", "SendError"]
                    .iter()
                    .any(|word| message.contains(word)) =>
            {
                SftpError::ConnectionLost
            }
            other => SftpError::Failure(other.to_string()),
        }
    }
}

impl From<LibError> for SftpError {
    fn from(error: LibError) -> Self {
        Self::from(&error)
    }
}

/// An I/O error of a remote file. russh-sftp wraps its own error (a lost
/// channel, a timeout) in the `io::Error`, and a status as
/// `io::Error::other` with the server's message.
fn remote_io(error: std::io::Error) -> SftpError {
    if let Some(inner) = error
        .get_ref()
        .and_then(|inner| inner.downcast_ref::<LibError>())
    {
        return SftpError::from(inner);
    }
    match error.kind() {
        std::io::ErrorKind::NotFound => SftpError::NotFound,
        std::io::ErrorKind::PermissionDenied => SftpError::PermissionDenied,
        std::io::ErrorKind::TimedOut => SftpError::Timeout,
        std::io::ErrorKind::BrokenPipe
        | std::io::ErrorKind::ConnectionAborted
        | std::io::ErrorKind::ConnectionReset
        | std::io::ErrorKind::UnexpectedEof => SftpError::ConnectionLost,
        _ => SftpError::Failure(error.to_string()),
    }
}

fn local_io(error: std::io::Error) -> SftpError {
    SftpError::Local(error.kind().to_string())
}

/// What an entry is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EntryKind {
    File,
    Dir,
    /// A symbolic link: its target (when it can be read) and whether the
    /// target is a folder.
    Symlink {
        target: Option<String>,
        to_dir: bool,
    },
    Other,
}

/// One entry of a remote folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteEntry {
    pub name: String,
    pub kind: EntryKind,
    pub size: u64,
    /// Modification time, Unix seconds.
    pub modified: Option<u64>,
    /// Permission bits (`0o7777`).
    pub permissions: Option<u32>,
}

/// `dir` and `name` joined with `/` (remote paths are POSIX).
pub fn join(dir: &str, name: &str) -> String {
    match dir {
        "" => name.to_owned(),
        _ if dir.ends_with('/') => format!("{dir}{name}"),
        _ => format!("{dir}/{name}"),
    }
}

/// `path` without trailing slashes (`/` stays): `lstat("link/")` would
/// follow the link.
fn trimmed(path: &str) -> &str {
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() && path.starts_with('/') {
        "/"
    } else {
        trimmed
    }
}

/// The local path of a file being downloaded to `path`.
pub fn local_part(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(PART_SUFFIX);
    path.with_file_name(name)
}

fn kind_of(attrs: &FileAttributes) -> EntryKind {
    let file_type = attrs.file_type();
    if file_type.is_symlink() {
        EntryKind::Symlink {
            target: None,
            to_dir: false,
        }
    } else if file_type.is_dir() {
        EntryKind::Dir
    } else if file_type.is_file() {
        EntryKind::File
    } else {
        EntryKind::Other
    }
}

fn entry(name: String, attrs: &FileAttributes) -> RemoteEntry {
    RemoteEntry {
        name,
        kind: kind_of(attrs),
        size: attrs.size.unwrap_or(0),
        modified: attrs.mtime.map(u64::from),
        permissions: attrs.permissions.map(|mode| mode & 0o7777),
    }
}

/// Copies `source` into `target` from `done` bytes on, reporting the bytes
/// done after each step.
async fn copy<R, W>(
    source: &mut R,
    target: &mut W,
    mut done: u64,
    progress: &mut (dyn FnMut(u64) + Send),
    read_error: &(dyn Fn(std::io::Error) -> SftpError + Sync),
    write_error: &(dyn Fn(std::io::Error) -> SftpError + Sync),
) -> Result<u64, SftpError>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut buffer = vec![0u8; COPY_BUFFER];
    loop {
        let read = source.read(&mut buffer).await.map_err(read_error)?;
        if read == 0 {
            break;
        }
        target
            .write_all(&buffer[..read])
            .await
            .map_err(write_error)?;
        done += read as u64;
        progress(done);
    }
    target.flush().await.map_err(write_error)?;
    Ok(done)
}

/// Reads exactly `buffer.len()` bytes from `offset` of `file`.
async fn read_at<F>(file: &mut F, offset: u64, buffer: &mut [u8]) -> std::io::Result<()>
where
    F: AsyncRead + tokio::io::AsyncSeek + Unpin,
{
    file.seek(SeekFrom::Start(offset)).await?;
    file.read_exact(buffer).await.map(|_| ())
}

/// An SFTP session on one channel of a [`crate::Connection`].
pub struct Sftp {
    session: SftpSession,
    /// The connection: kept alive while the session is open, and asked
    /// whether it is gone when a request fails.
    link: Arc<ConnCore>,
}

impl std::fmt::Debug for Sftp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Sftp").finish_non_exhaustive()
    }
}

impl Sftp {
    pub(crate) async fn start<S>(stream: S, link: Arc<ConnCore>) -> Result<Self, SftpError>
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        let session = SftpSession::new_with_config(stream, session_config()).await?;
        Ok(Self { session, link })
    }

    /// The error of a failed request: `ConnectionLost` once the connection
    /// is gone, whatever the library says.
    fn fail(&self, error: LibError) -> SftpError {
        if self.link.is_closed() {
            SftpError::ConnectionLost
        } else {
            error.into()
        }
    }

    fn fail_io(&self, error: std::io::Error) -> SftpError {
        if self.link.is_closed() {
            SftpError::ConnectionLost
        } else {
            remote_io(error)
        }
    }

    /// The absolute form of `path` on the server (`.` is the home folder).
    pub async fn canonicalize(&self, path: &str) -> Result<String, SftpError> {
        self.session
            .canonicalize(path)
            .await
            .map_err(|error| self.fail(error))
    }

    /// The entry at `path` (a symbolic link as a link).
    pub async fn stat(&self, path: &str) -> Result<RemoteEntry, SftpError> {
        let path = trimmed(path);
        let attrs = self
            .session
            .symlink_metadata(path)
            .await
            .map_err(|error| self.fail(error))?;
        let name = path.rsplit('/').next().unwrap_or(path).to_owned();
        let mut found = entry(name, &attrs);
        self.describe_link(path, &mut found).await;
        Ok(found)
    }

    /// The entry `path` points to (a symbolic link followed).
    pub async fn stat_target(&self, path: &str) -> Result<RemoteEntry, SftpError> {
        let path = trimmed(path);
        let attrs = self
            .session
            .metadata(path)
            .await
            .map_err(|error| self.fail(error))?;
        let name = path.rsplit('/').next().unwrap_or(path).to_owned();
        Ok(entry(name, &attrs))
    }

    async fn describe_link(&self, path: &str, found: &mut RemoteEntry) {
        if let EntryKind::Symlink { target, to_dir } = &mut found.kind {
            *target = self.session.read_link(path).await.ok();
            *to_dir = self
                .session
                .metadata(path)
                .await
                .is_ok_and(|attrs| attrs.file_type().is_dir());
        }
    }

    /// The entries of the folder `path`, `.` and `..` left out. The links
    /// among them are resolved [`LINKS_AT_ONCE`] at a time.
    pub async fn list(&self, path: &str) -> Result<Vec<RemoteEntry>, SftpError> {
        let path = trimmed(path);
        let mut out = Vec::new();
        for item in self
            .session
            .read_dir(path)
            .await
            .map_err(|error| self.fail(error))?
        {
            let name = item.file_name();
            if name == "." || name == ".." {
                continue;
            }
            out.push(entry(name, &item.metadata()));
        }
        let links: Vec<usize> = out
            .iter()
            .enumerate()
            .filter(|(_, found)| matches!(found.kind, EntryKind::Symlink { .. }))
            .map(|(at, _)| at)
            .collect();
        for batch in links.chunks(LINKS_AT_ONCE) {
            let resolved = futures::future::join_all(batch.iter().map(|at| {
                let mut found = out[*at].clone();
                async move {
                    self.describe_link(&join(path, &found.name), &mut found)
                        .await;
                    found.kind
                }
            }))
            .await;
            for (at, kind) in batch.iter().zip(resolved) {
                out[*at].kind = kind;
            }
        }
        Ok(out)
    }

    pub async fn create_dir(&self, path: &str) -> Result<(), SftpError> {
        self.session
            .create_dir(path)
            .await
            .map_err(|error| self.fail(error))
    }

    pub async fn remove_file(&self, path: &str) -> Result<(), SftpError> {
        self.session
            .remove_file(path)
            .await
            .map_err(|error| self.fail(error))
    }

    pub async fn remove_dir(&self, path: &str) -> Result<(), SftpError> {
        self.session
            .remove_dir(path)
            .await
            .map_err(|error| self.fail(error))
    }

    pub async fn rename(&self, from: &str, to: &str) -> Result<(), SftpError> {
        self.session
            .rename(from, to)
            .await
            .map_err(|error| self.fail(error))
    }

    /// Sets the permission bits of `path` (Edit Permissions, UI 5.4).
    pub async fn set_permissions(&self, path: &str, mode: u32) -> Result<(), SftpError> {
        let attrs = FileAttributes {
            permissions: Some(mode & 0o7777),
            ..FileAttributes::empty()
        };
        self.session
            .set_metadata(path, attrs)
            .await
            .map_err(|error| self.fail(error))
    }

    /// Edit Permissions with "apply recursively" (UI 5.4): `mode` on `path`
    /// and on every folder and file in it; links are left alone and never
    /// entered (a link's own mode means nothing, and following it would
    /// change what lies outside). Returns how many entries were changed.
    pub async fn set_permissions_tree(&self, path: &str, mode: u32) -> Result<usize, SftpError> {
        let path = trimmed(path);
        self.set_permissions(path, mode).await?;
        if !self.is_real_dir(path).await? {
            return Ok(1);
        }
        let mut changed = 1;
        let mut stack = vec![path.to_owned()];
        while let Some(dir) = stack.pop() {
            let items = self
                .session
                .read_dir(dir.as_str())
                .await
                .map_err(|error| self.fail(error))?;
            for item in items {
                let name = item.file_name();
                if name == "." || name == ".." || item.file_type().is_symlink() {
                    continue;
                }
                let child = join(&dir, &name);
                let folder = item.file_type().is_dir() && self.is_real_dir(&child).await?;
                if !folder && !item.file_type().is_file() {
                    continue;
                }
                self.set_permissions(&child, mode).await?;
                changed += 1;
                if folder {
                    stack.push(child);
                }
            }
        }
        Ok(changed)
    }

    /// Whether `path` is a folder and not a link, by its own lstat.
    async fn is_real_dir(&self, path: &str) -> Result<bool, SftpError> {
        let attrs = self
            .session
            .symlink_metadata(path)
            .await
            .map_err(|error| self.fail(error))?;
        let file_type = attrs.file_type();
        Ok(file_type.is_dir() && !file_type.is_symlink())
    }

    /// Removes `path` and, for a folder, everything in it; returns how many
    /// entries were removed. A symbolic link is removed itself: what it
    /// points to is never entered or removed. Each folder is checked with
    /// its own lstat before it is entered, so a server whose listing
    /// reports what a link points to cannot lead the delete through it.
    pub async fn remove_tree(&self, path: &str) -> Result<usize, SftpError> {
        let path = trimmed(path);
        if !self.is_real_dir(path).await? {
            self.remove_file(path).await?;
            return Ok(1);
        }
        let mut removed = 0;
        // (folder, its entries removed already)
        let mut stack = vec![(path.to_owned(), false)];
        while let Some((dir, emptied)) = stack.pop() {
            if emptied {
                self.remove_dir(&dir).await?;
                removed += 1;
                continue;
            }
            stack.push((dir.clone(), true));
            let items = self
                .session
                .read_dir(dir.as_str())
                .await
                .map_err(|error| self.fail(error))?;
            for item in items {
                let name = item.file_name();
                if name == "." || name == ".." {
                    continue;
                }
                let child = join(&dir, &name);
                if item.file_type().is_dir() && self.is_real_dir(&child).await? {
                    stack.push((child, false));
                } else {
                    self.remove_file(&child).await?;
                    removed += 1;
                }
            }
        }
        Ok(removed)
    }

    /// What a delete of `path` would remove, for the confirmation of UI
    /// 5.4 ("N files"): (files and links, folders), `path` included. Links
    /// are counted, never entered.
    pub async fn count_tree(&self, path: &str) -> Result<(usize, usize), SftpError> {
        let path = trimmed(path);
        if !self.is_real_dir(path).await? {
            return Ok((1, 0));
        }
        let (mut files, mut folders) = (0, 0);
        let mut stack = vec![path.to_owned()];
        while let Some(dir) = stack.pop() {
            folders += 1;
            let items = self
                .session
                .read_dir(dir.as_str())
                .await
                .map_err(|error| self.fail(error))?;
            for item in items {
                let name = item.file_name();
                if name == "." || name == ".." {
                    continue;
                }
                let child = join(&dir, &name);
                if item.file_type().is_dir() && self.is_real_dir(&child).await? {
                    stack.push(child);
                } else {
                    files += 1;
                }
            }
        }
        Ok((files, folders))
    }

    /// Whether the last block before `offset` of the remote `path` holds
    /// `local` (the same block of the local side). A remote file that is
    /// shorter does not; an error (a timeout, no permission) is returned, so
    /// a partial transfer is never thrown away on a guess.
    async fn tail_matches(&self, path: &str, offset: u64, local: &[u8]) -> Result<bool, SftpError> {
        let mut file = self
            .session
            .open(path)
            .await
            .map_err(|error| self.fail(error))?;
        let mut remote = vec![0u8; local.len()];
        let read = read_at(&mut file, offset - local.len() as u64, &mut remote).await;
        let _ = file.shutdown().await;
        match read {
            Ok(()) => Ok(remote == local),
            Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => Ok(false),
            Err(error) => Err(self.fail_io(error)),
        }
    }

    /// The permission bits of `path`, when it exists.
    pub(crate) async fn mode_of(&self, path: &str) -> Option<u32> {
        self.session
            .metadata(path)
            .await
            .ok()
            .and_then(|attrs| attrs.permissions)
            .map(|mode| mode & 0o7777)
    }

    /// Sets `mode` on an open file (fchmod: the server's umask and the
    /// mode of an older `.part` do not apply). Best effort.
    async fn set_mode(file: &RemoteFile, mode: Option<u32>) {
        if let Some(mode) = mode {
            let attrs = FileAttributes {
                permissions: Some(mode),
                ..FileAttributes::empty()
            };
            let _ = file.set_metadata(attrs).await;
        }
    }

    /// Downloads `remote` to `local` through `local.part`; with `resume` a
    /// `.part` that is there is continued when its last block matches.
    /// Returns the size.
    pub async fn download(
        &self,
        remote: &str,
        local: &Path,
        resume: bool,
        progress: &mut (dyn FnMut(u64) + Send),
    ) -> Result<u64, SftpError> {
        let attrs = self
            .session
            .metadata(remote)
            .await
            .map_err(|error| self.fail(error))?;
        if !attrs.file_type().is_file() {
            return Err(SftpError::NotAFile);
        }
        let size = attrs.size;
        let part = local_part(local);
        let mut offset = if resume {
            tokio::fs::metadata(&part)
                .await
                .map_or(0, |meta| meta.len())
        } else {
            0
        };
        if size.is_some_and(|size| offset > size) {
            offset = 0;
        }
        if offset > 0 {
            let check = offset.min(RESUME_CHECK);
            let mut tail = vec![0u8; check as usize];
            let mut file = tokio::fs::File::open(&part).await.map_err(local_io)?;
            read_at(&mut file, offset - check, &mut tail)
                .await
                .map_err(local_io)?;
            drop(file);
            if !self.tail_matches(remote, offset, &tail).await? {
                // Another file by now: start over.
                offset = 0;
            }
        }
        let mut source = self
            .session
            .open(remote)
            .await
            .map_err(|error| self.fail(error))?;
        if offset > 0 {
            source
                .seek(SeekFrom::Start(offset))
                .await
                .map_err(|error| self.fail_io(error))?;
        }
        // Written at `offset`, not appended: whatever lies past it (a write
        // of the cut transfer that landed late) is cut off first.
        let mut target = tokio::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(false)
            .open(&part)
            .await
            .map_err(local_io)?;
        target.set_len(offset).await.map_err(local_io)?;
        target
            .seek(SeekFrom::Start(offset))
            .await
            .map_err(local_io)?;
        progress(offset);
        let copied = copy(
            &mut source,
            &mut target,
            offset,
            progress,
            &|error| self.fail_io(error),
            &local_io,
        )
        .await;
        // What was copied is on disk before any error is reported, so a
        // resume measures the .part right.
        let _ = target.flush().await;
        let done = copied?;
        target.sync_all().await.map_err(local_io)?;
        drop(target);
        let _ = source.shutdown().await;
        let written = tokio::fs::metadata(&part).await.map_err(local_io)?.len();
        if written != done || size.is_some_and(|size| size != done) {
            return Err(SftpError::Changed);
        }
        if let Some(mtime) = attrs.mtime
            && let Ok(file) = std::fs::OpenOptions::new().write(true).open(&part)
        {
            // Best effort: the content is what counts.
            let _ = file.set_modified(UNIX_EPOCH + Duration::from_secs(u64::from(mtime)));
        }
        tokio::fs::rename(&part, local).await.map_err(local_io)?;
        Ok(done)
    }

    /// Uploads `local` to `remote` through `remote.part`; with `resume` a
    /// `.part` that is there is continued when its last block matches. An
    /// existing `remote` is replaced once the upload is complete and keeps
    /// its permissions. Returns the size.
    pub async fn upload(
        &self,
        local: &Path,
        remote: &str,
        resume: bool,
        progress: &mut (dyn FnMut(u64) + Send),
    ) -> Result<u64, SftpError> {
        let mut source = tokio::fs::File::open(local).await.map_err(local_io)?;
        let meta = source.metadata().await.map_err(local_io)?;
        if !meta.is_file() {
            return Err(SftpError::NotAFile);
        }
        let size = meta.len();
        let part = format!("{remote}{PART_SUFFIX}");
        let mut offset = if resume {
            self.session
                .metadata(part.as_str())
                .await
                .ok()
                .and_then(|attrs| attrs.size)
                .filter(|length| *length <= size)
                .unwrap_or(0)
        } else {
            0
        };
        if offset > 0 {
            let check = offset.min(RESUME_CHECK);
            let mut tail = vec![0u8; check as usize];
            read_at(&mut source, offset - check, &mut tail)
                .await
                .map_err(local_io)?;
            if !self.tail_matches(&part, offset, &tail).await? {
                offset = 0;
            }
        }
        // A file that is replaced keeps its permissions, from its .part on
        // (an edited 0600 file never sits there as 0644).
        let mode = self.mode_of(remote).await;
        let flags = if offset > 0 {
            OpenFlags::WRITE
        } else {
            OpenFlags::WRITE | OpenFlags::CREATE | OpenFlags::TRUNCATE
        };
        let attrs = FileAttributes {
            permissions: mode,
            ..FileAttributes::empty()
        };
        let mut target = self
            .session
            .open_with_flags_and_attributes(part.as_str(), flags, attrs)
            .await
            .map_err(|error| self.fail(error))?;
        // On the handle too: CREATE applies the umask, and an existing .part
        // keeps its own mode.
        Self::set_mode(&target, mode).await;
        if offset > 0 {
            target
                .seek(SeekFrom::Start(offset))
                .await
                .map_err(|error| self.fail_io(error))?;
        }
        source
            .seek(SeekFrom::Start(offset))
            .await
            .map_err(local_io)?;
        progress(offset);
        let done = copy(
            &mut source,
            &mut target,
            offset,
            progress,
            &local_io,
            &|error| self.fail_io(error),
        )
        .await?;
        target
            .shutdown()
            .await
            .map_err(|error| self.fail_io(error))?;
        drop(target);
        if done != size {
            return Err(SftpError::Changed);
        }
        if let Ok(modified) = meta.modified()
            && let Ok(since) = modified.duration_since(UNIX_EPOCH)
            && let Ok(seconds) = u32::try_from(since.as_secs())
        {
            let attrs = FileAttributes {
                atime: Some(seconds),
                mtime: Some(seconds),
                ..FileAttributes::empty()
            };
            // Best effort: a server may refuse SETSTAT (sftp-server -P).
            let _ = self.session.set_metadata(part.as_str(), attrs).await;
        }
        self.replace(&part, remote).await?;
        Ok(done)
    }

    /// Renames the complete `part` to `target`. SFTP v3 does not rename
    /// over a file: when the server refuses and `target` is there, `target`
    /// is first renamed aside, then the part takes its name, then the old
    /// one is removed; if the part cannot take the name, the old one gets
    /// it back. Any other error (a timeout, a lost connection) is returned
    /// as it is: nothing is removed on a guess.
    pub(crate) async fn replace(&self, part: &str, target: &str) -> Result<(), SftpError> {
        let refused = match self.session.rename(part, target).await {
            Ok(()) => return Ok(()),
            Err(LibError::Status(status)) if status.status_code == StatusCode::Failure => status,
            Err(error) => return Err(self.fail(error)),
        };
        let part_there = self.session.symlink_metadata(part).await.is_ok();
        let target_type = self
            .session
            .symlink_metadata(target)
            .await
            .ok()
            .map(|attrs| attrs.file_type());
        match target_type {
            // A folder (or a device) of that name is never moved aside.
            Some(kind) if !kind.is_file() && !kind.is_symlink() => {
                return Err(SftpError::NotAFile);
            }
            Some(_) if part_there => {}
            _ => return Err(SftpError::Failure(refused.error_message)),
        }
        let aside = format!(
            "{target}.{:04x}.old",
            std::time::SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |elapsed| elapsed.subsec_nanos() & 0xffff)
        );
        self.rename(target, &aside).await?;
        if let Err(error) = self.rename(part, target).await {
            if self.session.rename(aside.as_str(), target).await.is_err() {
                return Err(SftpError::Failure(format!(
                    "{error}; the old file is kept as {aside}"
                )));
            }
            return Err(error);
        }
        let _ = self.session.remove_file(aside).await;
        Ok(())
    }

    /// The file `path` open for reading (pipelined reads).
    pub(crate) async fn open_read(&self, path: &str) -> Result<RemoteFile, SftpError> {
        self.session
            .open(path)
            .await
            .map_err(|error| self.fail(error))
    }

    /// The file `path` created or emptied for writing (pipelined writes),
    /// with `mode` set on it when given.
    pub(crate) async fn open_write(
        &self,
        path: &str,
        mode: Option<u32>,
    ) -> Result<RemoteFile, SftpError> {
        let flags = OpenFlags::WRITE | OpenFlags::CREATE | OpenFlags::TRUNCATE;
        let file = self
            .session
            .open_with_flags(path, flags)
            .await
            .map_err(|error| self.fail(error))?;
        Self::set_mode(&file, mode).await;
        Ok(file)
    }

    /// A remote file error as this session sees it.
    pub(crate) fn file_error(&self, error: std::io::Error) -> SftpError {
        self.fail_io(error)
    }

    /// Whether the connection under this session is gone (a request would
    /// fail with `ConnectionLost`).
    pub fn is_closed(&self) -> bool {
        self.link.is_closed()
    }

    /// Ends the session (the channel closes; the connection stays).
    pub async fn close(&self) {
        let _ = self.session.close().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_paths_join_with_one_slash() {
        assert_eq!(join("/home/u", "a"), "/home/u/a");
        assert_eq!(join("/", "a"), "/a");
        assert_eq!(join("", "a"), "a");
        assert_eq!(join("dir/", "a b"), "dir/a b");
        assert_eq!(trimmed("/home/u/link/"), "/home/u/link");
        assert_eq!(trimmed("link//"), "link");
        assert_eq!(trimmed("/"), "/");
        assert_eq!(trimmed("//"), "/");
    }

    #[test]
    fn a_download_goes_into_a_part_file() {
        let path = Path::new("dir").join("файл.txt");
        assert_eq!(local_part(&path), Path::new("dir").join("файл.txt.part"));
    }

    #[test]
    fn server_statuses_become_clear_errors() {
        let status = |code| {
            LibError::Status(russh_sftp::protocol::Status {
                id: 1,
                status_code: code,
                error_message: "disk full".to_owned(),
                language_tag: "en".to_owned(),
            })
        };
        assert_eq!(
            SftpError::from(status(StatusCode::NoSuchFile)),
            SftpError::NotFound
        );
        assert_eq!(
            SftpError::from(status(StatusCode::PermissionDenied)),
            SftpError::PermissionDenied
        );
        assert_eq!(
            SftpError::from(status(StatusCode::Failure)),
            SftpError::Failure("disk full".to_owned())
        );
        assert_eq!(
            SftpError::from(status(StatusCode::ConnectionLost)),
            SftpError::ConnectionLost
        );
        assert_eq!(SftpError::from(LibError::Timeout), SftpError::Timeout);
        assert_eq!(
            SftpError::from(LibError::UnexpectedBehavior("sender dropped".to_owned())),
            SftpError::ConnectionLost
        );
    }
}
