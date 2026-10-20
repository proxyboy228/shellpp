//! SFTP over an open connection (night plan, stage 4, item 1; UI 5; night
//! decision D45).
//!
//! [`crate::Connection::open_sftp`] starts the `sftp` subsystem on a new
//! channel of a connection that a terminal tab may already use, so a second
//! login is never needed. Up to 64 reads and 64 writes are in flight per
//! file (russh-sftp's pipelining, set as OpenSSH's `sftp` does): without
//! that SFTP is slow on a high ping.
//!
//! - [`Sftp::list`]: the entries of a folder, `.` and `..` left out; a
//!   symbolic link is reported as a link with its target and whether that
//!   is a folder, never as what it points to.
//! - [`Sftp::download`] and [`Sftp::upload`]: into `<name>.part` first,
//!   renamed when complete; with `resume`, a `.part` left by an interrupted
//!   transfer is continued from its length. The modification time is kept;
//!   an upload's permissions follow the server's umask.
//! - [`Sftp::remove_tree`]: a recursive delete that removes a symbolic link
//!   itself and never what it points to.
//! - A name that is not UTF-8 on the server is shown with U+FFFD (russh-sftp
//!   decodes names lossily); such a file is listed but cannot be opened.
//!
//! Nothing here logs paths or file content.

use std::io::SeekFrom;
use std::path::{Path, PathBuf};
use std::time::{Duration, UNIX_EPOCH};

use russh_sftp::client::SftpSession;
use russh_sftp::client::error::Error as LibError;
use russh_sftp::protocol::{FileAttributes, OpenFlags, StatusCode};
use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncSeekExt as _, AsyncWrite, AsyncWriteExt as _};

/// The buffer of one copy step (several SFTP requests: russh-sftp splits it
/// and keeps them in flight).
const COPY_BUFFER: usize = 256 * 1024;
/// Requests in flight per file, as OpenSSH's `sftp` (`-R 64`): with 32 KiB
/// each, 2 MiB on the way, enough for a fast link with a 50 ms ping
/// (russh-sftp's default of 16 halves an upload there).
const REQUESTS_IN_FLIGHT: usize = 64;

fn session_config() -> russh_sftp::client::Config {
    russh_sftp::client::Config {
        max_concurrent_reads: REQUESTS_IN_FLIGHT,
        max_concurrent_writes: REQUESTS_IN_FLIGHT,
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

impl From<LibError> for SftpError {
    fn from(error: LibError) -> Self {
        match error {
            LibError::Status(status) => match status.status_code {
                StatusCode::NoSuchFile => SftpError::NotFound,
                StatusCode::PermissionDenied => SftpError::PermissionDenied,
                StatusCode::NoConnection | StatusCode::ConnectionLost => SftpError::ConnectionLost,
                _ => SftpError::Failure(status.error_message),
            },
            LibError::Timeout => SftpError::Timeout,
            LibError::IO(_) => SftpError::ConnectionLost,
            other => SftpError::Failure(other.to_string()),
        }
    }
}

/// An I/O error of a remote file (russh-sftp reports them as `io::Error`).
pub(crate) fn remote_io(error: std::io::Error) -> SftpError {
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
    read_error: fn(std::io::Error) -> SftpError,
    write_error: fn(std::io::Error) -> SftpError,
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

/// An SFTP session on one channel of a [`crate::Connection`].
pub struct Sftp {
    session: SftpSession,
}

impl std::fmt::Debug for Sftp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Sftp").finish_non_exhaustive()
    }
}

impl Sftp {
    pub(crate) async fn start<S>(stream: S, timeout: Duration) -> Result<Self, SftpError>
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        let session = SftpSession::new_with_config(stream, session_config()).await?;
        session.set_timeout(timeout.as_secs().max(1));
        Ok(Self { session })
    }

    /// The absolute form of `path` on the server (`.` is the home folder).
    pub async fn canonicalize(&self, path: &str) -> Result<String, SftpError> {
        Ok(self.session.canonicalize(path).await?)
    }

    /// The entry at `path` (a symbolic link as a link).
    pub async fn stat(&self, path: &str) -> Result<RemoteEntry, SftpError> {
        let attrs = self.session.symlink_metadata(path).await?;
        let name = path.rsplit('/').next().unwrap_or(path).to_owned();
        let mut found = entry(name, &attrs);
        self.describe_link(path, &mut found).await;
        Ok(found)
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

    /// The entries of the folder `path`, `.` and `..` left out, in the
    /// server's order.
    pub async fn list(&self, path: &str) -> Result<Vec<RemoteEntry>, SftpError> {
        let mut out = Vec::new();
        for item in self.session.read_dir(path).await? {
            let name = item.file_name();
            if name == "." || name == ".." {
                continue;
            }
            let mut found = entry(name, &item.metadata());
            self.describe_link(&join(path, &found.name), &mut found)
                .await;
            out.push(found);
        }
        Ok(out)
    }

    pub async fn create_dir(&self, path: &str) -> Result<(), SftpError> {
        Ok(self.session.create_dir(path).await?)
    }

    pub async fn remove_file(&self, path: &str) -> Result<(), SftpError> {
        Ok(self.session.remove_file(path).await?)
    }

    pub async fn remove_dir(&self, path: &str) -> Result<(), SftpError> {
        Ok(self.session.remove_dir(path).await?)
    }

    pub async fn rename(&self, from: &str, to: &str) -> Result<(), SftpError> {
        Ok(self.session.rename(from, to).await?)
    }

    /// Sets the permission bits of `path` (Edit Permissions, UI 5.4).
    pub async fn set_permissions(&self, path: &str, mode: u32) -> Result<(), SftpError> {
        let attrs = FileAttributes {
            permissions: Some(mode & 0o7777),
            ..FileAttributes::empty()
        };
        Ok(self.session.set_metadata(path, attrs).await?)
    }

    /// Removes `path` and, for a folder, everything in it; returns how many
    /// entries were removed. A symbolic link is removed itself: what it
    /// points to is never entered or removed.
    pub async fn remove_tree(&self, path: &str) -> Result<usize, SftpError> {
        let attrs = self.session.symlink_metadata(path).await?;
        if !attrs.file_type().is_dir() {
            self.session.remove_file(path).await?;
            return Ok(1);
        }
        let mut removed = 0;
        // (folder, its entries removed already)
        let mut stack = vec![(path.to_owned(), false)];
        while let Some((dir, emptied)) = stack.pop() {
            if emptied {
                self.session.remove_dir(dir).await?;
                removed += 1;
                continue;
            }
            stack.push((dir.clone(), true));
            for item in self.session.read_dir(dir.as_str()).await? {
                let name = item.file_name();
                if name == "." || name == ".." {
                    continue;
                }
                let child = join(&dir, &name);
                // READDIR reports a link as a link (lstat), never as a folder.
                if item.file_type().is_dir() {
                    stack.push((child, false));
                } else {
                    self.session.remove_file(child).await?;
                    removed += 1;
                }
            }
        }
        Ok(removed)
    }

    /// Downloads `remote` to `local` through `local.part`; with `resume` a
    /// `.part` that is there is continued. Returns the size.
    pub async fn download(
        &self,
        remote: &str,
        local: &Path,
        resume: bool,
        progress: &mut (dyn FnMut(u64) + Send),
    ) -> Result<u64, SftpError> {
        let attrs = self.session.metadata(remote).await?;
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
        let mut source = self.session.open(remote).await?;
        if offset > 0 {
            source
                .seek(SeekFrom::Start(offset))
                .await
                .map_err(remote_io)?;
        }
        let mut options = tokio::fs::OpenOptions::new();
        options.create(true);
        if offset > 0 {
            options.append(true);
        } else {
            options.write(true).truncate(true);
        }
        let mut target = options.open(&part).await.map_err(local_io)?;
        progress(offset);
        let done = copy(
            &mut source,
            &mut target,
            offset,
            progress,
            remote_io,
            local_io,
        )
        .await?;
        target.sync_all().await.map_err(local_io)?;
        drop(target);
        let _ = source.shutdown().await;
        if size.is_some_and(|size| size != done) {
            return Err(SftpError::Changed);
        }
        if let Some(mtime) = attrs.mtime {
            let file = std::fs::OpenOptions::new()
                .write(true)
                .open(&part)
                .map_err(local_io)?;
            file.set_modified(UNIX_EPOCH + Duration::from_secs(u64::from(mtime)))
                .map_err(local_io)?;
        }
        tokio::fs::rename(&part, local).await.map_err(local_io)?;
        Ok(done)
    }

    /// Uploads `local` to `remote` through `remote.part`; with `resume` a
    /// `.part` that is there is continued. An existing `remote` is replaced
    /// once the upload is complete. Returns the size.
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
        let offset = if resume {
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
        let flags = if offset > 0 {
            OpenFlags::WRITE
        } else {
            OpenFlags::WRITE | OpenFlags::CREATE | OpenFlags::TRUNCATE
        };
        let mut target = self.session.open_with_flags(part.as_str(), flags).await?;
        if offset > 0 {
            target
                .seek(SeekFrom::Start(offset))
                .await
                .map_err(remote_io)?;
            source
                .seek(SeekFrom::Start(offset))
                .await
                .map_err(local_io)?;
        }
        progress(offset);
        let done = copy(
            &mut source,
            &mut target,
            offset,
            progress,
            local_io,
            remote_io,
        )
        .await?;
        target.shutdown().await.map_err(remote_io)?;
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
            self.session.set_metadata(part.as_str(), attrs).await?;
        }
        self.replace(&part, remote).await?;
        Ok(done)
    }

    /// Renames the complete `part` to `target`, over an existing `target`
    /// (SFTP v3 does not rename over a file: the old one is removed first).
    pub(crate) async fn replace(&self, part: &str, target: &str) -> Result<(), SftpError> {
        if self.session.rename(part, target).await.is_err() {
            if self.session.symlink_metadata(target).await.is_ok() {
                self.session.remove_file(target).await?;
            }
            self.session.rename(part, target).await?;
        }
        Ok(())
    }

    /// The file `path` open for reading (pipelined reads).
    pub(crate) async fn open_read(
        &self,
        path: &str,
    ) -> Result<russh_sftp::client::fs::File, SftpError> {
        Ok(self.session.open(path).await?)
    }

    /// The file `path` created or emptied for writing (pipelined writes).
    pub(crate) async fn open_write(
        &self,
        path: &str,
    ) -> Result<russh_sftp::client::fs::File, SftpError> {
        let flags = OpenFlags::WRITE | OpenFlags::CREATE | OpenFlags::TRUNCATE;
        Ok(self.session.open_with_flags(path, flags).await?)
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
    }
}
