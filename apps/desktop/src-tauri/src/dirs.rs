//! Data, log and temporary directories (docs/architecture.md, section 5.3;
//! docs/storage-format.md, section 2.1).
//!
//! | OS | Data | Logs | Temporary files |
//! |---|---|---|---|
//! | Windows | `%LOCALAPPDATA%\shellpp` | `<data>\logs` | `<data>\tmp` |
//! | macOS | `~/Library/Application Support/shellpp` | `~/Library/Logs/shellpp` | `~/Library/Caches/shellpp/tmp` |
//! | Linux | `$XDG_DATA_HOME/shellpp` | `$XDG_STATE_HOME/shellpp/logs` | `$XDG_CACHE_HOME/shellpp/tmp` |
//!
//! `SHELLPP_DATA_DIR` replaces all three with `<dir>`, `<dir>/logs` and
//! `<dir>/tmp`, so test runs never share or wipe each other's directories.
//! With the override the webview profile (WebView2 / WebKitGTK data:
//! `localStorage`, caches) also moves to `<dir>/webview`, so e2e runs and a
//! developer's running instance never share webview state; without it the
//! Tauri default location is used.
//! Directories are created with mode `0700` on Unix.

use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use shellpp_core::ids;

/// The resolved directories of this process.
#[derive(Clone, PartialEq, Eq)]
pub struct AppDirs {
    /// The data directory: `shellpp.db`, backups, recordings.
    pub data: PathBuf,
    /// Diagnostic logs (`shellpp.log`).
    pub logs: PathBuf,
    /// Temporary plaintext files; emptied at start and exit.
    pub tmp: PathBuf,
    /// Webview profile directory; set only with `SHELLPP_DATA_DIR`.
    pub webview: Option<PathBuf>,
}

// Paths contain the user name on most systems; keep them out of Debug output
// that might end up in a log line (docs/security.md, section 9.1).
impl std::fmt::Debug for AppDirs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AppDirs { .. }")
    }
}

/// Why the directories could not be resolved or created.
#[derive(Debug, thiserror::Error)]
pub enum DirsError {
    #[error("the OS did not report a base directory")]
    NoBaseDirectory,
    #[error("could not create a directory")]
    Create(#[source] io::Error),
}

/// Base directories as reported by the OS (injected for tests).
#[derive(Clone, Debug, Default)]
pub struct BaseDirs {
    pub home: Option<PathBuf>,
    pub data: Option<PathBuf>,
    pub data_local: Option<PathBuf>,
    pub state: Option<PathBuf>,
    pub cache: Option<PathBuf>,
}

impl BaseDirs {
    /// The real base directories of the current user.
    pub fn from_os() -> Self {
        Self {
            home: ::dirs::home_dir(),
            data: ::dirs::data_dir(),
            data_local: ::dirs::data_local_dir(),
            state: ::dirs::state_dir(),
            cache: ::dirs::cache_dir(),
        }
    }
}

/// Target OS family for the directory table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OsFamily {
    Windows,
    Macos,
    Linux,
}

impl OsFamily {
    /// The OS this binary was built for (other Unix systems use the Linux layout).
    pub const fn current() -> Self {
        if cfg!(target_os = "windows") {
            Self::Windows
        } else if cfg!(target_os = "macos") {
            Self::Macos
        } else {
            Self::Linux
        }
    }
}

/// Resolves the directories from the environment of this process.
pub fn resolve_current() -> Result<AppDirs, DirsError> {
    resolve(
        std::env::var_os(ids::ENV_DATA_DIR),
        OsFamily::current(),
        &BaseDirs::from_os(),
    )
}

/// Resolves the directories. `override_dir` is the value of `SHELLPP_DATA_DIR`
/// (an empty value counts as unset); a relative override is made absolute
/// against the current directory.
pub fn resolve(
    override_dir: Option<OsString>,
    os: OsFamily,
    base: &BaseDirs,
) -> Result<AppDirs, DirsError> {
    if let Some(dir) = override_dir.filter(|value| !value.is_empty()) {
        let dir = PathBuf::from(dir);
        let dir = std::path::absolute(&dir).unwrap_or(dir);
        return Ok(AppDirs {
            logs: dir.join("logs"),
            tmp: dir.join("tmp"),
            webview: Some(dir.join("webview")),
            data: dir,
        });
    }
    let name = ids::DATA_DIR_NAME;
    let need = |dir: &Option<PathBuf>| dir.clone().ok_or(DirsError::NoBaseDirectory);
    match os {
        OsFamily::Windows => {
            let data = need(&base.data_local)?.join(name);
            Ok(AppDirs {
                logs: data.join("logs"),
                tmp: data.join("tmp"),
                webview: None,
                data,
            })
        }
        OsFamily::Macos => Ok(AppDirs {
            data: need(&base.data)?.join(name),
            logs: need(&base.home)?.join("Library").join("Logs").join(name),
            tmp: need(&base.cache)?.join(name).join("tmp"),
            webview: None,
        }),
        OsFamily::Linux => Ok(AppDirs {
            data: need(&base.data)?.join(name),
            logs: need(&base.state)?.join(name).join("logs"),
            tmp: need(&base.cache)?.join(name).join("tmp"),
            webview: None,
        }),
    }
}

impl AppDirs {
    /// Creates the three directories (Unix: mode 0700).
    pub fn create(&self) -> Result<(), DirsError> {
        for dir in [&self.data, &self.logs, &self.tmp] {
            create_private_dir(dir).map_err(DirsError::Create)?;
        }
        Ok(())
    }

    /// Deletes everything inside `tmp/` (docs/storage-format.md, section 9).
    /// Entries that cannot be removed (a file still open in another program on
    /// Windows) are skipped and removed at the next start. Returns the number
    /// of entries that could not be removed.
    pub fn empty_tmp(&self) -> usize {
        empty_dir(&self.tmp)
    }
}

fn create_private_dir(dir: &Path) -> io::Result<()> {
    fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

fn empty_dir(dir: &Path) -> usize {
    let Ok(entries) = fs::read_dir(dir) else {
        return 0;
    };
    let mut failed = 0;
    for entry in entries.flatten() {
        let path = entry.path();
        let is_dir = entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false);
        let result = if is_dir {
            fs::remove_dir_all(&path)
        } else {
            fs::remove_file(&path)
        };
        if result.is_err() {
            failed += 1;
        }
    }
    failed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> BaseDirs {
        BaseDirs {
            home: Some(PathBuf::from("/home/u")),
            data: Some(PathBuf::from("/home/u/.local/share")),
            data_local: Some(PathBuf::from("C:/Users/u/AppData/Local")),
            state: Some(PathBuf::from("/home/u/.local/state")),
            cache: Some(PathBuf::from("/home/u/.cache")),
        }
    }

    #[test]
    fn windows_uses_the_local_profile() {
        let dirs = resolve(None, OsFamily::Windows, &base()).unwrap();
        let root = PathBuf::from("C:/Users/u/AppData/Local").join("shellpp");
        assert_eq!(dirs.data, root);
        assert_eq!(dirs.logs, root.join("logs"));
        assert_eq!(dirs.tmp, root.join("tmp"));
        assert_eq!(dirs.webview, None);
    }

    #[test]
    fn macos_layout() {
        let dirs = resolve(None, OsFamily::Macos, &base()).unwrap();
        assert_eq!(dirs.data, PathBuf::from("/home/u/.local/share/shellpp"));
        assert_eq!(
            dirs.logs,
            PathBuf::from("/home/u")
                .join("Library")
                .join("Logs")
                .join("shellpp")
        );
        assert_eq!(dirs.tmp, PathBuf::from("/home/u/.cache/shellpp/tmp"));
    }

    #[test]
    fn linux_layout() {
        let dirs = resolve(None, OsFamily::Linux, &base()).unwrap();
        assert_eq!(dirs.data, PathBuf::from("/home/u/.local/share/shellpp"));
        assert_eq!(
            dirs.logs,
            PathBuf::from("/home/u/.local/state/shellpp/logs")
        );
        assert_eq!(dirs.tmp, PathBuf::from("/home/u/.cache/shellpp/tmp"));
    }

    #[test]
    fn override_replaces_all_three() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("portable");
        for os in [OsFamily::Windows, OsFamily::Macos, OsFamily::Linux] {
            let dirs = resolve(Some(root.clone().into_os_string()), os, &base()).unwrap();
            assert_eq!(dirs.data, root);
            assert_eq!(dirs.logs, root.join("logs"));
            assert_eq!(dirs.tmp, root.join("tmp"));
            assert_eq!(dirs.webview, Some(root.join("webview")));
        }
    }

    #[test]
    fn empty_override_counts_as_unset() {
        let dirs = resolve(Some(OsString::new()), OsFamily::Linux, &base()).unwrap();
        assert_eq!(dirs.data, PathBuf::from("/home/u/.local/share/shellpp"));
    }

    #[test]
    fn relative_override_becomes_absolute() {
        let dirs = resolve(Some(OsString::from("rel-dir")), OsFamily::Linux, &base()).unwrap();
        assert!(dirs.data.is_absolute());
        assert!(dirs.data.ends_with("rel-dir"));
        assert_eq!(dirs.logs, dirs.data.join("logs"));
    }

    #[test]
    fn missing_base_directory_is_an_error() {
        let result = resolve(None, OsFamily::Linux, &BaseDirs::default());
        assert!(matches!(result, Err(DirsError::NoBaseDirectory)));
    }

    #[test]
    fn create_and_empty_tmp() {
        let temp = tempfile::tempdir().unwrap();
        let dirs = resolve(
            Some(temp.path().join("d").into_os_string()),
            OsFamily::current(),
            &base(),
        )
        .unwrap();
        dirs.create().unwrap();
        assert!(dirs.data.is_dir() && dirs.logs.is_dir() && dirs.tmp.is_dir());
        fs::create_dir_all(dirs.tmp.join("a").join("b")).unwrap();
        fs::write(dirs.tmp.join("a").join("b").join("f.txt"), b"x").unwrap();
        fs::write(dirs.tmp.join("g.txt"), b"y").unwrap();
        assert_eq!(dirs.empty_tmp(), 0);
        assert!(dirs.tmp.is_dir());
        assert_eq!(fs::read_dir(&dirs.tmp).unwrap().count(), 0);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&dirs.data).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o700);
        }
    }

    #[test]
    fn debug_hides_paths() {
        let dirs = resolve(None, OsFamily::Linux, &base()).unwrap();
        assert_eq!(format!("{dirs:?}"), "AppDirs { .. }");
    }
}
