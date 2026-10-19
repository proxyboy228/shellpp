//! "Start with a new vault" (UI 13.2, decision D29): the user forgot the
//! master password and starts over. The database file is renamed, never
//! deleted: `shellpp.db` becomes `shellpp.db.<UTC yyyymmddThhmmssZ>.old`,
//! with its `-wal` and `-shm` files (if any) renamed alongside as
//! `<archive>-wal` and `<archive>-shm`, so SQLite still opens the archive
//! as one database. A second archive in the same second gets `-2`, `-3`,
//! ... before `.old`.
//!
//! The caller closes the database first (Windows cannot rename open files)
//! and opens a new one afterwards, holding the lock of the data directory
//! all along; see [`crate::service::Core::start_new_vault`]. Nothing here
//! deletes a file: when the new database cannot be created, the one that
//! was started is set aside as `shellpp.db.<stamp>-failed.old` and the
//! archive is put back.

use std::fs;
use std::io;
use std::path::Path;
use std::time::SystemTime;

use time::OffsetDateTime;
use time::macros::format_description;

use crate::ids::DB_FILE_NAME;

/// Suffix of an archived database file.
pub const ARCHIVE_SUFFIX: &str = ".old";
/// Marks a new database set aside because it could not be completed.
pub const FAILED_TAG: &str = "-failed";
/// The SQLite side files that belong to a database file.
const SIDE_SUFFIXES: [&str; 2] = ["-wal", "-shm"];

/// `shellpp.db.<UTC yyyymmddThhmmssZ>.old`, with `-<n>` before `.old` for
/// `n` > 1.
pub fn archive_name(when: SystemTime, n: u32) -> String {
    tagged_name(when, "", n)
}

/// `shellpp.db.<stamp><tag>[-<n>].old`.
fn tagged_name(when: SystemTime, tag: &str, n: u32) -> String {
    let format = format_description!("[year][month][day]T[hour][minute][second]Z");
    let stamp = OffsetDateTime::from(when)
        .format(&format)
        .unwrap_or_else(|_| "00000000T000000Z".to_owned());
    if n > 1 {
        format!("{DB_FILE_NAME}.{stamp}{tag}-{n}{ARCHIVE_SUFFIX}")
    } else {
        format!("{DB_FILE_NAME}.{stamp}{tag}{ARCHIVE_SUFFIX}")
    }
}

/// The first name `shellpp.db.<stamp><tag>[-<n>].old` that is free together
/// with its side-file names.
fn free_name(data_dir: &Path, when: SystemTime, tag: &str) -> io::Result<String> {
    (1..=999)
        .map(|n| tagged_name(when, tag, n))
        .find(|name| {
            !data_dir.join(name).exists()
                && SIDE_SUFFIXES
                    .iter()
                    .all(|suffix| !data_dir.join(format!("{name}{suffix}")).exists())
        })
        .ok_or_else(|| io::Error::new(io::ErrorKind::AlreadyExists, "no free archive name"))
}

/// Renames `<from>`, `<from>-wal` and `<from>-shm` (those that exist) to
/// `<to>`, `<to>-wal` and `<to>-shm`. When a rename fails, the renames
/// already done are undone. Targets are free names (see [`free_name`]) or
/// names emptied just before, so no rename replaces a file.
fn rename_set(data_dir: &Path, from: &str, to: &str) -> io::Result<()> {
    let mut done: Vec<&str> = Vec::new();
    for suffix in std::iter::once("").chain(SIDE_SUFFIXES) {
        let source = data_dir.join(format!("{from}{suffix}"));
        if !source.exists() {
            continue;
        }
        let target = data_dir.join(format!("{to}{suffix}"));
        if target.exists() {
            undo(data_dir, from, to, &done);
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "target name taken",
            ));
        }
        if let Err(error) = fs::rename(&source, &target) {
            undo(data_dir, from, to, &done);
            return Err(error);
        }
        done.push(suffix);
    }
    Ok(())
}

fn undo(data_dir: &Path, from: &str, to: &str, done: &[&str]) {
    for suffix in done {
        let _ = fs::rename(
            data_dir.join(format!("{to}{suffix}")),
            data_dir.join(format!("{from}{suffix}")),
        );
    }
}

/// `true` when `shellpp.db` or one of its side files exists.
fn database_present(data_dir: &Path) -> bool {
    std::iter::once("")
        .chain(SIDE_SUFFIXES)
        .any(|suffix| data_dir.join(format!("{DB_FILE_NAME}{suffix}")).exists())
}

/// Renames the closed database of `data_dir` (and its side files) to a
/// free archive name; returns that name (a file name, never a path).
/// Nothing is renamed when it fails: renames already done are undone.
pub fn archive_data_file(data_dir: &Path, when: SystemTime) -> io::Result<String> {
    if !data_dir.join(DB_FILE_NAME).is_file() {
        return Err(io::Error::new(io::ErrorKind::NotFound, "no database file"));
    }
    let name = free_name(data_dir, when, "")?;
    rename_set(data_dir, DB_FILE_NAME, &name)?;
    Ok(name)
}

/// Undoes [`archive_data_file`]: used when the new database cannot be
/// created, so the app keeps the old one. A database started meanwhile
/// (with its side files) is not deleted but set aside under a free
/// `shellpp.db.<stamp>-failed[-<n>].old` name first.
pub fn restore_archived(data_dir: &Path, name: &str, when: SystemTime) -> io::Result<()> {
    if database_present(data_dir) {
        let aside = free_name(data_dir, when, FAILED_TAG)?;
        rename_set(data_dir, DB_FILE_NAME, &aside)?;
    }
    rename_set(data_dir, name, DB_FILE_NAME)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, UNIX_EPOCH};

    fn when() -> SystemTime {
        // 2026-10-20T15:30:05Z
        UNIX_EPOCH + Duration::from_secs(1_792_510_205)
    }

    #[test]
    fn archive_names() {
        assert_eq!(archive_name(when(), 1), "shellpp.db.20261020T153005Z.old");
        assert_eq!(archive_name(when(), 2), "shellpp.db.20261020T153005Z-2.old");
        assert_eq!(
            tagged_name(when(), FAILED_TAG, 1),
            "shellpp.db.20261020T153005Z-failed.old"
        );
    }

    #[test]
    fn the_database_and_its_side_files_are_renamed_never_deleted() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path();
        fs::write(path.join("shellpp.db"), b"db").unwrap();
        fs::write(path.join("shellpp.db-wal"), b"wal").unwrap();
        fs::write(path.join("shellpp.lock"), b"").unwrap();

        let first = archive_data_file(path, when()).unwrap();
        assert_eq!(first, "shellpp.db.20261020T153005Z.old");
        assert!(!path.join("shellpp.db").exists());
        assert!(!path.join("shellpp.db-wal").exists());
        assert_eq!(fs::read(path.join(&first)).unwrap(), b"db");
        assert_eq!(fs::read(path.join(format!("{first}-wal"))).unwrap(), b"wal");
        assert!(path.join("shellpp.lock").exists(), "other files stay");

        // A second archive in the same second gets a number.
        fs::write(path.join("shellpp.db"), b"db2").unwrap();
        let second = archive_data_file(path, when()).unwrap();
        assert_eq!(second, "shellpp.db.20261020T153005Z-2.old");
        assert_eq!(fs::read(path.join(&first)).unwrap(), b"db");

        // Without a database there is nothing to archive.
        assert!(archive_data_file(path, when()).is_err());
    }

    #[test]
    fn undo_sets_a_new_database_aside_instead_of_deleting_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path();
        fs::write(path.join("shellpp.db"), b"old").unwrap();
        fs::write(path.join("shellpp.db-wal"), b"old-wal").unwrap();
        let archived = archive_data_file(path, when()).unwrap();

        // A new database was started (with side files), then failed.
        fs::write(path.join("shellpp.db"), b"new").unwrap();
        fs::write(path.join("shellpp.db-shm"), b"new-shm").unwrap();
        restore_archived(path, &archived, when()).unwrap();

        assert_eq!(fs::read(path.join("shellpp.db")).unwrap(), b"old");
        assert_eq!(fs::read(path.join("shellpp.db-wal")).unwrap(), b"old-wal");
        assert!(!path.join("shellpp.db-shm").exists(), "the new -shm moved");
        assert!(!path.join(&archived).exists());
        let aside = "shellpp.db.20261020T153005Z-failed.old";
        assert_eq!(fs::read(path.join(aside)).unwrap(), b"new");
        assert_eq!(
            fs::read(path.join(format!("{aside}-shm"))).unwrap(),
            b"new-shm"
        );

        // A second failure in the same second gets a free name as well.
        let archived = archive_data_file(path, when()).unwrap();
        fs::write(path.join("shellpp.db-wal"), b"stray").unwrap();
        restore_archived(path, &archived, when()).unwrap();
        assert_eq!(fs::read(path.join("shellpp.db")).unwrap(), b"old");
        assert_eq!(
            fs::read(path.join("shellpp.db.20261020T153005Z-failed-2.old-wal")).unwrap(),
            b"stray"
        );
    }

    #[test]
    fn a_rename_never_replaces_an_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path();
        fs::write(path.join("a"), b"a").unwrap();
        fs::write(path.join("a-wal"), b"a-wal").unwrap();
        fs::write(path.join("b-wal"), b"b-wal").unwrap();
        assert!(rename_set(path, "a", "b").is_err());
        // Undone: both sets as before.
        assert_eq!(fs::read(path.join("a")).unwrap(), b"a");
        assert_eq!(fs::read(path.join("a-wal")).unwrap(), b"a-wal");
        assert_eq!(fs::read(path.join("b-wal")).unwrap(), b"b-wal");
        assert!(!path.join("b").exists());
    }
}
