//! Folder transfers over [`crate::Sftp`] (night plan, stage 4, items 3 and
//! 4; night decision D47).
//!
//! - [`download_tree`] and [`upload_tree`] copy a folder with everything in
//!   it: the folders are created first, then each file goes through the
//!   engine (its `.part`, its modification time).
//! - A symbolic link inside the tree is never followed, neither to a folder
//!   nor to a file: it is left out and counted ([`TreeSummary::links`]), so
//!   a link can neither pull in a folder outside the tree nor loop.
//! - [`copy_between`] copies one file from one server to another (remote ↔
//!   remote panels): the bytes pass through this machine without a local
//!   file.

use std::path::{Path, PathBuf};

use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

use crate::sftp::{EntryKind, PART_SUFFIX, Sftp, SftpError, join};

/// What a folder transfer did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TreeSummary {
    pub files: usize,
    pub folders: usize,
    pub bytes: u64,
    /// Symbolic links left out.
    pub links: usize,
    /// Entries that are neither files, folders nor links (devices, sockets).
    pub other: usize,
}

fn local_io(error: std::io::Error) -> SftpError {
    SftpError::Local(error.kind().to_string())
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
    let mut folders: Vec<(String, PathBuf)> = vec![(remote.to_owned(), local.to_path_buf())];
    let mut files: Vec<(String, PathBuf)> = Vec::new();
    while let Some((dir, target)) = folders.pop() {
        tokio::fs::create_dir_all(&target).await.map_err(local_io)?;
        summary.folders += 1;
        for entry in sftp.list(&dir).await? {
            let source = join(&dir, &entry.name);
            // A name Windows cannot hold gets the suggested one (D46).
            let name = if cfg!(windows) {
                crate::local_names::windows_safe_name(&entry.name)
                    .unwrap_or_else(|| entry.name.clone())
            } else {
                entry.name.clone()
            };
            let path = target.join(name);
            match entry.kind {
                EntryKind::Dir => folders.push((source, path)),
                EntryKind::File => files.push((source, path)),
                EntryKind::Symlink { .. } => summary.links += 1,
                EntryKind::Other => summary.other += 1,
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
    let mut folders: Vec<(PathBuf, String)> = vec![(local.to_path_buf(), remote.to_owned())];
    let mut files: Vec<(PathBuf, String)> = Vec::new();
    while let Some((dir, target)) = folders.pop() {
        match sftp.stat(&target).await {
            Ok(entry) if entry.kind == EntryKind::Dir => {}
            Ok(_) => return Err(SftpError::NotAFile),
            Err(SftpError::NotFound) => sftp.create_dir(&target).await?,
            Err(error) => return Err(error),
        }
        summary.folders += 1;
        let mut entries = tokio::fs::read_dir(&dir).await.map_err(local_io)?;
        while let Some(entry) = entries.next_entry().await.map_err(local_io)? {
            // The entry itself, never what a link points to.
            let kind = entry.file_type().await.map_err(local_io)?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let path = entry.path();
            let destination = join(&target, &name);
            if kind.is_symlink() {
                summary.links += 1;
            } else if kind.is_dir() {
                folders.push((path, destination));
            } else if kind.is_file() {
                files.push((path, destination));
            } else {
                summary.other += 1;
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
    let mut writer = to.open_write(&part).await?;
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
