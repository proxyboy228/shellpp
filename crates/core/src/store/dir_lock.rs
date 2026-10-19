//! One app process per data directory (docs/storage-format.md, section 2.1).
//!
//! While a store is open, it holds an exclusive OS file lock on
//! `<data_dir>/shellpp.lock`. A second process that opens the same data
//! directory (a second launch where the single-instance check is not
//! available, for example on Linux without a D-Bus session bus) fails with
//! [`StoreError::InUse`] instead of working on its own stale copy of the
//! index and overwriting the first process's changes.
//!
//! Stores of the same process share one lock: the host opens the store once;
//! tests open a data directory again while an earlier store is still alive.
//! The lock is released when the last store of the directory is dropped, and
//! by the OS when the process ends.

use std::fs::{self, File, OpenOptions, TryLockError};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::time::{Duration, Instant};

use super::StoreError;

/// The lock file in the data directory (empty; only its OS lock matters).
pub(crate) const LOCK_FILE_NAME: &str = "shellpp.lock";

/// How long [`DirLock::acquire`] waits for another process to let go: a
/// restart starts the new process while the old one is still exiting.
pub(crate) const LOCK_WAIT: Duration = Duration::from_secs(3);

const RETRY_EVERY: Duration = Duration::from_millis(50);

/// Locks held by this process, by canonical data directory.
static HELD: Mutex<Vec<(PathBuf, Weak<File>)>> = Mutex::new(Vec::new());

/// The exclusive lock of a data directory, held while it lives.
#[derive(Clone, Debug)]
pub(crate) struct DirLock {
    _file: Arc<File>,
}

impl DirLock {
    /// Takes the lock of `data_dir` (which must exist), waiting up to `wait`
    /// for another process to release it.
    pub(crate) fn acquire(data_dir: &Path, wait: Duration) -> Result<Self, StoreError> {
        let key = fs::canonicalize(data_dir)?;
        let mut held = HELD.lock().unwrap_or_else(PoisonError::into_inner);
        held.retain(|(_, file)| file.strong_count() > 0);
        if let Some(file) = held
            .iter()
            .filter(|(path, _)| *path == key)
            .find_map(|(_, file)| file.upgrade())
        {
            return Ok(Self { _file: file });
        }

        let file = open_lock_file(&key.join(LOCK_FILE_NAME))?;
        let deadline = Instant::now() + wait;
        loop {
            match file.try_lock() {
                Ok(()) => break,
                Err(TryLockError::WouldBlock) if Instant::now() < deadline => {
                    std::thread::sleep(RETRY_EVERY);
                }
                Err(TryLockError::WouldBlock) => return Err(StoreError::InUse),
                Err(TryLockError::Error(error)) => return Err(error.into()),
            }
        }
        let file = Arc::new(file);
        held.push((key, Arc::downgrade(&file)));
        Ok(Self { _file: file })
    }
}

/// Opens or creates the lock file (unix 0600) without truncating it.
fn open_lock_file(path: &Path) -> Result<File, StoreError> {
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    Ok(options.open(path)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Another process's lock, simulated by a second handle that is not
    /// shared through `HELD` (OS file locks conflict between handles).
    fn foreign_lock(dir: &Path) -> File {
        let file = open_lock_file(&dir.join(LOCK_FILE_NAME)).unwrap();
        file.try_lock().expect("foreign lock");
        file
    }

    #[test]
    fn a_second_process_is_refused_while_the_lock_is_held() {
        let tmp = tempfile::tempdir().unwrap();
        let lock = DirLock::acquire(tmp.path(), Duration::ZERO).unwrap();
        // Same process: shared.
        let again = DirLock::acquire(tmp.path(), Duration::ZERO).unwrap();
        // Another handle (another process) cannot take it.
        let other = open_lock_file(&tmp.path().join(LOCK_FILE_NAME)).unwrap();
        assert!(matches!(other.try_lock(), Err(TryLockError::WouldBlock)));
        drop(lock);
        assert!(matches!(other.try_lock(), Err(TryLockError::WouldBlock)));
        drop(again);
        other.try_lock().expect("released with the last store");
    }

    /// "Start with a new vault" closes the database while it keeps a handle:
    /// no other process can take the directory meanwhile, and the new store
    /// shares the lock.
    #[test]
    fn a_handle_keeps_the_directory_locked_after_the_store_is_dropped() {
        let tmp = tempfile::tempdir().unwrap();
        let (store, _) = crate::store::Store::open(tmp.path(), "0.1.0").unwrap();
        let handle = store.dir_lock().expect("a file store holds the lock");
        drop(store);
        let other = open_lock_file(&tmp.path().join(LOCK_FILE_NAME)).unwrap();
        assert!(matches!(other.try_lock(), Err(TryLockError::WouldBlock)));
        let (again, _) = crate::store::Store::open(tmp.path(), "0.1.0").expect("shared");
        drop(handle);
        assert!(matches!(other.try_lock(), Err(TryLockError::WouldBlock)));
        drop(again);
        other.try_lock().expect("released with the last handle");
        assert!(
            crate::store::Store::open_in_memory()
                .unwrap()
                .dir_lock()
                .is_none()
        );
    }

    #[test]
    fn waits_for_a_process_that_is_exiting_then_gives_up() {
        let tmp = tempfile::tempdir().unwrap();
        let foreign = foreign_lock(tmp.path());
        let started = Instant::now();
        assert!(matches!(
            DirLock::acquire(tmp.path(), Duration::from_millis(200)),
            Err(StoreError::InUse)
        ));
        assert!(started.elapsed() >= Duration::from_millis(200));

        let releaser = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(150));
            drop(foreign);
        });
        DirLock::acquire(tmp.path(), Duration::from_secs(5)).expect("taken after release");
        releaser.join().unwrap();
    }
}
