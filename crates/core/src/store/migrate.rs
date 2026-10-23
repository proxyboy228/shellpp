//! Schema creation and migrations (docs/storage-format.md, section 11).
//!
//! - `PRAGMA user_version` is the schema version; `0` with no schema objects
//!   is a new database.
//! - Migrations are an ordered list of SQL scripts, one per step N -> N + 1,
//!   additive only (new tables, columns with defaults, indexes). They never
//!   need keys and never touch envelope or wrap bytes.
//! - Before the first step the database is copied with `VACUUM INTO` to a
//!   temporary name, synced and renamed to
//!   `backups/shellpp-<UTC yyyymmddThhmmssZ>.db` (section 10: new file, then
//!   rename), so a crash never leaves a truncated copy under a backup name;
//!   only the 3 newest copies are kept.
//! - Each step runs in its own transaction that also sets the new
//!   `user_version`; a failed step is rolled back.
//! - A version above the newest known one is refused without modifying the file.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use rusqlite::{Connection, TransactionBehavior};
use time::OffsetDateTime;
use time::macros::format_description;

use super::StoreError;

/// One migration step from `from` to `from + 1`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Migration {
    pub from: u32,
    pub sql: &'static str,
}

/// How to build and upgrade the schema: the version 1 script and the steps
/// after it, in order (`steps[i].from == 1 + i`).
#[derive(Clone, Copy, Debug)]
pub(crate) struct MigrationPlan {
    pub v1_sql: &'static str,
    pub steps: &'static [Migration],
}

impl MigrationPlan {
    /// The newest schema version this plan produces.
    pub fn target(&self) -> u32 {
        1 + u32::try_from(self.steps.len()).expect("fewer than 2^32 migrations")
    }

    fn check(&self) {
        for (i, step) in self.steps.iter().enumerate() {
            debug_assert_eq!(
                step.from as usize,
                1 + i,
                "migration steps must be consecutive from version 1"
            );
        }
    }
}

/// The migration steps shipped with this app version. Schema 1 is created
/// directly from `0001_initial.sql`; step 1 -> 2 adds the sync tables of
/// stage 7 (`0002_sync.sql`, D93).
pub(crate) const MIGRATIONS: &[Migration] = &[Migration {
    from: 1,
    sql: include_str!("migrations/0002_sync.sql"),
}];

/// Number of pre-migration copies kept in `backups/`.
pub(crate) const BACKUPS_KEPT: usize = 3;

/// Reads `PRAGMA user_version`.
pub(crate) fn user_version(conn: &Connection) -> Result<u32, StoreError> {
    let v: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    // SQLite stores a signed 32-bit value; a negative one was not written by us.
    u32::try_from(v).map_err(|_| StoreError::Corrupt("negative schema version"))
}

/// Number of schema objects (tables, indexes, views, triggers) in the database.
pub(crate) fn schema_object_count(conn: &Connection) -> Result<i64, StoreError> {
    Ok(conn.query_row(
        "SELECT count(*) FROM sqlite_master WHERE name NOT LIKE 'sqlite_%'",
        [],
        |r| r.get(0),
    )?)
}

/// Creates the newest schema in a new, empty database: version 1 and every
/// later step, each in a transaction that sets `user_version`. No backup.
pub(crate) fn create(conn: &mut Connection, plan: &MigrationPlan) -> Result<(), StoreError> {
    plan.check();
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    if user_version(&tx)? != 0 || schema_object_count(&tx)? != 0 {
        return Err(StoreError::Corrupt("database is not empty"));
    }
    tx.execute_batch(plan.v1_sql)
        .map_err(|_| StoreError::Migration { to: 1 })?;
    tx.pragma_update(None, "user_version", 1)?;
    tx.commit()?;
    run_steps(conn, plan, 1)
}

/// Runs the steps after `from` (each in its own transaction).
pub(crate) fn run_steps(
    conn: &mut Connection,
    plan: &MigrationPlan,
    from: u32,
) -> Result<(), StoreError> {
    plan.check();
    for step in plan.steps.iter().filter(|s| s.from >= from) {
        let to = step.from + 1;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if user_version(&tx)? != step.from {
            return Err(StoreError::Migration { to });
        }
        tx.execute_batch(step.sql)
            .map_err(|_| StoreError::Migration { to })?;
        tx.pragma_update(None, "user_version", to)
            .map_err(|_| StoreError::Migration { to })?;
        tx.commit().map_err(|_| StoreError::Migration { to })?;
    }
    Ok(())
}

/// `shellpp-<UTC yyyymmddThhmmssZ>.db` for `when`.
pub(crate) fn backup_file_name(when: SystemTime) -> String {
    let format = format_description!("[year][month][day]T[hour][minute][second]Z");
    let stamp = OffsetDateTime::from(when)
        .format(&format)
        .expect("a UTC timestamp always formats");
    format!("shellpp-{stamp}.db")
}

/// True for names of the form `shellpp-<8 digits>T<6 digits>Z.db`.
pub(crate) fn is_backup_file_name(name: &str) -> bool {
    let Some(stamp) = name
        .strip_prefix("shellpp-")
        .and_then(|rest| rest.strip_suffix("Z.db"))
    else {
        return false;
    };
    let bytes = stamp.as_bytes();
    bytes.len() == 15
        && bytes[8] == b'T'
        && bytes[..8].iter().all(u8::is_ascii_digit)
        && bytes[9..].iter().all(u8::is_ascii_digit)
}

/// Suffix of a backup that is still being written.
const TEMP_SUFFIX: &str = ".tmp";

/// Deletes `shellpp-*.db.tmp` files left by a crash during a backup.
fn remove_partial_backups(dir: &Path) -> Result<(), StoreError> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let is_partial = entry.file_name().to_str().is_some_and(|name| {
            name.strip_suffix(TEMP_SUFFIX)
                .is_some_and(|name| name.starts_with("shellpp-") && name.ends_with(".db"))
        });
        if is_partial && entry.file_type()?.is_file() {
            fs::remove_file(entry.path())?;
        }
    }
    Ok(())
}

/// Writes a consistent copy of the open database into `backups/` with
/// `VACUUM INTO` and deletes all but the newest [`BACKUPS_KEPT`] copies.
/// The copy is written under a temporary name, synced, and renamed, so only
/// complete copies carry a backup name. Returns the path of the new copy.
pub(crate) fn backup(
    conn: &Connection,
    data_dir: &Path,
    now: SystemTime,
) -> Result<PathBuf, StoreError> {
    let dir = data_dir.join(crate::ids::BACKUPS_DIR_NAME);
    fs::create_dir_all(&dir)?;
    super::restrict_dir(&dir)?;
    remove_partial_backups(&dir)?;

    // Two migrations within one second would collide: move to the next free second.
    let mut when = now;
    let mut path = dir.join(backup_file_name(when));
    let mut tries = 0;
    while path.exists() {
        tries += 1;
        if tries > 3600 {
            return Err(StoreError::Io(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                "no free backup file name",
            )));
        }
        when += Duration::from_secs(1);
        path = dir.join(backup_file_name(when));
    }

    let mut temp_name = path.clone().into_os_string();
    temp_name.push(TEMP_SUFFIX);
    let temp = PathBuf::from(temp_name);
    let target = temp.to_str().ok_or_else(|| {
        StoreError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "backup path is not valid UTF-8",
        ))
    })?;
    conn.execute("VACUUM INTO ?1", [target])?;
    super::restrict_file(&temp)?;
    // Windows flushes only through a handle with write access.
    fs::OpenOptions::new().write(true).open(&temp)?.sync_all()?;
    fs::rename(&temp, &path)?;
    sync_dir(&dir)?;
    rotate_backups(&dir)?;
    Ok(path)
}

/// Makes the rename durable (unix: fsync of the directory). Windows has no
/// directory handle to sync; NTFS journals the rename.
fn sync_dir(dir: &Path) -> Result<(), StoreError> {
    #[cfg(unix)]
    fs::File::open(dir)?.sync_all()?;
    #[cfg(not(unix))]
    let _ = dir;
    Ok(())
}

/// Deletes all but the newest [`BACKUPS_KEPT`] backup files (by name, which
/// sorts by time). Other files in the directory are left alone.
pub(crate) fn rotate_backups(dir: &Path) -> Result<(), StoreError> {
    let mut names: Vec<String> = fs::read_dir(dir)?
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_type().map(|t| t.is_file()).unwrap_or(false))
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| is_backup_file_name(name))
        .collect();
    names.sort_unstable_by(|a, b| b.cmp(a));
    for name in names.iter().skip(BACKUPS_KEPT) {
        fs::remove_file(dir.join(name))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backup_names() {
        let t = SystemTime::UNIX_EPOCH + Duration::from_secs(1_792_362_665); // 2026-10-18T22:31:05Z
        assert_eq!(backup_file_name(t), "shellpp-20261018T223105Z.db");
        assert!(is_backup_file_name("shellpp-20261018T223105Z.db"));
        assert!(!is_backup_file_name("shellpp-20261018T223105.db"));
        assert!(!is_backup_file_name("shellpp-2026101xT223105Z.db"));
        assert!(!is_backup_file_name("shellpp.db"));
        assert!(!is_backup_file_name("notes.txt"));
    }

    #[test]
    fn partial_backups_are_removed_and_never_counted() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let partial = "shellpp-20261018T223105Z.db.tmp";
        assert!(!is_backup_file_name(partial));
        fs::write(dir.join(partial), b"truncated").unwrap();
        fs::write(dir.join("notes.tmp"), b"other").unwrap();
        remove_partial_backups(dir).unwrap();
        assert!(!dir.join(partial).exists());
        assert!(dir.join("notes.tmp").exists(), "other files are left alone");
    }

    #[test]
    fn target_version() {
        let plan = MigrationPlan {
            v1_sql: "",
            steps: &[],
        };
        assert_eq!(plan.target(), 1);
        static STEPS: [Migration; 2] = [
            Migration { from: 1, sql: "" },
            Migration { from: 2, sql: "" },
        ];
        let plan = MigrationPlan {
            v1_sql: "",
            steps: &STEPS,
        };
        assert_eq!(plan.target(), 3);
    }
}
