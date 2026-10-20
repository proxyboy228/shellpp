//! Local names of downloaded files (night plan, stage 4, item 8; D46).
//!
//! - [`windows_safe_name`]: a name Windows cannot hold (`:`, `?`, `*`, `<`,
//!   `>`, `|`, `"`, `\`, control characters, a trailing dot or space, a
//!   reserved device name such as `CON` or `com1.txt`) gets a suggestion for
//!   the rename dialog: each such character becomes `_`, a reserved name
//!   gets a leading `_`.
//! - [`long_path`]: on Windows, an absolute path of 260 characters or more
//!   gets the `\\?\` prefix (`\\?\UNC\` for a share), so the file API takes
//!   it.

use std::path::{Path, PathBuf};

/// Device names Windows reserves in any folder, with any extension.
const RESERVED: [&str; 22] = [
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// Paths this long need the `\\?\` prefix on Windows (`MAX_PATH`).
const MAX_PATH: usize = 260;

/// `None` when Windows can hold `name` as it is; else the suggested name.
pub fn windows_safe_name(name: &str) -> Option<String> {
    let mut out: String = name
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') {
                '_'
            } else {
                c
            }
        })
        .collect();
    // A trailing dot or space is dropped by Windows: it becomes `_`.
    let kept = out.trim_end_matches(['.', ' ']).len();
    if kept < out.len() {
        let tail = out.len() - kept;
        out.truncate(kept);
        out.extend(std::iter::repeat_n('_', tail));
    }
    let stem = out.split('.').next().unwrap_or_default().trim_end();
    if RESERVED
        .iter()
        .any(|reserved| stem.eq_ignore_ascii_case(reserved))
    {
        out.insert(0, '_');
    }
    (out != name).then_some(out)
}

/// `path` as the Windows file API takes it when it is long (elsewhere, and
/// for short or relative paths, as it is).
pub fn long_path(path: &Path) -> PathBuf {
    let text = path.to_string_lossy();
    if !cfg!(windows) || text.chars().count() < MAX_PATH || text.starts_with(r"\\?\") {
        return path.to_path_buf();
    }
    if let Some(share) = text.strip_prefix(r"\\") {
        return PathBuf::from(format!(r"\\?\UNC\{share}"));
    }
    if path.is_absolute() {
        return PathBuf::from(format!(r"\\?\{}", text.replace('/', r"\")));
    }
    path.to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_windows_cannot_hold_get_a_suggestion() {
        assert_eq!(windows_safe_name("report.txt"), None);
        assert_eq!(windows_safe_name("Отчёт 😀.txt"), None);
        assert_eq!(
            windows_safe_name("a:b?c*.log").as_deref(),
            Some("a_b_c_.log")
        );
        assert_eq!(windows_safe_name("x<y>|\"z").as_deref(), Some("x_y___z"));
        assert_eq!(windows_safe_name("tab\there").as_deref(), Some("tab_here"));
        assert_eq!(windows_safe_name("name.").as_deref(), Some("name_"));
        assert_eq!(windows_safe_name("name. ").as_deref(), Some("name__"));
        assert_eq!(windows_safe_name("CON").as_deref(), Some("_CON"));
        assert_eq!(windows_safe_name("com1.txt").as_deref(), Some("_com1.txt"));
        assert_eq!(windows_safe_name("console.txt"), None);
        assert_eq!(
            windows_safe_name("back\\slash").as_deref(),
            Some("back_slash")
        );
    }

    #[test]
    fn long_windows_paths_get_the_prefix() {
        let short = Path::new("C:/short/file.txt");
        assert_eq!(long_path(short), short);
        let long = format!("C:\\{}\\file.txt", "d".repeat(300));
        if cfg!(windows) {
            assert_eq!(
                long_path(Path::new(&long)),
                PathBuf::from(format!(r"\\?\{long}"))
            );
            let share = format!(r"\\server\share\{}", "d".repeat(300));
            assert_eq!(
                long_path(Path::new(&share)),
                PathBuf::from(format!(r"\\?\UNC\server\share\{}", "d".repeat(300)))
            );
            let prefixed = PathBuf::from(format!(r"\\?\{long}"));
            assert_eq!(long_path(&prefixed), prefixed);
        } else {
            assert_eq!(long_path(Path::new(&long)), Path::new(&long));
        }
    }
}
