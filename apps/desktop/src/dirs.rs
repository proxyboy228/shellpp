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
//! Directories are created with mode `0700` on Unix.
//!
//! Test isolation (owner's step 0): in a test build of this crate
//! (`cfg(test)`) or with `SHELLPP_TEST=1`, resolving the directories without
//! `SHELLPP_DATA_DIR` fails with [`DirsError::PermanentProfileInTest`]
//! instead of falling back to the permanent profile. Tests normally never
//! resolve directories at all: they use [`crate::testing::TestProfile`].

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
    /// A test run asked for the permanent profile.
    #[error(
        "refusing to use the permanent profile in a test run (SHELLPP_TEST=1 or a test build): \
         set SHELLPP_DATA_DIR to a temporary directory"
    )]
    PermanentProfileInTest,
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

/// Whether this process is a test run: a test build of this crate, or
/// `SHELLPP_TEST=1` in the environment (integration tests, scripts).
pub fn is_test_run() -> bool {
    cfg!(test) || std::env::var_os(crate::ENV_TEST).is_some_and(|value| value == "1")
}

/// Resolves the directories from the environment of this process.
pub fn resolve_current() -> Result<AppDirs, DirsError> {
    resolve(
        std::env::var_os(ids::ENV_DATA_DIR),
        is_test_run(),
        OsFamily::current(),
        &BaseDirs::from_os(),
    )
}

/// Resolves the directories. `override_dir` is the value of `SHELLPP_DATA_DIR`
/// (an empty value counts as unset); a relative override is made absolute
/// against the current directory. With `test_run`, a missing override is an
/// error instead of the permanent profile.
pub fn resolve(
    override_dir: Option<OsString>,
    test_run: bool,
    os: OsFamily,
    base: &BaseDirs,
) -> Result<AppDirs, DirsError> {
    if let Some(dir) = override_dir.filter(|value| !value.is_empty()) {
        return Ok(AppDirs::under(PathBuf::from(dir)));
    }
    if test_run {
        return Err(DirsError::PermanentProfileInTest);
    }
    permanent(os, base)
}

/// The permanent profile of this user (never created here).
pub fn permanent(os: OsFamily, base: &BaseDirs) -> Result<AppDirs, DirsError> {
    let name = ids::DATA_DIR_NAME;
    let need = |dir: &Option<PathBuf>| dir.clone().ok_or(DirsError::NoBaseDirectory);
    match os {
        OsFamily::Windows => {
            let data = need(&base.data_local)?.join(name);
            Ok(AppDirs {
                logs: data.join("logs"),
                tmp: data.join("tmp"),
                data,
            })
        }
        OsFamily::Macos => Ok(AppDirs {
            data: need(&base.data)?.join(name),
            logs: need(&base.home)?.join("Library").join("Logs").join(name),
            tmp: need(&base.cache)?.join(name).join("tmp"),
        }),
        OsFamily::Linux => Ok(AppDirs {
            data: need(&base.data)?.join(name),
            logs: need(&base.state)?.join(name).join("logs"),
            tmp: need(&base.cache)?.join(name).join("tmp"),
        }),
    }
}

impl AppDirs {
    /// The layout of `SHELLPP_DATA_DIR` (and of temporary profiles):
    /// `<dir>`, `<dir>/logs`, `<dir>/tmp`. A relative path is made absolute.
    pub fn under(dir: PathBuf) -> Self {
        let dir = std::path::absolute(&dir).unwrap_or(dir);
        Self {
            logs: dir.join("logs"),
            tmp: dir.join("tmp"),
            data: dir,
        }
    }

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

pub(crate) fn create_private_dir(dir: &Path) -> io::Result<()> {
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
    use crate::profile::TempProfile;

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
        let dirs = resolve(None, false, OsFamily::Windows, &base()).unwrap();
        let root = PathBuf::from("C:/Users/u/AppData/Local").join("shellpp");
        assert_eq!(dirs.data, root);
        assert_eq!(dirs.logs, root.join("logs"));
        assert_eq!(dirs.tmp, root.join("tmp"));
    }

    #[test]
    fn macos_layout() {
        let dirs = resolve(None, false, OsFamily::Macos, &base()).unwrap();
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
        let dirs = resolve(None, false, OsFamily::Linux, &base()).unwrap();
        assert_eq!(dirs.data, PathBuf::from("/home/u/.local/share/shellpp"));
        assert_eq!(
            dirs.logs,
            PathBuf::from("/home/u/.local/state/shellpp/logs")
        );
        assert_eq!(dirs.tmp, PathBuf::from("/home/u/.cache/shellpp/tmp"));
    }

    #[test]
    fn override_replaces_all_three() {
        let profile = TempProfile::create("shellpp-test-dirs").unwrap();
        let root = profile.path().join("portable");
        for os in [OsFamily::Windows, OsFamily::Macos, OsFamily::Linux] {
            for test_run in [false, true] {
                let dirs =
                    resolve(Some(root.clone().into_os_string()), test_run, os, &base()).unwrap();
                assert_eq!(dirs.data, root);
                assert_eq!(dirs.logs, root.join("logs"));
                assert_eq!(dirs.tmp, root.join("tmp"));
            }
        }
    }

    #[test]
    fn empty_override_counts_as_unset() {
        let dirs = resolve(Some(OsString::new()), false, OsFamily::Linux, &base()).unwrap();
        assert_eq!(dirs.data, PathBuf::from("/home/u/.local/share/shellpp"));
    }

    #[test]
    fn relative_override_becomes_absolute() {
        let dirs = resolve(
            Some(OsString::from("rel-dir")),
            false,
            OsFamily::Linux,
            &base(),
        )
        .unwrap();
        assert!(dirs.data.is_absolute());
        assert!(dirs.data.ends_with("rel-dir"));
        assert_eq!(dirs.logs, dirs.data.join("logs"));
    }

    #[test]
    fn missing_base_directory_is_an_error() {
        let result = resolve(None, false, OsFamily::Linux, &BaseDirs::default());
        assert!(matches!(result, Err(DirsError::NoBaseDirectory)));
    }

    /// Step 0 guard: a test run never falls back to the permanent profile,
    /// on any OS layout, empty override included.
    #[test]
    fn test_runs_refuse_the_permanent_profile() {
        for os in [OsFamily::Windows, OsFamily::Macos, OsFamily::Linux] {
            for override_dir in [None, Some(OsString::new())] {
                let result = resolve(override_dir, true, os, &base());
                assert!(
                    matches!(result, Err(DirsError::PermanentProfileInTest)),
                    "{os:?}"
                );
            }
        }
    }

    /// This unit-test binary is a test build, so the guard is always on here,
    /// whatever the environment says.
    #[test]
    fn unit_tests_are_always_a_test_run() {
        assert!(is_test_run());
        if std::env::var_os(ids::ENV_DATA_DIR).is_none_or(|value| value.is_empty()) {
            assert!(matches!(
                resolve_current(),
                Err(DirsError::PermanentProfileInTest)
            ));
        }
    }

    #[test]
    fn create_and_empty_tmp() {
        let profile = TempProfile::create("shellpp-test-dirs").unwrap();
        let dirs = AppDirs::under(profile.path().join("d"));
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
        let dirs = resolve(None, false, OsFamily::Linux, &base()).unwrap();
        assert_eq!(format!("{dirs:?}"), "AppDirs { .. }");
    }
}
