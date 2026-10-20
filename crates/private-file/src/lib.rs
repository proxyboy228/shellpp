//! Files only the current user can read (night plan, stage 3 item 11;
//! D38, D42): the private keys of the SSH files export and the plain Export
//! Data file, which Shell++ writes into a folder the user chose, outside its
//! own data folder.
//!
//! [`write`] creates or replaces a file:
//! - a symbolic link at the path is refused and nothing is written;
//! - Unix: the file is opened with mode 0600 and `O_NOFOLLOW`, and an
//!   existing file gets mode 0600 before it is emptied and written, so the
//!   content is never readable by others, not even for a moment;
//! - Windows: before any byte is written the file gets a protected DACL with
//!   one entry, full control for the current user, and no inherited entries,
//!   so a folder that every local user can read (`C:\export`) does not
//!   expose it.
//!
//! [`is_private`] reads it back (tests, and a check after writing).

use std::fs::OpenOptions;
use std::io::{self, Write as _};
use std::path::Path;

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
    let mut options = OpenOptions::new();
    options.write(true);
    match existing {
        Existing::Replace => options.create(true),
        Existing::Refuse => options.create_new(true),
    };
    sys::prepare(&mut options);
    let mut file = options.open(path)?;
    // The permissions first, then the content.
    sys::restrict(&file)?;
    file.set_len(0)?;
    file.write_all(bytes)?;
    file.sync_all()
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
