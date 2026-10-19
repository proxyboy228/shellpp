//! Shell++ local terminal (desktop only, decision 5; docs/architecture.md,
//! section 2.4): shell detection per UI 12.4 and a pseudo-terminal session on
//! portable-pty. Serial ports and Telnet join in stage 9.
//!
//! - [`detect_shells`]: Windows: PowerShell, PowerShell 7, cmd, WSL
//!   distributions, Git Bash; macOS and Linux: the shells listed in
//!   `/etc/shells` that exist.
//! - [`resolve_shell`]: the shell for a new local terminal from the
//!   requested id, the `terminal.local_shell` setting (an id or a custom
//!   absolute path) or the default (PowerShell on Windows, the login shell
//!   elsewhere).
//! - [`spawn`]: starts the shell in a PTY; output arrives on a tokio channel
//!   from a reader thread, input and resizes go through [`PtyWriter`]. See
//!   the [`pty`] module for the exit and end-of-file order.
//!
//! Nothing here logs paths, shell output or input.
//!
//! Owner: S3. The public API is the S3/S5 contract written by the lead
//! (stage 2 plan, section 4.3).

#![forbid(unsafe_code)]

pub mod pty;
pub mod shells;

pub use pty::{ExitInfo, LocalSession, PtyConfig, PtyWriter, spawn};
pub use shells::{ShellInfo, ShellSpec, default_shell_id, detect_shells, resolve_shell};

/// Errors of the local terminal. No paths or output in the messages.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum LocalError {
    /// No shell was found (or the requested id is unknown).
    #[error("no shell")]
    NoShell,
    /// The custom shell path is not an absolute path to an existing file.
    #[error("invalid shell path")]
    InvalidShell,
    /// The PTY or the process could not be started.
    #[error("spawn failed")]
    Spawn,
    /// Reading, writing or resizing failed.
    #[error("i/o error")]
    Io,
    /// The session already ended.
    #[error("closed")]
    Closed,
}
