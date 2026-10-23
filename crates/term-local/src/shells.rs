//! Shell detection (UI 12.4 "Local terminal shell").
//!
//! Ids are stable strings used in `terminal.local_shell` and `local_open`:
//! `powershell`, `pwsh`, `cmd`, `wsl:<distribution>`, `git-bash` on Windows;
//! the absolute path (for example `/bin/zsh`) on macOS and Linux.
//!
//! Windows (in display order):
//! - `powershell`: Windows PowerShell,
//!   `%SystemRoot%\System32\WindowsPowerShell\v1.0\powershell.exe -NoLogo`;
//! - `pwsh`: PowerShell 7, `pwsh.exe` on `PATH` or
//!   `%ProgramFiles%\PowerShell\7\pwsh.exe`, `-NoLogo`;
//! - `cmd`: Command Prompt, `%ComSpec%`;
//! - `wsl:<distribution>`: every WSL distribution from
//!   `wsl.exe --list --quiet` (empty and `docker-desktop*` entries skipped),
//!   started with `wsl.exe -d <distribution>`; the listing runs in a job
//!   object with a 3 s timeout, which ends its whole process tree (D100);
//! - `git-bash`: Git Bash, `%ProgramFiles%\Git\bin\bash.exe --login -i`
//!   (or the per-user install under `%LocalAppData%\Programs\Git`).
//!
//! The default is `pwsh` (PowerShell 7) when it is installed, else
//! `powershell` (stage 2b, owner's step 0.5).
//!
//! macOS and Linux: every existing executable listed in `/etc/shells`, then
//! `$SHELL` when it is not listed but exists, de-duplicated by real path;
//! `/bin/sh` when nothing else is found. Id = the listed path, label = its
//! file name. bash, zsh, fish and the sh family start as login shells (`-l`).
//! The default is `$SHELL` when it is detected, else the first entry.

use std::path::{Path, PathBuf};

use crate::LocalError;

/// A detected shell.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShellInfo {
    pub id: String,
    /// Display name, for example `PowerShell 7` or `zsh`.
    pub label: String,
    pub program: PathBuf,
    pub args: Vec<String>,
}

/// What [`crate::spawn`] starts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShellSpec {
    /// The detected shell's id, or `None` for a custom path.
    pub id: Option<String>,
    pub label: String,
    pub program: PathBuf,
    pub args: Vec<String>,
}

impl From<&ShellInfo> for ShellSpec {
    fn from(shell: &ShellInfo) -> Self {
        Self {
            id: Some(shell.id.clone()),
            label: shell.label.clone(),
            program: shell.program.clone(),
            args: shell.args.clone(),
        }
    }
}

/// Every shell found on this machine, in display order.
///
/// Blocking: on Windows it runs `wsl.exe --list --quiet` (at most about
/// 3 s); call it from a blocking-friendly context.
pub fn detect_shells() -> Vec<ShellInfo> {
    platform::detect()
}

/// The default shell: PowerShell 7 (`pwsh`) on Windows when it is
/// installed, else Windows PowerShell; the login shell (`$SHELL`, else the
/// first `/etc/shells` entry that exists) elsewhere. Falls back to the first
/// detected shell; `None` when `shells` is empty.
pub fn default_shell_id(shells: &[ShellInfo]) -> Option<String> {
    platform::default_id(shells)
}

/// The shell to start: `requested_id` (from `local_open`) wins, then the
/// `terminal.local_shell` setting (an id from `shells` or a custom absolute
/// path to an existing executable), then the default.
///
/// - an unknown `requested_id` -> [`LocalError::NoShell`];
/// - a setting that is neither a detected id nor an absolute path to an
///   existing regular file (executable on Unix) -> [`LocalError::InvalidShell`];
///   an empty setting counts as unset;
/// - no default (nothing detected) -> [`LocalError::NoShell`].
///
/// A custom shell starts without arguments on Windows and with the login
/// argument of its family on Unix (see the module documentation).
pub fn resolve_shell(
    requested_id: Option<&str>,
    setting: Option<&str>,
    shells: &[ShellInfo],
) -> Result<ShellSpec, LocalError> {
    if let Some(id) = requested_id {
        return find(shells, id)
            .map(ShellSpec::from)
            .ok_or(LocalError::NoShell);
    }
    if let Some(setting) = setting.map(str::trim).filter(|s| !s.is_empty()) {
        if let Some(shell) = find(shells, setting) {
            return Ok(ShellSpec::from(shell));
        }
        return custom_shell(Path::new(setting));
    }
    default_shell_id(shells)
        .and_then(|id| find(shells, &id))
        .map(ShellSpec::from)
        .ok_or(LocalError::NoShell)
}

fn find<'a>(shells: &'a [ShellInfo], id: &str) -> Option<&'a ShellInfo> {
    shells.iter().find(|shell| shell.id == id)
}

/// A custom shell from the setting: an absolute path to an existing file.
fn custom_shell(path: &Path) -> Result<ShellSpec, LocalError> {
    if !path.is_absolute() || !is_executable_file(path) {
        return Err(LocalError::InvalidShell);
    }
    let label = file_label(path).ok_or(LocalError::InvalidShell)?;
    Ok(ShellSpec {
        id: None,
        args: platform::custom_args(path),
        label,
        program: path.to_path_buf(),
    })
}

/// The file name of `path` as a display label.
fn file_label(path: &Path) -> Option<String> {
    path.file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
}

/// An existing regular file (following links); on Unix also executable.
fn is_executable_file(path: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    if !meta.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

#[cfg(windows)]
use windows as platform;

#[cfg(not(windows))]
use unix as platform;

/// Windows detection.
#[cfg_attr(not(windows), allow(dead_code))]
mod windows {
    use std::ffi::OsString;
    use std::path::{Path, PathBuf};
    use std::time::Duration;

    use super::{ShellInfo, is_executable_file};

    pub(super) const POWERSHELL: &str = "powershell";
    pub(super) const PWSH: &str = "pwsh";
    pub(super) const CMD: &str = "cmd";
    pub(super) const GIT_BASH: &str = "git-bash";
    pub(super) const WSL_PREFIX: &str = "wsl:";

    /// How long `wsl.exe --list --quiet` may take.
    pub(super) const WSL_TIMEOUT: Duration = Duration::from_secs(3);

    #[cfg(windows)]
    pub(super) fn detect() -> Vec<ShellInfo> {
        let env = |name: &str| std::env::var_os(name).filter(|value| !value.is_empty());
        let system_root = env("SystemRoot")
            .or_else(|| env("windir"))
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
        let system32 = system_root.join("System32");
        let program_files = env("ProgramFiles").map(PathBuf::from);
        let local_app_data = env("LocalAppData").map(PathBuf::from);
        let path = env("PATH");

        let mut shells = Vec::new();

        let powershell = system32.join(r"WindowsPowerShell\v1.0\powershell.exe");
        if is_executable_file(&powershell) {
            shells.push(info(POWERSHELL, "PowerShell", powershell, &["-NoLogo"]));
        }

        let pwsh = find_on_path(path.as_ref(), "pwsh.exe").or_else(|| {
            program_files
                .as_ref()
                .map(|dir| dir.join(r"PowerShell\7\pwsh.exe"))
                .filter(|p| is_executable_file(p))
        });
        if let Some(pwsh) = pwsh {
            shells.push(info(PWSH, "PowerShell 7", pwsh, &["-NoLogo"]));
        }

        let cmd = env("ComSpec")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute() && is_executable_file(p))
            .or_else(|| Some(system32.join("cmd.exe")).filter(|p| is_executable_file(p)));
        if let Some(cmd) = cmd {
            shells.push(info(CMD, "Command Prompt", cmd, &[]));
        }

        let wsl = Some(system32.join("wsl.exe"))
            .filter(|p| is_executable_file(p))
            .or_else(|| find_on_path(path.as_ref(), "wsl.exe"));
        if let Some(wsl) = wsl {
            for name in wsl_distributions(&wsl) {
                shells.push(ShellInfo {
                    id: format!("{WSL_PREFIX}{name}"),
                    label: format!("WSL: {name}"),
                    program: wsl.clone(),
                    args: vec!["-d".to_owned(), name],
                });
            }
        }

        let git_bash = [
            program_files
                .as_ref()
                .map(|dir| dir.join(r"Git\bin\bash.exe")),
            local_app_data
                .as_ref()
                .map(|dir| dir.join(r"Programs\Git\bin\bash.exe")),
        ]
        .into_iter()
        .flatten()
        .find(|p| is_executable_file(p));
        if let Some(git_bash) = git_bash {
            shells.push(info(GIT_BASH, "Git Bash", git_bash, &["--login", "-i"]));
        }

        shells
    }

    /// PowerShell 7 when it is installed, else Windows PowerShell, else the
    /// first detected shell (owner's stage 2b step 0.5).
    pub(super) fn default_id(shells: &[ShellInfo]) -> Option<String> {
        let by_id = |id: &str| shells.iter().find(|shell| shell.id == id);
        by_id(PWSH)
            .or_else(|| by_id(POWERSHELL))
            .or_else(|| shells.first())
            .map(|shell| shell.id.clone())
    }

    pub(super) fn custom_args(_path: &Path) -> Vec<String> {
        Vec::new()
    }

    fn info(id: &str, label: &str, program: PathBuf, args: &[&str]) -> ShellInfo {
        ShellInfo {
            id: id.to_owned(),
            label: label.to_owned(),
            program,
            args: args.iter().map(|arg| (*arg).to_owned()).collect(),
        }
    }

    /// The first `PATH` entry that holds `file` (an absolute path).
    fn find_on_path(path: Option<&OsString>, file: &str) -> Option<PathBuf> {
        std::env::split_paths(path?)
            .filter(|dir| dir.is_absolute())
            .map(|dir| dir.join(file))
            .find(|candidate| is_executable_file(candidate))
    }

    /// The installed WSL distributions; empty on any failure or timeout.
    /// The listing runs in a job object: a timeout ends `wsl.exe` and the
    /// second `wsl.exe` it starts (D100, item 5).
    #[cfg(windows)]
    fn wsl_distributions(wsl: &Path) -> Vec<String> {
        let mut command = std::process::Command::new(wsl);
        command.args(["--list", "--quiet"]);
        match crate::windows::run_in_job(command, WSL_TIMEOUT) {
            Ok(output) if output.success => parse_wsl_list(&output.stdout),
            Ok(_) => Vec::new(),
            Err(crate::windows::JobError::Setup(error)) => {
                tracing::debug!(kind = ?error.kind(), "the WSL listing did not start");
                Vec::new()
            }
            Err(crate::windows::JobError::TimedOut) => {
                tracing::debug!("the WSL listing timed out and was ended");
                Vec::new()
            }
        }
    }

    /// Parses the output of `wsl.exe --list --quiet`: UTF-16LE (with or
    /// without a byte order mark) or UTF-8 (`WSL_UTF8=1`), one distribution
    /// per line. Empty lines, `docker-desktop*` entries and anything that is
    /// not a valid distribution name (messages) are skipped.
    pub(super) fn parse_wsl_list(output: &[u8]) -> Vec<String> {
        let text = decode_wsl_output(output);
        let mut names: Vec<String> = Vec::new();
        for line in text.lines() {
            let name =
                line.trim_matches(|c: char| c.is_whitespace() || c == '\u{feff}' || c == '\0');
            if name.is_empty()
                || name.to_ascii_lowercase().starts_with("docker-desktop")
                || !is_distribution_name(name)
                || names.iter().any(|known| known.eq_ignore_ascii_case(name))
            {
                continue;
            }
            names.push(name.to_owned());
        }
        names
    }

    fn decode_wsl_output(output: &[u8]) -> String {
        let utf16 = output.starts_with(&[0xff, 0xfe])
            || (output.len() >= 2 && output.len().is_multiple_of(2) && {
                let pairs = output.len() / 2;
                let zero_high = output
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .filter(|pair| pair[1] == 0)
                    .count();
                zero_high * 2 > pairs
            });
        if utf16 {
            let units: Vec<u16> = output
                .as_chunks::<2>()
                .0
                .iter()
                .map(|pair| u16::from_le_bytes(*pair))
                .collect();
            String::from_utf16_lossy(&units)
        } else {
            String::from_utf8_lossy(output).into_owned()
        }
    }

    /// WSL distribution names: letters, digits, `.`, `-` and `_`.
    fn is_distribution_name(name: &str) -> bool {
        name.len() <= 64
            && name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
    }
}

/// macOS and Linux detection.
#[cfg_attr(windows, allow(dead_code))]
mod unix {
    use std::path::{Path, PathBuf};

    use super::{ShellInfo, file_label, is_executable_file};

    /// The fallback when nothing else is found.
    const FALLBACK: &str = "/bin/sh";

    /// Shells that take `-l` to start as a login shell.
    const LOGIN_SHELLS: &[&str] = &[
        "bash", "zsh", "fish", "sh", "dash", "ksh", "ksh93", "mksh", "oksh", "loksh", "pdksh",
        "ash", "yash",
    ];

    #[cfg(not(windows))]
    pub(super) fn detect() -> Vec<ShellInfo> {
        let listed = std::fs::read_to_string("/etc/shells").unwrap_or_default();
        let login = std::env::var("SHELL").ok();
        build(parse_etc_shells(&listed), login.as_deref())
    }

    /// The detected shells from the `/etc/shells` entries and `$SHELL`.
    pub(super) fn build(listed: Vec<String>, login: Option<&str>) -> Vec<ShellInfo> {
        let mut candidates = listed;
        if let Some(login) = login.filter(|l| l.starts_with('/')) {
            candidates.push(login.to_owned());
        }
        let mut shells = collect(candidates);
        if shells.is_empty() {
            shells = collect(vec![FALLBACK.to_owned()]);
        }
        shells
    }

    fn collect(candidates: Vec<String>) -> Vec<ShellInfo> {
        let mut shells = Vec::new();
        let mut seen: Vec<PathBuf> = Vec::new();
        for candidate in candidates {
            let program = PathBuf::from(&candidate);
            if !program.is_absolute() || !is_executable_file(&program) {
                continue;
            }
            let real = std::fs::canonicalize(&program).unwrap_or_else(|_| program.clone());
            if seen.contains(&real) {
                continue;
            }
            let Some(label) = file_label(&program) else {
                continue;
            };
            seen.push(real);
            shells.push(ShellInfo {
                args: login_args(&program),
                id: candidate,
                label,
                program,
            });
        }
        shells
    }

    /// The paths listed in `/etc/shells`, in order (comments and blank lines
    /// skipped, only absolute paths).
    pub(super) fn parse_etc_shells(text: &str) -> Vec<String> {
        text.lines()
            .map(|line| line.split('#').next().unwrap_or("").trim())
            .filter(|line| line.starts_with('/'))
            .map(str::to_owned)
            .collect()
    }

    pub(super) fn default_id(shells: &[ShellInfo]) -> Option<String> {
        let login = std::env::var("SHELL").ok();
        default_for(shells, login.as_deref())
    }

    /// `$SHELL` when it is detected (same id or same real path), else the
    /// first entry.
    pub(super) fn default_for(shells: &[ShellInfo], login: Option<&str>) -> Option<String> {
        let by_login = login.and_then(|login| {
            shells.iter().find(|shell| shell.id == login).or_else(|| {
                let real = std::fs::canonicalize(login).ok()?;
                shells.iter().find(|shell| {
                    std::fs::canonicalize(&shell.program).ok().as_ref() == Some(&real)
                })
            })
        });
        by_login
            .or_else(|| shells.first())
            .map(|shell| shell.id.clone())
    }

    pub(super) fn custom_args(path: &Path) -> Vec<String> {
        login_args(path)
    }

    /// `-l` for bash, zsh, fish and the sh family; nothing for other shells.
    pub(super) fn login_args(program: &Path) -> Vec<String> {
        let name = program
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("");
        if LOGIN_SHELLS.contains(&name) {
            vec!["-l".to_owned()]
        } else {
            Vec::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shell(id: &str, program: &Path) -> ShellInfo {
        ShellInfo {
            id: id.to_owned(),
            label: format!("label {id}"),
            program: program.to_path_buf(),
            args: vec!["-x".to_owned()],
        }
    }

    fn sample() -> (PathBuf, Vec<ShellInfo>) {
        let exe = std::env::current_exe().expect("current exe");
        let first = if cfg!(windows) {
            "powershell"
        } else {
            "/bin/sh"
        };
        let shells = vec![
            shell(first, &exe),
            shell("second", &exe),
            shell("third", &exe),
        ];
        (exe, shells)
    }

    #[test]
    fn requested_id_wins_over_the_setting() {
        let (_, shells) = sample();
        let spec = resolve_shell(Some("third"), Some("second"), &shells).expect("resolved");
        assert_eq!(spec.id.as_deref(), Some("third"));
        assert_eq!(spec.label, "label third");
        assert_eq!(spec.args, vec!["-x".to_owned()]);
    }

    #[test]
    fn unknown_requested_id_is_no_shell() {
        let (_, shells) = sample();
        assert_eq!(
            resolve_shell(Some("missing"), Some("second"), &shells),
            Err(LocalError::NoShell)
        );
        assert_eq!(
            resolve_shell(Some(""), None, &shells),
            Err(LocalError::NoShell)
        );
    }

    #[test]
    fn setting_with_a_detected_id() {
        let (_, shells) = sample();
        let spec = resolve_shell(None, Some("second"), &shells).expect("resolved");
        assert_eq!(spec.id.as_deref(), Some("second"));
    }

    #[test]
    fn setting_with_a_custom_path() {
        let (exe, shells) = sample();
        let setting = exe.to_str().expect("utf-8 test path");
        let spec = resolve_shell(None, Some(setting), &shells).expect("resolved");
        assert_eq!(spec.id, None);
        assert_eq!(spec.program, exe);
        assert_eq!(
            Some(spec.label.as_str()),
            exe.file_name().and_then(|n| n.to_str())
        );
    }

    #[test]
    fn invalid_custom_paths() {
        let (exe, shells) = sample();
        let missing = std::env::temp_dir().join("shellpp-no-such-shell-for-tests");
        let directory = std::env::temp_dir();
        let relative = exe.file_name().and_then(|n| n.to_str()).expect("file name");
        for setting in [
            missing.to_str().expect("utf-8"),
            directory.to_str().expect("utf-8"),
            relative,
            "unknown-id",
        ] {
            assert_eq!(
                resolve_shell(None, Some(setting), &shells),
                Err(LocalError::InvalidShell),
                "setting kind: {}",
                if setting == relative {
                    "relative"
                } else {
                    "other"
                }
            );
        }
    }

    #[test]
    fn fallback_to_the_default() {
        let (_, shells) = sample();
        let expected = default_shell_id(&shells);
        assert!(expected.is_some());
        for setting in [None, Some(""), Some("   ")] {
            let spec = resolve_shell(None, setting, &shells).expect("resolved");
            assert_eq!(spec.id, expected);
        }
    }

    #[test]
    fn nothing_detected_is_no_shell() {
        assert_eq!(default_shell_id(&[]), None);
        assert_eq!(resolve_shell(None, None, &[]), Err(LocalError::NoShell));
    }

    #[test]
    fn windows_default_prefers_powershell_7_then_windows_powershell() {
        let exe = std::env::current_exe().expect("current exe");
        // PowerShell 7 wins whenever it is installed, wherever it is listed.
        let shells = vec![
            shell("powershell", &exe),
            shell("cmd", &exe),
            shell("pwsh", &exe),
        ];
        assert_eq!(windows::default_id(&shells).as_deref(), Some("pwsh"));
        // Without it, Windows PowerShell.
        let shells = vec![shell("cmd", &exe), shell("powershell", &exe)];
        assert_eq!(windows::default_id(&shells).as_deref(), Some("powershell"));
        let shells = vec![shell("cmd", &exe), shell("git-bash", &exe)];
        assert_eq!(windows::default_id(&shells).as_deref(), Some("cmd"));
        assert_eq!(windows::default_id(&[]), None);
    }

    fn utf16le(text: &str, bom: bool) -> Vec<u8> {
        let mut bytes = Vec::new();
        if bom {
            bytes.extend_from_slice(&[0xff, 0xfe]);
        }
        for unit in text.encode_utf16() {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        bytes
    }

    #[test]
    fn wsl_list_parsing() {
        let listing = "Ubuntu-22.04\r\n\r\ndocker-desktop\r\ndocker-desktop-data\r\nDebian\r\n\0";
        let expected = vec!["Ubuntu-22.04".to_owned(), "Debian".to_owned()];
        assert_eq!(windows::parse_wsl_list(&utf16le(listing, false)), expected);
        assert_eq!(windows::parse_wsl_list(&utf16le(listing, true)), expected);
        assert_eq!(windows::parse_wsl_list(listing.as_bytes()), expected);
    }

    #[test]
    fn wsl_list_skips_messages_and_duplicates() {
        let listing = "Windows Subsystem for Linux has no installed distributions.\r\n\
                       Use 'wsl.exe --install' to install.\r\nUbuntu\r\nubuntu\r\nbad;name\r\n";
        assert_eq!(
            windows::parse_wsl_list(&utf16le(listing, false)),
            vec!["Ubuntu".to_owned()]
        );
        assert!(windows::parse_wsl_list(&[]).is_empty());
    }

    #[test]
    fn etc_shells_parsing() {
        let text = "# /etc/shells: valid login shells\n/bin/sh\n\n  /bin/bash  \n\
                    /usr/bin/zsh # comment\nrelative/shell\n#/bin/commented\n";
        assert_eq!(
            unix::parse_etc_shells(text),
            vec![
                "/bin/sh".to_owned(),
                "/bin/bash".to_owned(),
                "/usr/bin/zsh".to_owned()
            ]
        );
    }

    #[test]
    fn login_arguments() {
        for name in [
            "/bin/bash",
            "/usr/bin/zsh",
            "/usr/local/bin/fish",
            "/bin/sh",
            "/bin/dash",
        ] {
            assert_eq!(
                unix::login_args(Path::new(name)),
                vec!["-l".to_owned()],
                "{name}"
            );
        }
        for name in ["/bin/tcsh", "/usr/bin/nu", "/usr/bin/xonsh"] {
            assert!(unix::login_args(Path::new(name)).is_empty(), "{name}");
        }
    }

    #[test]
    fn unix_build_and_default() {
        // The rules are path-agnostic apart from `$SHELL` needing a leading
        // `/`, so the current test binary stands in for a shell everywhere.
        let exe = std::env::current_exe().expect("current exe");
        let exe_str = exe.to_str().expect("utf-8").to_owned();
        let missing = std::env::temp_dir().join("shellpp-no-such-shell-for-tests");
        let missing = missing.to_str().expect("utf-8").to_owned();
        let shells = unix::build(
            vec![missing.clone(), exe_str.clone(), exe_str.clone()],
            None,
        );
        assert_eq!(shells.len(), 1, "missing skipped, duplicates merged");
        assert_eq!(shells[0].id, exe_str);
        assert_eq!(shells[0].program, exe);
        assert_eq!(
            unix::default_for(&shells, Some(&exe_str)),
            Some(exe_str.clone())
        );
        assert_eq!(
            unix::default_for(&shells, Some(&missing)),
            Some(exe_str.clone())
        );
        assert_eq!(unix::default_for(&shells, None), Some(exe_str));
        assert_eq!(unix::default_for(&[], None), None);
        if cfg!(unix) {
            let fallback = unix::build(Vec::new(), None);
            assert_eq!(fallback.len(), 1);
            assert_eq!(fallback[0].id, "/bin/sh");
            assert_eq!(fallback[0].args, vec!["-l".to_owned()]);
        }
    }
}
