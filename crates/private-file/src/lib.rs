//! Files only the current user can read (night plan, stage 3 item 11;
//! D38, D42): the private keys of the SSH files export and the plain Export
//! Data file, which Shell++ writes into a folder the user chose, outside its
//! own data folder.
//!
//! [`write`] creates or replaces a file:
//! - a symbolic link at the path is refused and nothing is written;
//! - the content always goes into a file this call creates: the file itself
//!   ([`Existing::Refuse`]), or a temporary file in the same folder that is
//!   then renamed over the old one ([`Existing::Replace`]), so a handle
//!   someone opened on the old file never sees the new content, and a file
//!   another user owns is replaced instead of reused;
//! - Unix: the new file is created with mode 0600 and `O_NOFOLLOW`;
//! - Windows: the new file is opened without sharing (no other handle can
//!   open it meanwhile) and gets a protected DACL with one entry, full
//!   control for the current user, and no inherited entries, before any
//!   byte is written, so a folder that every local user can read
//!   (`C:\export`) does not expose it.
//!
//! [`is_private`] reads it back (tests, and a check after writing).

use std::fs::OpenOptions;
use std::io::{self, Write as _};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

#[cfg(unix)]
mod unix;
#[cfg(unix)]
use unix as sys;
#[cfg(windows)]
mod windows;
#[cfg(windows)]
use windows as sys;

/// What [`write`] does when the file exists.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Existing {
    /// Replace its content (and its permissions).
    Replace,
    /// Fail with [`io::ErrorKind::AlreadyExists`].
    Refuse,
}

/// Writes `bytes` to `path` so that only the current user can read it.
pub fn write(path: &Path, bytes: &[u8], existing: Existing) -> io::Result<()> {
    if std::fs::symlink_metadata(path).is_ok_and(|meta| meta.file_type().is_symlink()) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "the path is a symbolic link",
        ));
    }
    match existing {
        Existing::Refuse => write_new(path, bytes),
        Existing::Replace => {
            let temporary = create_temporary(path, bytes)?;
            std::fs::rename(&temporary, path).inspect_err(|_| {
                let _ = std::fs::remove_file(&temporary);
            })
        }
    }
}

/// Creates `path` (it must not exist) for the user only, then writes it.
fn write_new(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    sys::prepare(&mut options);
    let mut file = options.open(path)?;
    // The permissions first, then the content.
    let written = sys::restrict(&file)
        .and_then(|()| file.write_all(bytes))
        .and_then(|()| file.sync_all());
    if written.is_err() {
        drop(file);
        let _ = std::fs::remove_file(path);
    }
    written
}

/// A new private file next to `path` that holds `bytes`
/// (`.<name>.<process>-<n>.tmp`).
fn create_temporary(path: &Path, bytes: &[u8]) -> io::Result<std::path::PathBuf> {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let dir = match path.parent() {
        Some(dir) if !dir.as_os_str().is_empty() => dir,
        _ => Path::new("."),
    };
    let name = path
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "no file name"))?
        .to_string_lossy()
        .into_owned();
    let mut last = None;
    for _ in 0..16 {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.subsec_nanos());
        let temporary = dir.join(format!(".{name}.{}-{nanos}-{n}.tmp", std::process::id()));
        match write_new(&temporary, bytes) {
            Ok(()) => return Ok(temporary),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => last = Some(error),
            Err(error) => return Err(error),
        }
    }
    Err(last.unwrap_or_else(|| io::Error::other("no temporary file")))
}

/// Whether only the current user can read `path`: Unix no group or other
/// bits; Windows a protected DACL whose one entry allows the current user.
pub fn is_private(path: &Path) -> io::Result<bool> {
    sys::is_private(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_file_is_private_and_holds_the_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("id_test");
        write(&path, b"secret", Existing::Refuse).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"secret");
        assert!(is_private(&path).unwrap());
    }

    #[test]
    fn an_existing_file_becomes_private_before_it_is_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("export.json");
        std::fs::write(&path, b"an older and longer content").unwrap();
        assert!(!is_private(&path).unwrap(), "a plain file is not private");
        assert_eq!(
            write(&path, b"new", Existing::Refuse).unwrap_err().kind(),
            io::ErrorKind::AlreadyExists
        );
        write(&path, b"new", Existing::Replace).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"new");
        assert!(is_private(&path).unwrap());
        // Nothing but the file is left in the folder.
        let names: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(names, ["export.json"]);
    }

    /// A handle opened on the old file before the replace never reads the
    /// new content: the content goes into another file.
    #[test]
    fn a_handle_on_the_old_file_does_not_see_the_new_content() {
        use std::io::Read as _;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("id_key");
        std::fs::write(&path, b"old").unwrap();
        #[cfg(windows)]
        let mut old = {
            use std::os::windows::fs::OpenOptionsExt as _;
            // FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE, so the
            // rename can replace the file while it is open.
            OpenOptions::new()
                .read(true)
                .share_mode(7)
                .open(&path)
                .unwrap()
        };
        #[cfg(not(windows))]
        let mut old = std::fs::File::open(&path).unwrap();
        write(&path, b"secret", Existing::Replace).unwrap();
        let mut seen = Vec::new();
        old.read_to_end(&mut seen).unwrap();
        assert_eq!(seen, b"old");
        assert_eq!(std::fs::read(&path).unwrap(), b"secret");
    }

    #[cfg(unix)]
    #[test]
    fn a_symbolic_link_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("target");
        std::fs::write(&target, b"keep").unwrap();
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        assert!(write(&link, b"new", Existing::Replace).is_err());
        assert_eq!(std::fs::read(&target).unwrap(), b"keep");
    }
}
