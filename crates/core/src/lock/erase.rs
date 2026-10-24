//! Log out with "Also delete the local copy" (UI 12.2; decision D197): the
//! local copy is deleted for real, not set aside. Only the names the app
//! writes go, so a data directory the user chose (`SHELLPP_DATA_DIR`) keeps
//! anything else: the database with its side files, every set-aside copy
//! (`shellpp.db.<stamp>[-failed][-<n>].old` with its side files), the
//! backups (`backups/shellpp-….db`), the recordings and their keys
//! (`recordings/<uuid>.…`), and the temporary directory, which the app
//! empties at every start anyway. A folder is removed only once it is empty;
//! the lock file and the logs (no secrets) stay.
//!
//! The caller closes the database first and holds the lock of the data
//! directory all along, as for "Start with a new vault" ([`super::new_vault`]).

use std::fs;
use std::io;
use std::path::Path;
use std::time::SystemTime;

use uuid::Uuid;

use super::new_vault::ARCHIVE_SUFFIX;
use crate::ids::{BACKUPS_DIR_NAME, DB_FILE_NAME};
use crate::recording;

/// The temporary directory inside the data directory (Windows and
/// `SHELLPP_DATA_DIR`; docs/data-format.md, section 1).
const TMP_DIR_NAME: &str = "tmp";

/// What a file the server does not have is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LocalFileKind {
    /// A database set aside by "Start with a new vault" or an earlier
    /// sign-out (`shellpp.db.<stamp>.old`).
    SetAside,
    /// A copy taken before a schema migration or a sign-in merge
    /// (`backups/shellpp-<stamp>.db`).
    Backup,
}

/// A file of the local copy that the server does not have, listed in the
/// confirmation before it is deleted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalFile {
    /// Relative to the data directory, with `/`.
    pub name: String,
    pub kind: LocalFileKind,
    pub modified: Option<SystemTime>,
    pub bytes: u64,
}

fn set_aside_prefix() -> String {
    format!("{DB_FILE_NAME}.")
}

/// `shellpp.db.<yyyymmddThhmmssZ>[-failed][-<n>].old`, the name the app
/// gives the database of a set-aside copy (`super::new_vault`); a file
/// the user named `shellpp.db.mine.old` is not one (review of run 10).
fn is_set_aside(name: &str) -> bool {
    let Some(rest) = name
        .strip_prefix(&set_aside_prefix())
        .and_then(|rest| rest.strip_suffix(ARCHIVE_SUFFIX))
    else {
        return false;
    };
    let bytes = rest.as_bytes();
    if bytes.len() < 16 {
        return false;
    }
    let stamp_ok = bytes[..16].iter().enumerate().all(|(i, b)| match i {
        8 => *b == b'T',
        15 => *b == b'Z',
        _ => b.is_ascii_digit(),
    });
    let tail = &rest[16..];
    let tail = tail.strip_prefix("-failed").unwrap_or(tail);
    let tail_ok = tail.is_empty()
        || tail
            .strip_prefix('-')
            .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()));
    stamp_ok && tail_ok
}

/// A set-aside copy or one of its side files (`….old-wal`, `….old-shm`).
fn is_set_aside_file(name: &str) -> bool {
    let database = name
        .strip_suffix("-wal")
        .or_else(|| name.strip_suffix("-shm"))
        .unwrap_or(name);
    is_set_aside(database)
}

/// `backups/shellpp-<yyyymmddThhmmssZ>.db`, a copy the app took before a
/// migration or a merge (`crate::store`).
fn is_backup(name: &str) -> bool {
    name.strip_prefix("shellpp-")
        .and_then(|rest| rest.strip_suffix(".db"))
        .is_some_and(|stamp| {
            stamp.len() == 16
                && stamp.bytes().enumerate().all(|(i, b)| match i {
                    8 => b == b'T',
                    15 => b == b'Z',
                    _ => b.is_ascii_digit(),
                })
        })
}

/// `recordings/<uuid>.…`: a recording, its `.rekey` file, or a
/// re-encryption's temporary file.
fn is_recording_file(name: &str) -> bool {
    name.get(..36).is_some_and(|id| Uuid::parse_str(id).is_ok())
        && name.as_bytes().get(36) == Some(&b'.')
}

/// The set-aside copies and the backups of `data_dir`, by name.
pub fn files_not_on_server(data_dir: &Path) -> Vec<LocalFile> {
    let mut files = Vec::new();
    let places = [
        (data_dir.to_path_buf(), LocalFileKind::SetAside, ""),
        (
            data_dir.join(BACKUPS_DIR_NAME),
            LocalFileKind::Backup,
            BACKUPS_DIR_NAME,
        ),
    ];
    for (dir, kind, prefix) in places {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let ours = match kind {
                LocalFileKind::SetAside => is_set_aside(&name),
                LocalFileKind::Backup => is_backup(&name),
            };
            if !ours {
                continue;
            }
            let Ok(metadata) = entry.metadata() else {
                continue;
            };
            if !metadata.is_file() {
                continue;
            }
            files.push(LocalFile {
                name: if prefix.is_empty() {
                    name
                } else {
                    format!("{prefix}/{name}")
                },
                kind,
                modified: metadata.modified().ok(),
                bytes: metadata.len(),
            });
        }
    }
    files.sort_by(|a, b| a.name.cmp(&b.name));
    files
}

/// The recordings in `data_dir`: how many, and their bytes (they never go
/// to the server; the confirmation counts them, D197).
pub fn recordings(data_dir: &Path) -> (usize, u64) {
    let suffix = format!(".{}", recording::EXTENSION);
    files_in(data_dir, recording::DIR_NAME, is_recording_file)
        .iter()
        .filter(|name| name.ends_with(&suffix))
        .filter_map(|name| fs::metadata(data_dir.join(name)).ok())
        .fold((0, 0), |(count, bytes), metadata| {
            (count + 1, bytes + metadata.len())
        })
}

/// The database ids of the set-aside copies, read from their `meta` table,
/// so their device keys can leave the OS keychain with them. A copy that
/// does not open is skipped.
pub fn set_aside_db_ids(data_dir: &Path) -> Vec<Uuid> {
    let Ok(entries) = fs::read_dir(data_dir) else {
        return Vec::new();
    };
    let mut ids = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !is_set_aside(&name) {
            continue;
        }
        let flags =
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX;
        let Ok(connection) = rusqlite::Connection::open_with_flags(entry.path(), flags) else {
            continue;
        };
        let id: Option<String> = connection
            .query_row(
                "SELECT value FROM meta WHERE key = ?1",
                [crate::store::META_DB_ID],
                |row| row.get(0),
            )
            .ok();
        if let Some(id) = id.and_then(|id| Uuid::parse_str(&id).ok()) {
            ids.push(id);
        }
    }
    ids
}

fn remove(path: &Path) -> io::Result<()> {
    let result = if path.is_dir() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    };
    match result {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        other => other,
    }
}

/// The files of `dir` (relative names `<folder>/<name>`) that `ours` names.
fn files_in(data_dir: &Path, folder: &str, ours: fn(&str) -> bool) -> Vec<String> {
    let Ok(entries) = fs::read_dir(data_dir.join(folder)) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .flatten()
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| ours(name))
        .map(|name| format!("{folder}/{name}"))
        .collect();
    names.sort();
    names
}

/// Deletes the closed database of `data_dir`, then the rest of the local
/// copy. The database's side files go first and the database last: when
/// one of them cannot be deleted, nothing else is touched and the error is
/// returned (the caller keeps the database, checkpointed, so its side files
/// hold nothing; and no new database ever meets an old write-ahead log).
/// Afterwards every name is tried, and the names that stayed are returned.
pub fn erase_data_files(data_dir: &Path) -> io::Result<Vec<String>> {
    for suffix in ["-wal", "-shm", ""] {
        remove(&data_dir.join(format!("{DB_FILE_NAME}{suffix}")))?;
    }
    let mut names: Vec<String> = Vec::new();
    if let Ok(entries) = fs::read_dir(data_dir) {
        let mut set_aside: Vec<String> = entries
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| is_set_aside_file(name))
            .collect();
        set_aside.sort();
        names.extend(set_aside);
    }
    names.extend(files_in(data_dir, BACKUPS_DIR_NAME, is_backup));
    names.extend(files_in(data_dir, recording::DIR_NAME, is_recording_file));
    names.push(TMP_DIR_NAME.to_owned());
    let mut left = Vec::new();
    for name in names {
        if let Err(error) = remove(&data_dir.join(&name)) {
            tracing::warn!(kind = ?error.kind(), "a file of the local copy was not deleted");
            left.push(name);
        }
    }
    // A folder of the app goes when nothing else is in it.
    for folder in [BACKUPS_DIR_NAME, recording::DIR_NAME] {
        let _ = fs::remove_dir(data_dir.join(folder));
    }
    Ok(left)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch(path: &Path, bytes: usize) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, vec![0u8; bytes]).unwrap();
    }

    #[test]
    fn the_local_copy_goes_and_nothing_else() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        let recording = format!("recordings/{}.shpprec", Uuid::now_v7());
        let rekey = format!("recordings/{}.{}.rekey", Uuid::now_v7(), Uuid::now_v7());
        for name in [
            "shellpp.db",
            "shellpp.db-wal",
            "shellpp.db.20261020T101500Z.old",
            "shellpp.db.20261020T101500Z.old-wal",
            "shellpp.db.20261021T000000Z-failed.old",
            "shellpp.db.20261021T000000Z-2.old",
            "shellpp.lock",
            "notes.txt",
            "shellpp.db.backup-by-hand",
            "shellpp.db.mine.old",
            "backups/shellpp-20261020T101500Z.db",
            "backups/mine/x.txt",
            recording.as_str(),
            rekey.as_str(),
            "recordings/song.mp3",
            "tmp/x",
            "logs/shellpp.log",
        ] {
            touch(&d.join(name), 10);
        }
        assert_eq!(
            files_not_on_server(d)
                .iter()
                .map(|file| (file.name.as_str(), file.kind))
                .collect::<Vec<_>>(),
            vec![
                ("backups/shellpp-20261020T101500Z.db", LocalFileKind::Backup),
                ("shellpp.db.20261020T101500Z.old", LocalFileKind::SetAside),
                ("shellpp.db.20261021T000000Z-2.old", LocalFileKind::SetAside),
                (
                    "shellpp.db.20261021T000000Z-failed.old",
                    LocalFileKind::SetAside
                ),
            ]
        );
        assert_eq!(erase_data_files(d).unwrap(), Vec::<String>::new());
        let mut left: Vec<String> = fs::read_dir(d)
            .unwrap()
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        left.sort();
        assert_eq!(
            left,
            vec![
                "backups",
                "logs",
                "notes.txt",
                "recordings",
                "shellpp.db.backup-by-hand",
                "shellpp.db.mine.old",
                "shellpp.lock"
            ]
        );
        // What the user put in the app's folders stays.
        assert!(d.join("backups/mine/x.txt").is_file());
        assert!(d.join("recordings/song.mp3").is_file());
        assert!(!d.join(&recording).exists() && !d.join(&rekey).exists());
        // Again, with nothing left to delete.
        assert_eq!(erase_data_files(d).unwrap(), Vec::<String>::new());
    }
}
