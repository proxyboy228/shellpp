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

/// The entries of the local folder `dir`, in the order of the system.
pub fn list(dir: &Path) -> std::io::Result<Vec<Entry>> {
    let mut out = Vec::new();
    for item in std::fs::read_dir(dir)? {
        let item = item?;
        let name = item.file_name().to_string_lossy().into_owned();
        // The entry itself, never what a link points to.
        let Ok(meta) = std::fs::symlink_metadata(item.path()) else {
            continue;
        };
        let kind = if meta.file_type().is_symlink() {
            Kind::Link {
                to_folder: std::fs::metadata(item.path()).is_ok_and(|target| target.is_dir()),
            }
        } else if meta.is_dir() {
            Kind::Folder
        } else if meta.is_file() {
            Kind::File
        } else {
            Kind::Other
        };
        out.push(Entry {
            hidden: hidden(&name, &meta),
            name,
            kind,
            size: if meta.is_file() { meta.len() } else { 0 },
            modified: meta
                .modified()
                .ok()
                .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                .map(|since| since.as_secs()),
            permissions: None,
        });
    }
    Ok(out)
}

/// The drives of the drive selection (Windows: each letter that has a root
/// folder now; elsewhere `/`).
pub fn drives() -> Vec<PathBuf> {
    if cfg!(windows) {
        ('A'..='Z')
            .map(|letter| PathBuf::from(format!("{letter}:\\")))
            .filter(|root| root.is_dir())
            .collect()
    } else {
        vec![PathBuf::from("/")]
    }
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
        size: entry.size,
        modified: entry.modified,
        permissions: entry.permissions,
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
        assert!(drives.iter().all(|root| root.is_dir()));
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
    }
}
