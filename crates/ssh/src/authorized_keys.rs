//! Export to host (UI 7.3 block 6; night plan, stage 3 item 9): appends a
//! public key to `~/.ssh/authorized_keys` on the server, like `ssh-copy-id`,
//! through one exec channel of an open [`Connection`] (never the shell).
//!
//! The command runs under `sh` whatever the login shell is (`exec sh -c`),
//! with `umask 077`: it creates `~/.ssh` (mode 700) and `authorized_keys`
//! (mode 600) when they are missing and sets those modes when they exist,
//! adds a line break first when the file does not end with one, and appends
//! the key only when no line holds its base64 blob yet (no duplicates, a
//! different comment included). The key line is checked first: an algorithm
//! name, a base64 blob, a comment of plain characters only, so nothing in it
//! can reach the shell as syntax.

use std::time::Duration;

use crate::client::Connection;
use crate::error::{FailureKind, SshError};

/// The exit status of the command when the key was already there.
const ALREADY_PRESENT: u32 = 3;
/// The most a comment keeps (the rest is cut).
const MAX_COMMENT_CHARS: usize = 128;
/// The time the command may take.
const TIMEOUT: Duration = Duration::from_secs(30);

/// What the export did on one host.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExportOutcome {
    /// The key was appended.
    Added,
    /// A line with the same key was already there.
    AlreadyPresent,
}

/// Why a key line cannot be exported.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum KeyLineError {
    #[error("not a public key line")]
    Invalid,
}

/// The checked parts of a public key line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyLine {
    pub algorithm: String,
    pub blob: String,
    /// Only `[A-Za-z0-9._@:+-]`; other characters become `_`.
    pub comment: String,
}

impl KeyLine {
    /// Checks `<algorithm> <base64> [comment]`.
    pub fn parse(line: &str) -> Result<Self, KeyLineError> {
        let mut parts = line.split_whitespace();
        let algorithm = parts.next().ok_or(KeyLineError::Invalid)?;
        let blob = parts.next().ok_or(KeyLineError::Invalid)?;
        let algorithm_ok = !algorithm.is_empty()
            && algorithm.len() <= 64
            && algorithm
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | '@'));
        let blob_ok = blob.len() >= 16
            && blob.len() <= 16_384
            && blob
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '/' | '='));
        if !algorithm_ok || !blob_ok {
            return Err(KeyLineError::Invalid);
        }
        let comment: String = parts
            .collect::<Vec<_>>()
            .join("_")
            .chars()
            .take(MAX_COMMENT_CHARS)
            .map(|c| {
                if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '@' | ':' | '+' | '-') {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        Ok(Self {
            algorithm: algorithm.to_owned(),
            blob: blob.to_owned(),
            comment,
        })
    }

    /// The line written to the file.
    pub fn line(&self) -> String {
        if self.comment.is_empty() {
            format!("{} {}", self.algorithm, self.blob)
        } else {
            format!("{} {} {}", self.algorithm, self.blob, self.comment)
        }
    }
}

/// The command of the exec channel. Every part of the key line is plain
/// (see [`KeyLine::parse`]), so double quotes inside the single-quoted
/// script are enough.
pub fn command(key: &KeyLine) -> String {
    let script = format!(
        "umask 077; \
         d=\"$HOME/.ssh\"; f=\"$d/authorized_keys\"; \
         mkdir -p \"$d\" || exit 10; chmod 700 \"$d\" || exit 11; \
         [ -f \"$f\" ] || : > \"$f\" || exit 12; chmod 600 \"$f\" || exit 13; \
         if grep -v \"^[[:space:]]*#\" \"$f\" | grep -qF \"{blob}\"; then exit {present}; fi; \
         if [ -s \"$f\" ] && [ -n \"$(tail -c 1 \"$f\")\" ]; then echo >> \"$f\" || exit 14; fi; \
         printf '%s\\n' \"{line}\" >> \"$f\" || exit 14",
        blob = key.blob,
        line = key.line(),
        present = ALREADY_PRESENT,
    );
    // The script holds no single quote except the printf format, which is
    // closed and reopened around it.
    format!("exec sh -c '{}'", script.replace('\'', "'\\''"))
}

/// Appends `public_key` (one `authorized_keys` line) on the server of
/// `connection`. A refused channel, a timeout or another exit status is an
/// error (`FailureKind::ChannelFailed`).
pub async fn export_public_key(
    connection: &Connection,
    public_key: &str,
) -> Result<ExportOutcome, SshError> {
    let key = KeyLine::parse(public_key).map_err(|_| SshError::new(FailureKind::ChannelFailed))?;
    let output = connection.exec(&command(&key), TIMEOUT, 4096).await?;
    match output.exit_status {
        Some(0) => Ok(ExportOutcome::Added),
        Some(ALREADY_PRESENT) => Ok(ExportOutcome::AlreadyPresent),
        _ => Err(SshError::new(FailureKind::ChannelFailed)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BLOB: &str = "AAAAC3NzaC1lZDI1NTE5AAAAIIE5EuMYZWQuWmjd0Ss6T1txVGDAn+d/sJdJ8BnaeW9a";

    #[test]
    fn key_lines_are_checked_and_comments_made_plain() {
        let key = KeyLine::parse(&format!("ssh-ed25519 {BLOB} me@laptop")).unwrap();
        assert_eq!(key.line(), format!("ssh-ed25519 {BLOB} me@laptop"));
        let key = KeyLine::parse(&format!("ssh-ed25519 {BLOB} a 'b' $(rm -rf ~) \"c\"")).unwrap();
        assert!(!key.comment.contains(['\'', '"', '$', '(', ')', ' ', '~']));
        assert_eq!(
            KeyLine::parse(&format!("ssh-ed25519 {BLOB}"))
                .unwrap()
                .comment,
            ""
        );
        for bad in [
            "".to_owned(),
            "ssh-ed25519".to_owned(),
            format!("ssh-ed25519 {BLOB}$x"),
            format!("ssh ed25519 {BLOB}"),
            format!("ssh'ed25519 {BLOB}"),
            "ssh-ed25519 AAAA".to_owned(),
        ] {
            assert!(KeyLine::parse(&bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_command_quotes_the_script_once() {
        let key = KeyLine::parse(&format!("ssh-ed25519 {BLOB} test")).unwrap();
        let command = command(&key);
        assert!(command.starts_with("exec sh -c '"));
        assert!(command.ends_with('\''));
        assert!(command.contains(BLOB));
        assert!(command.contains("umask 077"));
        assert!(command.contains("chmod 700"));
        assert!(command.contains("chmod 600"));
        // The only single quotes inside are the escaped printf format.
        let inner = &command["exec sh -c '".len()..command.len() - 1];
        assert_eq!(inner.matches("'\\''").count(), 2);
    }

    /// The script itself, run by `sh` in a temporary home folder: a key
    /// that is only in a comment is added, a key that is there is not.
    #[cfg(unix)]
    #[test]
    fn a_commented_out_copy_does_not_count_as_present() {
        let home = tempfile::tempdir().unwrap();
        let ssh = home.path().join(".ssh");
        std::fs::create_dir_all(&ssh).unwrap();
        let file = ssh.join("authorized_keys");
        std::fs::write(&file, format!("# ssh-ed25519 {BLOB} old\n")).unwrap();
        let key = KeyLine::parse(&format!("ssh-ed25519 {BLOB} test")).unwrap();
        let run = || {
            std::process::Command::new("sh")
                .arg("-c")
                .arg(command(&key))
                .env("HOME", home.path())
                .status()
                .unwrap()
                .code()
        };
        assert_eq!(run(), Some(0), "added");
        assert_eq!(run(), Some(ALREADY_PRESENT as i32), "now present");
        let text = std::fs::read_to_string(&file).unwrap();
        let keys = text
            .lines()
            .filter(|line| line.starts_with("ssh-ed25519"))
            .count();
        assert_eq!(keys, 1, "{text}");
    }
}
