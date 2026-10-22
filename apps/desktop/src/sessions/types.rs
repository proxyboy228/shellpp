//! The public types of the session layer: what the UI (session tabs, the
//! connection screen, dialogs and overlays) reads and sends.
//!
//! The UI renders a session from its latest [`SessionView`]; every change
//! arrives as one [`SessionEvent::Changed`] with the whole view, so the UI
//! never has to merge partial updates. Server texts in prompts are
//! untrusted plain text.

use std::time::Instant;

use secrecy::SecretString;
use shellpp_core::service::dto::{Backspace, HostOs, RecordRef};
use shellpp_ssh::FailureKind;

pub use shellpp_core::service::dto::{
    ConnectStep, HostKeyInfo, SessionKind, SessionState, SessionTarget,
};

use super::strings;

/// Identity of a session in this process (never reused).
pub type SessionId = u64;

/// One change of a session: its whole current view.
#[derive(Clone, Debug)]
pub enum SessionEvent {
    /// The view changed (state, step, prompt, error, exit status, reconnect
    /// countdown, title, OS). When `view.state` is [`SessionState::Closed`]
    /// the session is gone and no event follows.
    Changed(SessionView),
}

impl SessionEvent {
    pub fn view(&self) -> &SessionView {
        match self {
            Self::Changed(view) => view,
        }
    }
}

/// Everything the UI shows about one session (UI 4.1, 4.6, 4.8, 10.2).
///
/// | `state` | What the tab shows |
/// |---|---|
/// | `Connecting` | the connection screen with `step`; `prompt` when a form or a host key dialog is needed; Cancel |
/// | `Connected` | the terminal |
/// | `Ended` | the terminal and "Session ended with code N" (`exit_status`, `exit_signal`) with Reconnect / Close |
/// | `Dropped` | the terminal and the reason (`error`) with Reconnect / Close; with `reconnect` the countdown and Cancel |
/// | `Failed` | the failure screen: the reason (`error`, `host_key` when revoked) with Retry / Close |
/// | `Closed` | nothing: the tab closes (neighbor rule of 4.6) |
#[derive(Clone)]
pub struct SessionView {
    pub id: SessionId,
    pub kind: SessionKind,
    pub state: SessionState,
    /// The current step while connecting.
    pub step: Option<ConnectStep>,
    /// Tab title: the host label or address; the shell name for a local terminal.
    pub title: String,
    /// The saved host (its OS icon and name); `None` for quick connects and local terminals.
    pub host: Option<RecordRef>,
    pub address: Option<String>,
    pub port: Option<u16>,
    /// The username in use once known.
    pub username: Option<String>,
    /// The host's detected OS (tab icon); `None` shows the server icon.
    pub os: Option<HostOs>,
    /// Built-in terminal theme slug (`graphite`, `paper`).
    pub theme: String,
    /// What Backspace sends (host setting).
    pub backspace: Backspace,
    /// A question waiting for the user.
    pub prompt: Option<SessionPrompt>,
    /// Why the session failed or dropped.
    pub error: Option<SessionError>,
    /// The presented key when `error` is [`SessionError::HostKeyRevoked`].
    pub host_key: Option<HostKeyInfo>,
    /// Where on the route `error` happened, when not on the host itself
    /// (stage 5: a jump host or the proxy).
    pub error_place: Option<ErrorPlace>,
    /// `Ended`: the exit status (255 when the shell was ended by a signal or
    /// its code is unknown).
    pub exit_status: Option<u32>,
    /// `Ended` by a signal: its name (for example `KILL`).
    pub exit_signal: Option<String>,
    /// `Dropped` with automatic reconnection on.
    pub reconnect: Option<ReconnectView>,
    /// A local terminal's shell, once resolved (`terminal.local_shell` or
    /// the detected default when none was requested); `None` for SSH and
    /// for a custom shell path. Duplicate, a new split pane and Restore
    /// open the same shell with it (UI 4.4).
    pub shell_id: Option<String>,
    /// The startup snippet of the last login waits for the values of its
    /// variables (UI 3.1, 9.3; stage 5): the pane asks them and sends the
    /// filled script.
    pub startup_ask: Option<StartupAsk>,
    /// Increases with every change.
    pub rev: u64,
}

/// A startup snippet whose variables need values; `login` counts the
/// logins of the session, so each is asked once.
#[derive(Clone, PartialEq, Eq)]
pub struct StartupAsk {
    pub login: u64,
    pub script: String,
    pub variables: Vec<String>,
}

impl std::fmt::Debug for StartupAsk {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StartupAsk")
            .field("login", &self.login)
            .field("variables", &self.variables.len())
            .finish_non_exhaustive()
    }
}

impl std::fmt::Debug for SessionView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionView")
            .field("id", &self.id)
            .field("kind", &self.kind)
            .field("state", &self.state)
            .field("step", &self.step)
            .field("prompt", &self.prompt.as_ref().map(|p| p.kind.name()))
            .field("error", &self.error)
            .field("exit_status", &self.exit_status)
            .field("reconnect", &self.reconnect)
            .field("rev", &self.rev)
            .finish_non_exhaustive()
    }
}

impl SessionView {
    /// The text of the overlay or failure screen: "Session ended with code
    /// N", the drop or failure reason. `None` while connecting or connected.
    pub fn message(&self) -> Option<String> {
        match self.state {
            SessionState::Ended => Some(match &self.exit_signal {
                Some(signal) => strings::ended_by_signal(signal),
                None => strings::ended_with_code(self.exit_status.unwrap_or(255)),
            }),
            SessionState::Dropped | SessionState::Failed => Some(
                self.error
                    .unwrap_or(SessionError::ClosedByServer)
                    .message()
                    .to_owned(),
            ),
            SessionState::Connecting | SessionState::Connected | SessionState::Closed => None,
        }
    }

    /// `true` while the session counts for the exit confirmation (UI 4.6):
    /// connecting, connected, or dropped with automatic attempts scheduled.
    pub fn is_active(&self) -> bool {
        match self.state {
            SessionState::Connecting | SessionState::Connected => true,
            SessionState::Dropped => self.reconnect.is_some(),
            SessionState::Ended | SessionState::Failed | SessionState::Closed => false,
        }
    }
}

/// Automatic reconnection of a dropped session (UI 4.6).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReconnectView {
    /// 1-based number of the next (or running) attempt.
    pub attempt: u32,
    /// When the next attempt starts; `None` while one runs or while waiting
    /// for unlock.
    pub next_at: Option<Instant>,
    /// The app is locked: attempts wait for unlock (UI 13.3).
    pub waiting_for_unlock: bool,
    /// An attempt is running now.
    pub in_progress: bool,
}

impl ReconnectView {
    /// Whole seconds until the next attempt (rounded up), for the countdown.
    pub fn seconds_left(&self, now: Instant) -> Option<u64> {
        let next = self.next_at?;
        let left = next.saturating_duration_since(now);
        Some(left.as_millis().div_ceil(1000) as u64)
    }
}

/// A question of a session; answer it with [`super::SessionHandle::reply`]
/// and this `id`. A newer view replaces or removes it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionPrompt {
    pub id: u64,
    pub kind: PromptKind,
}

/// What is asked (UI 4.8, 10.2). Server texts are untrusted.
#[derive(Clone, PartialEq, Eq)]
pub enum PromptKind {
    /// First connection (TOFU): "Accept and save" (default), "Connect once", "Cancel".
    HostKeyUnknown {
        key: HostKeyInfo,
    },
    /// The saved key differs: a blocking warning in the danger color with
    /// both fingerprints; only "Replace saved key" continues.
    HostKeyChanged {
        key: HostKeyInfo,
        old_key_type: String,
        old_fingerprint: String,
    },
    Username,
    /// `error`: the previous password was refused.
    Password {
        username: String,
        error: bool,
        save: SaveOption,
    },
    /// The passphrase of a key: a saved key record ([`SaveOption::Vault`])
    /// or a key file (`~/.ssh/id_*`, [`SaveOption::ThisSession`]).
    Passphrase {
        key_label: String,
        error: bool,
        save: SaveOption,
    },
    /// One keyboard-interactive question (2FA codes); `echo == false` masks
    /// the answer. `index` of `count` questions in this round.
    KeyboardInteractive {
        name: String,
        instruction: String,
        prompt: String,
        echo: bool,
        index: u32,
        count: u32,
        error: bool,
    },
    /// Agent forwarding with "Ask before every signature" (D60): the server
    /// asks the forwarded agent to sign with the key of `fingerprint`
    /// (`SHA256:`). [`PromptReply::Allow`] signs; anything else refuses.
    AgentSignature {
        fingerprint: String,
    },
}

impl std::fmt::Debug for PromptKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

impl PromptKind {
    /// The kind's name, for logs and tests.
    pub fn name(&self) -> &'static str {
        match self {
            Self::HostKeyUnknown { .. } => "host_key_unknown",
            Self::HostKeyChanged { .. } => "host_key_changed",
            Self::Username => "username",
            Self::Password { .. } => "password",
            Self::Passphrase { .. } => "passphrase",
            Self::KeyboardInteractive { .. } => "keyboard_interactive",
            Self::AgentSignature { .. } => "agent_signature",
        }
    }

    /// The answer must be masked.
    pub fn is_secret(&self) -> bool {
        match self {
            Self::Password { .. } | Self::Passphrase { .. } => true,
            Self::KeyboardInteractive { echo, .. } => !echo,
            Self::HostKeyUnknown { .. }
            | Self::HostKeyChanged { .. }
            | Self::Username
            | Self::AgentSignature { .. } => false,
        }
    }
}

/// The checkbox under a password or passphrase field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SaveOption {
    /// No checkbox (quick connect without a saved host).
    None,
    /// "Save": stored encrypted in the host or key record (UI 4.8).
    Vault,
    /// "Remember for this session": kept in memory until the app locks or
    /// quits (key files have no record to save to).
    ThisSession,
}

/// The user's answer to a [`SessionPrompt`].
pub enum PromptReply {
    /// Host key dialogs.
    HostKey(HostKeyChoice),
    /// Username, password, passphrase or keyboard-interactive answer; `save`
    /// is the checkbox ([`SaveOption`]).
    Text { value: SecretString, save: bool },
    /// "Cancel" of any prompt: the attempt fails with `Cancelled`.
    Cancel,
    /// "Allow" of a signature question ([`PromptKind::AgentSignature`]).
    Allow,
}

impl std::fmt::Debug for PromptReply {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::HostKey(choice) => f.debug_tuple("HostKey").field(choice).finish(),
            Self::Text { save, .. } => f
                .debug_struct("Text")
                .field("save", save)
                .finish_non_exhaustive(),
            Self::Cancel => f.write_str("Cancel"),
            Self::Allow => f.write_str("Allow"),
        }
    }
}

/// Buttons of the host key dialogs (UI 10.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostKeyChoice {
    /// Unknown key: save it in known hosts and connect.
    AcceptAndSave,
    /// Unknown key: connect without saving (this session only).
    ConnectOnce,
    /// Changed key: replace the saved key and connect.
    Replace,
    Cancel,
}

/// Where on a route a failure happened (the host itself has none).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ErrorPlace {
    /// The jump host at `index` (0-based) of `count`, with its title.
    JumpHost {
        index: usize,
        count: usize,
        title: String,
    },
    /// The proxy of the first connection.
    Proxy,
    /// The hop at `index` (0-based) of the chain is missing or repeated.
    Chain { index: Option<usize> },
}

/// Why a session failed or dropped. [`SessionError::message`] is the text
/// shown in the tab.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionError {
    /// The connection could not be established, or authentication failed.
    Ssh(FailureKind),
    /// The connection was lost (network error).
    ConnectionLost,
    /// The server stopped answering keepalives.
    KeepaliveTimeout,
    /// The server closed the connection without an exit status.
    ClosedByServer,
    /// The presented host key is revoked: blocked, no way to continue (10.2).
    HostKeyRevoked,
    /// Cancelled by the user (Cancel, a cancelled prompt or dialog).
    Cancelled,
    /// The app is locked.
    Locked,
    /// The saved host no longer exists.
    HostMissing,
    /// The quick-connect address, port or username is not valid.
    InvalidTarget,
    /// A jump host of the route is missing or named twice (data-model 8.3):
    /// the host is never connected directly instead.
    RouteInvalid,
    /// The local shell could not be started.
    ShellUnavailable,
    /// Something else went wrong (storage error).
    Internal,
}

impl SessionError {
    /// The user-facing text.
    pub fn message(self) -> &'static str {
        strings::error_message(self)
    }

    /// Failures an automatic reconnect attempt retries (UI 4.6): the server
    /// could not be reached or the connection was lost.
    pub fn is_transport(self) -> bool {
        matches!(
            self,
            Self::Ssh(
                FailureKind::NotFound
                    | FailureKind::Refused
                    | FailureKind::Timeout
                    | FailureKind::Network
                    | FailureKind::Disconnected
            ) | Self::ConnectionLost
                | Self::KeepaliveTimeout
                | Self::ClosedByServer
        )
    }

    /// The error of an SSH failure.
    pub fn from_failure(kind: FailureKind) -> Self {
        match kind {
            FailureKind::Cancelled => Self::Cancelled,
            kind => Self::Ssh(kind),
        }
    }

    /// The reason of a drop of a running shell.
    pub fn from_drop(kind: FailureKind) -> Self {
        match kind {
            FailureKind::Network => Self::ConnectionLost,
            FailureKind::Timeout => Self::KeepaliveTimeout,
            _ => Self::ClosedByServer,
        }
    }
}

/// What to open (UI 2.1, 2.4, 9).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OpenRequest {
    /// SSH to a saved host or a quick-connect address.
    Ssh(SessionTarget),
    /// A local terminal; `shell_id` from the detected shells, `None` = the
    /// `terminal.local_shell` setting or the default (PowerShell on Windows).
    Local { shell_id: Option<String> },
    /// Telnet to a saved host's Telnet block (D87).
    Telnet {
        vault_id: uuid::Uuid,
        record_id: uuid::Uuid,
    },
}

/// Why [`super::SessionHandle::write`] took nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum WriteError {
    /// The session is not connected (input is dropped).
    #[error("the session is not connected")]
    NotConnected,
    /// Too much input waits for the remote side; try again later.
    #[error("too much input is waiting")]
    Busy,
}
