//! Temporary profiles: a fresh data directory under the OS temporary directory
//! (`%TEMP%\<prefix>-<random>` on Windows), removed when the value is dropped.
//!
//! Used by the tests ([`crate::testing::TestProfile`]) and by `--demo`
//! (feature `dev-tools`), so neither ever touches the permanent profile of
//! [`crate::dirs`].

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::dirs::{AppDirs, create_private_dir};

/// A temporary data directory that is deleted on drop.
pub struct TempProfile {
    path: PathBuf,
    removed: bool,
}

// The path names the user's temporary directory; keep it out of Debug output.
impl std::fmt::Debug for TempProfile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TempProfile { .. }")
    }
}

/// Lowercase hex from the OS CSPRNG (24 characters).
pub fn random_suffix() -> io::Result<String> {
    let bytes = shellpp_core::crypto::random_array::<12>()
        .map_err(|_| io::Error::other("the OS random generator failed"))?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

impl TempProfile {
    /// Creates `<temp>/<prefix>-<random>` (Unix: mode 0700).
    pub fn create(prefix: &str) -> io::Result<Self> {
        Self::create_in(&std::env::temp_dir(), prefix)
    }

    /// Creates `<parent>/<prefix>-<random>`.
    pub fn create_in(parent: &Path, prefix: &str) -> io::Result<Self> {
        let path = parent.join(format!("{prefix}-{}", random_suffix()?));
        if path.exists() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "temporary profile exists",
            ));
        }
        create_private_dir(&path)?;
        let path = std::path::absolute(&path).unwrap_or(path);
        Ok(Self {
            path,
            removed: false,
        })
    }

    /// The profile directory (the value for `SHELLPP_DATA_DIR`).
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The directory layout of this profile (`<dir>`, `<dir>/logs`, `<dir>/tmp`).
    pub fn dirs(&self) -> AppDirs {
        AppDirs::under(self.path.clone())
    }

    /// Deletes the directory now. Everything that keeps files open inside it
    /// (the core's database, the log file) must be closed first. Retries for a
    /// short while: Windows releases file handles asynchronously.
    pub fn remove(&mut self) -> io::Result<()> {
        if self.removed {
            return Ok(());
        }
        let mut last = Ok(());
        for attempt in 0..20 {
            if attempt > 0 {
                std::thread::sleep(Duration::from_millis(50));
            }
            last = match fs::remove_dir_all(&self.path) {
                Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
                other => other,
            };
            if last.is_ok() {
                self.removed = true;
                break;
            }
        }
        last
    }
}

impl Drop for TempProfile {
    fn drop(&mut self) {
        if self.remove().is_err() {
            tracing::warn!("temporary profile could not be removed");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_a_fresh_directory_and_removes_it_on_drop() {
        let first = TempProfile::create("shellpp-test-profile").unwrap();
        let second = TempProfile::create("shellpp-test-profile").unwrap();
        assert_ne!(first.path(), second.path());
        assert!(first.path().is_dir());
        assert!(first.path().starts_with(std::env::temp_dir()) || first.path().is_absolute());
        let name = first
            .path()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        assert!(name.starts_with("shellpp-test-profile-"));
        assert_eq!(name.len(), "shellpp-test-profile-".len() + 24);

        let dirs = first.dirs();
        dirs.create().unwrap();
        fs::write(dirs.logs.join("x.log"), b"x").unwrap();
        let path = first.path().to_path_buf();
        drop(first);
        assert!(!path.exists());
        assert!(second.path().is_dir());
    }

    #[test]
    fn explicit_remove_is_idempotent() {
        let mut profile = TempProfile::create("shellpp-test-profile").unwrap();
        profile.remove().unwrap();
        assert!(!profile.path().exists());
        profile.remove().unwrap();
    }
}
