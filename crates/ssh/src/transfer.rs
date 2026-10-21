//! Folder transfers over [`crate::Sftp`] (night plan, stage 4, items 3 and
//! 4; night decision D47).
//!
//! - [`download_tree`] and [`upload_tree`] copy a folder with everything in
//!   it: the folders are created first, then each file goes through the
//!   engine (its `.part`, its modification time).
//! - A symbolic link inside the tree (D50, amending D47): a link to a file
//!   is copied as the content of its target; a link to a folder is never
//!   followed, so a link can neither pull in a folder outside the tree nor
//!   loop. What is left out (links to folders, links whose target cannot
//!   be read, devices and sockets, names that are not UTF-8 on the server)
//!   is listed by its path in the tree ([`TreeSummary::skipped`]) for the
//!   end of the transfer.
//! - [`copy_between`] copies one file from one server to another (remote ↔
//!   remote panels): the bytes pass through this machine without a local
//!   file.

use std::path::{Path, PathBuf};

use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

use crate::sftp::{EntryKind, PART_SUFFIX, Sftp, SftpError, join};

/// What a folder transfer did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TreeSummary {
    /// Files copied, the targets of links to files included.
    pub files: usize,
    pub folders: usize,
    pub bytes: u64,
    /// Links to files, copied as the content of their target.
    pub links: usize,
    /// What was left out, in the order it was met.
    pub skipped: Vec<Skipped>,
}

/// An entry a folder transfer left out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Skipped {
    /// Its path inside the copied folder, `/`-separated (`logs/current`).
    pub path: String,
    pub reason: SkipReason,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SkipReason {
    /// A symbolic link to a folder (never followed).
    LinkToFolder,
    /// A symbolic link whose target is missing or cannot be read.
    BrokenLink,
    /// Neither a file, a folder nor a link (a device, a socket, a pipe).
    Special,
    /// A name that is not UTF-8 on the server: the listing shows it with
    /// U+FFFD, and that name does not reach the entry (D45).
    NameNotUtf8,
}

impl TreeSummary {
    fn skip(&mut self, path: String, reason: SkipReason) {
        self.skipped.push(Skipped { path, reason });
    }
}

fn local_io(error: std::io::Error) -> SftpError {
    SftpError::Local(error.kind().to_string())
}

/// `name` under the tree path `parent` (empty at the top).
fn tree_path(parent: &str, name: &str) -> String {
    if parent.is_empty() {
        name.to_owned()
    } else {
        format!("{parent}/{name}")
    }
}

/// Downloads the remote folder `remote` into the local folder `local`
/// (created if needed). `progress` gets the bytes done in all.
pub async fn download_tree(
    sftp: &Sftp,
    remote: &str,
    local: &Path,
    progress: &mut (dyn FnMut(u64) + Send),
) -> Result<TreeSummary, SftpError> {
    let mut summary = TreeSummary::default();
    let mut folders: Vec<(String, PathBuf, String)> =
        vec![(remote.to_owned(), local.to_path_buf(), String::new())];
    let mut files: Vec<(String, PathBuf)> = Vec::new();
    while let Some((dir, target, at)) = folders.pop() {
        tokio::fs::create_dir_all(&target).await.map_err(local_io)?;
        summary.folders += 1;
        for entry in sftp.list(&dir).await? {
            let source = join(&dir, &entry.name);
            let inside = tree_path(&at, &entry.name);
            if entry.name.contains(char::REPLACEMENT_CHARACTER)
                && matches!(sftp.stat(&source).await, Err(SftpError::NotFound))
            {
                summary.skip(inside, SkipReason::NameNotUtf8);
                continue;
            }
            // A name Windows cannot hold gets the suggested one (D46).
            let name = if cfg!(windows) {
                crate::local_names::windows_safe_name(&entry.name)
                    .unwrap_or_else(|| entry.name.clone())
            } else {
                entry.name.clone()
            };
            let path = target.join(name);
            match entry.kind {
                EntryKind::Dir => folders.push((source, path, inside)),
                EntryKind::File => files.push((source, path)),
                EntryKind::Symlink { to_dir: true, .. } => {
                    summary.skip(inside, SkipReason::LinkToFolder);
                }
                // The download follows the link to its target's content.
                EntryKind::Symlink { .. } => match sftp.stat_target(&source).await {
                    Ok(target) if target.kind == EntryKind::File => {
                        summary.links += 1;
                        files.push((source, path));
                    }
                    Ok(target) if target.kind == EntryKind::Dir => {
                        summary.skip(inside, SkipReason::LinkToFolder);
                    }
                    Ok(_) => summary.skip(inside, SkipReason::Special),
                    // The link is fine; the session is not.
                    Err(error @ (SftpError::ConnectionLost | SftpError::Timeout)) => {
                        return Err(error);
                    }
                    Err(_) => summary.skip(inside, SkipReason::BrokenLink),
                },
                EntryKind::Other => summary.skip(inside, SkipReason::Special),
            }
        }
    }
    let mut base = 0u64;
    for (source, path) in files {
        let path = crate::local_names::long_path(&path);
        let size = sftp
            .download(&source, &path, false, &mut |done| progress(base + done))
            .await?;
        base += size;
        summary.files += 1;
    }
    summary.bytes = base;
    Ok(summary)
}

/// Uploads the local folder `local` into the remote folder `remote`
/// (created if needed). `progress` gets the bytes done in all.
pub async fn upload_tree(
    sftp: &Sftp,
    local: &Path,
    remote: &str,
    progress: &mut (dyn FnMut(u64) + Send),
) -> Result<TreeSummary, SftpError> {
    let mut summary = TreeSummary::default();
    let mut folders: Vec<(PathBuf, String, String)> =
        vec![(local.to_path_buf(), remote.to_owned(), String::new())];
    let mut files: Vec<(PathBuf, String)> = Vec::new();
    while let Some((dir, target, at)) = folders.pop() {
        match sftp.stat(&target).await {
            Ok(entry) if entry.kind == EntryKind::Dir => {}
            Ok(_) => return Err(SftpError::NotAFile),
            Err(SftpError::NotFound) => sftp.create_dir(&target).await?,
            Err(error) => return Err(error),
        }
        summary.folders += 1;
        let mut entries = tokio::fs::read_dir(&dir).await.map_err(local_io)?;
        while let Some(entry) = entries.next_entry().await.map_err(local_io)? {
            // The entry itself first: a link is decided by what it points to.
            let kind = entry.file_type().await.map_err(local_io)?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let inside = tree_path(&at, &name);
            let path = entry.path();
            let destination = join(&target, &name);
            if kind.is_symlink() {
                // The upload opens the link, so it sends the target's
                // content.
                match tokio::fs::metadata(&path).await {
                    Ok(target) if target.is_file() => {
                        summary.links += 1;
                        files.push((path, destination));
                    }
                    Ok(target) if target.is_dir() => {
                        summary.skip(inside, SkipReason::LinkToFolder);
                    }
                    Ok(_) => summary.skip(inside, SkipReason::Special),
                    Err(_) => summary.skip(inside, SkipReason::BrokenLink),
                }
            } else if kind.is_dir() {
                folders.push((path, destination, inside));
            } else if kind.is_file() {
                files.push((path, destination));
            } else {
                summary.skip(inside, SkipReason::Special);
            }
        }
    }
    let mut base = 0u64;
    for (path, destination) in files {
        let size = sftp
            .upload(&path, &destination, false, &mut |done| {
                progress(base + done)
            })
            .await?;
        base += size;
        summary.files += 1;
    }
    summary.bytes = base;
    Ok(summary)
}

/// Copies the file `source` of `from` to `target` on `to` (through
/// `target.part`, renamed when complete). Returns the size.
pub async fn copy_between(
    from: &Sftp,
    source: &str,
    to: &Sftp,
    target: &str,
    progress: &mut (dyn FnMut(u64) + Send),
) -> Result<u64, SftpError> {
    let entry = from.stat(source).await?;
    if entry.kind != EntryKind::File {
        return Err(SftpError::NotAFile);
    }
    let mut reader = from.open_read(source).await?;
    let part = format!("{target}{PART_SUFFIX}");
    // A file that is replaced keeps its permissions.
    let mode = to.mode_of(target).await;
    let mut writer = to.open_write(&part, mode).await?;
    let mut buffer = vec![0u8; 256 * 1024];
    let mut done = 0u64;
    loop {
        let read = reader
            .read(&mut buffer)
            .await
            .map_err(|error| from.file_error(error))?;
        if read == 0 {
            break;
        }
        writer
            .write_all(&buffer[..read])
            .await
            .map_err(|error| to.file_error(error))?;
        done += read as u64;
        progress(done);
    }
    writer
        .shutdown()
        .await
        .map_err(|error| to.file_error(error))?;
    drop(writer);
    let _ = reader.shutdown().await;
    if done != entry.size {
        return Err(SftpError::Changed);
    }
    to.replace(&part, target).await?;
    Ok(done)
}
