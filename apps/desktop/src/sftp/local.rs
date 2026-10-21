//! The local file system of a panel (UI 5.3; night decision D48): a
//! folder's entries as [`Entry`] rows, the drives of Windows, and the rows
//! of a remote folder.
//!
//! - A symbolic link (or a Windows junction) is a link, never what it points
//!   to; a link to a folder opens as one.
//! - Hidden: on Windows the hidden or the system attribute, on Unix a name
//!   that starts with a dot. A remote entry is hidden when its name starts
//!   with a dot.

use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use shellpp_ssh::{EntryKind, RemoteEntry};

use super::listing::{Entry, Kind};

/// `FILE_ATTRIBUTE_HIDDEN | FILE_ATTRIBUTE_SYSTEM`.
#[cfg(windows)]
const HIDDEN_OR_SYSTEM: u32 = 0x2 | 0x4;

fn hidden(name: &str, meta: &std::fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt as _;
        let _ = name;
        meta.file_attributes() & HIDDEN_OR_SYSTEM != 0
    }
    #[cfg(not(windows))]
    {
        let _ = meta;
        name.starts_with('.')
    }
}

/// The entries of the local folder `dir`, in the order of the system. The
/// type comes from the folder scan (a link is a link); an entry whose
/// details cannot be read is still listed, without size and date, and one
/// the scan itself cannot read is left out.
pub fn list(dir: &Path) -> std::io::Result<Vec<Entry>> {
    let mut out = Vec::new();
    for item in std::fs::read_dir(dir)? {
        let Ok(item) = item else {
            continue;
        };
        let name = item.file_name().to_string_lossy().into_owned();
        let Ok(file_type) = item.file_type() else {
            continue;
        };
        let kind = if file_type.is_symlink() {
            Kind::Link {
                to_folder: std::fs::metadata(item.path()).is_ok_and(|target| target.is_dir()),
            }
        } else if file_type.is_dir() {
            Kind::Folder
        } else if file_type.is_file() {
            Kind::File
        } else {
            Kind::Other
        };
        // The entry's own details (no link followed; on Windows from the
        // scan itself).
        let meta = item.metadata().ok();
        out.push(Entry {
            hidden: meta
                .as_ref()
                .map_or(name.starts_with('.'), |meta| hidden(&name, meta)),
            kind,
            size: match (&meta, kind) {
                (Some(meta), Kind::File) => meta.len(),
                _ => 0,
            },
            modified: meta
                .as_ref()
                .and_then(|meta| meta.modified().ok())
                .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                .map(|since| since.as_secs()),
            permissions: None,
            local_path: Some(item.path()),
            name,
        });
    }
    Ok(out)
}

/// The drives of the drive selection (Windows: the letters the system has,
/// read without touching any drive; elsewhere `/`; D48, D55). A drive that
/// is not ready fails when it is opened, as any folder that cannot be read.
pub fn drives() -> Vec<PathBuf> {
    crate::platform::drives::roots()
}

/// The row of a remote entry.
pub fn from_remote(entry: &RemoteEntry) -> Entry {
    Entry {
        name: entry.name.clone(),
        kind: match &entry.kind {
            EntryKind::Dir => Kind::Folder,
            EntryKind::File => Kind::File,
            EntryKind::Symlink { to_dir, .. } => Kind::Link { to_folder: *to_dir },
            EntryKind::Other => Kind::Other,
        },
        // A link's own size (the length of its target path) means nothing
        // here; the local panel has none either.
        size: if matches!(entry.kind, EntryKind::Symlink { .. }) {
            0
        } else {
            entry.size
        },
        modified: entry.modified,
        permissions: entry.permissions,
        local_path: None,
        hidden: entry.name.starts_with('.'),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_local_folder_is_listed_with_hidden_entries_marked() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("notes.txt"), "12345").unwrap();
        std::fs::create_dir(dir.path().join("src")).unwrap();
        let secret = if cfg!(windows) {
            "desktop.ini"
        } else {
            ".hidden"
        };
        std::fs::write(dir.path().join(secret), "x").unwrap();
        #[cfg(windows)]
        {
            let status = std::process::Command::new("attrib")
                .arg("+h")
                .arg(dir.path().join(secret))
                .status()
                .expect("attrib runs");
            assert!(status.success());
        }
        let mut entries = list(dir.path()).unwrap();
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        let find = |name: &str| entries.iter().find(|entry| entry.name == name).unwrap();
        assert_eq!(find("notes.txt").kind, Kind::File);
        assert_eq!(find("notes.txt").size, 5);
        assert!(!find("notes.txt").hidden);
        assert!(find("notes.txt").modified.is_some());
        assert_eq!(find("src").kind, Kind::Folder);
        assert!(find(secret).hidden);
        assert_eq!(
            find("notes.txt").local_path.as_deref(),
            Some(dir.path().join("notes.txt").as_path())
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_link_is_listed_as_a_link() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("target")).unwrap();
        std::os::unix::fs::symlink(dir.path().join("target"), dir.path().join("link")).unwrap();
        let entries = list(dir.path()).unwrap();
        let link = entries.iter().find(|entry| entry.name == "link").unwrap();
        assert_eq!(link.kind, Kind::Link { to_folder: true });
    }

    #[test]
    fn drives_and_remote_rows() {
        let drives = drives();
        assert!(!drives.is_empty());
        assert!(drives.iter().all(|root| root.is_absolute()));
        let remote = RemoteEntry {
            name: ".profile".to_owned(),
            kind: EntryKind::Symlink {
                target: Some("/etc/skel/.profile".to_owned()),
                to_dir: false,
            },
            size: 807,
            modified: Some(1_700_000_000),
            permissions: Some(0o777),
        };
        let row = from_remote(&remote);
        assert_eq!(row.kind, Kind::Link { to_folder: false });
        assert!(row.hidden);
        assert_eq!(row.permissions, Some(0o777));
        assert_eq!(row.size, 0, "a link's own size is not shown");
    }
}
