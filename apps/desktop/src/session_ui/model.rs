//! What a session tab shows, as plain data (UI 4.1, 4.6, 4.8, 10.2), and the
//! commands the tab sends back to the session layer ([`SessionControl`]).
//!
//! The session layer (`crate::sessions`: Tokio runtime, russh, the PTY)
//! reports to the tab through [`super::bridge`], which turns every view of a
//! session into these values; tests drive the tab with them directly. Texts
//! from the server (prompt names, instructions, questions) are untrusted and
//! shown as plain text; nothing here is logged, and the `Debug` output of
//! every type that carries a secret, an address or a username leaves it out.

use std::time::Instant;

use gpui::{App, SharedString};
use secrecy::SecretString;

use super::strings;

/// A step of the connection screen (UI 4.8).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConnectStep {
    Resolving,
    Connecting,
    HostKey,
    Authenticating,
    OpeningSession,
}

impl ConnectStep {
    /// The steps in order, as the connection screen lists them.
    pub const ALL: [ConnectStep; 5] = [
        ConnectStep::Resolving,
        ConnectStep::Connecting,
        ConnectStep::HostKey,
        ConnectStep::Authenticating,
        ConnectStep::OpeningSession,
    ];

    pub fn label(self) -> &'static str {
        match self {
            ConnectStep::Resolving => strings::STEP_RESOLVING,
            ConnectStep::Connecting => strings::STEP_CONNECTING,
            ConnectStep::HostKey => strings::STEP_HOST_KEY,
            ConnectStep::Authenticating => strings::STEP_AUTHENTICATING,
            ConnectStep::OpeningSession => strings::STEP_OPENING_SESSION,
        }
    }

    /// Stable element-id part (`connecting-step-<slug>`).
    pub fn slug(self) -> &'static str {
        match self {
            ConnectStep::Resolving => "resolving",
            ConnectStep::Connecting => "connecting",
            ConnectStep::HostKey => "host-key",
            ConnectStep::Authenticating => "authenticating",
            ConnectStep::OpeningSession => "opening-session",
        }
    }
}

/// Why a connection failed or dropped (UI 4.6, 4.8).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Failure {
    /// The name did not resolve (also "no network" for a name).
    NotFound,
    Refused,
    Timeout,
    /// No route, unreachable network, reset during the handshake.
    Network,
    /// The host key was not accepted.
    HostKeyRejected,
    /// The host key is in Known Hosts as revoked (UI 10.2).
    HostKeyRevoked,
    AuthFailed,
    /// The user cancelled the attempt.
    Cancelled,
    Protocol,
    ChannelFailed,
    /// The server closed the connection during the handshake.
    Disconnected,
    /// The running connection was lost (network error).
    ConnectionLost,
    /// The keepalive went unanswered.
    KeepaliveTimeout,
    /// The server closed the channel without an exit status.
    ClosedByServer,
    KeyUnusable,
    /// The app is locked.
    Locked,
    /// The saved host no longer exists.
    HostMissing,
    /// The quick-connect address, port or username is not valid.
    InvalidTarget,
    /// The host has jump hosts, which arrive with stage 5.
    JumpHosts,
    /// The local shell could not be started.
    LocalShell,
    /// Anything else (a storage error, no session layer).
    Other,
}

impl Failure {
    /// The text shown in the tab.
    pub fn text(self) -> &'static str {
        match self {
            Failure::NotFound => strings::REASON_NOT_FOUND,
            Failure::Refused => strings::REASON_REFUSED,
            Failure::Timeout => strings::REASON_TIMEOUT,
            Failure::Network => strings::REASON_NETWORK,
            Failure::HostKeyRejected => strings::REASON_HOST_KEY_REJECTED,
            Failure::HostKeyRevoked => strings::REASON_HOST_KEY_REVOKED,
            Failure::AuthFailed => strings::REASON_AUTH_FAILED,
            Failure::Cancelled => strings::REASON_CANCELLED,
            Failure::Protocol => strings::REASON_PROTOCOL,
            Failure::ChannelFailed => strings::REASON_CHANNEL_FAILED,
            Failure::Disconnected => strings::REASON_DISCONNECTED,
            Failure::ConnectionLost => strings::REASON_CONNECTION_LOST,
            Failure::KeepaliveTimeout => strings::REASON_KEEPALIVE,
            Failure::ClosedByServer => strings::REASON_CLOSED_BY_SERVER,
            Failure::KeyUnusable => strings::REASON_KEY_UNUSABLE,
            Failure::Locked => strings::REASON_LOCKED,
            Failure::HostMissing => strings::REASON_HOST_MISSING,
            Failure::InvalidTarget => strings::REASON_INVALID_TARGET,
            Failure::JumpHosts => strings::REASON_JUMP_HOSTS,
            Failure::LocalShell => strings::REASON_LOCAL_SHELL,
            Failure::Other => strings::REASON_OTHER,
        }
    }

    /// Whether the failure screen offers "Retry". A revoked key blocks the
    /// host (UI 10.2); a host that is gone or a quick-connect target that is
    /// not valid fails the same way every time, so only "Close" is shown.
    pub fn can_retry(self) -> bool {
        !matches!(
            self,
            Failure::HostKeyRevoked
                | Failure::HostMissing
                | Failure::InvalidTarget
                | Failure::JumpHosts
        )
    }

    /// Stable code for element ids (`failure-<code>`) and tests.
    pub fn code(self) -> &'static str {
        match self {
            Failure::NotFound => "not_found",
            Failure::Refused => "refused",
            Failure::Timeout => "timeout",
            Failure::Network => "network",
            Failure::HostKeyRejected => "host_key_rejected",
            Failure::HostKeyRevoked => "host_key_revoked",
            Failure::AuthFailed => "auth_failed",
            Failure::Cancelled => "cancelled",
            Failure::Protocol => "protocol",
            Failure::ChannelFailed => "channel_failed",
            Failure::Disconnected => "disconnected",
            Failure::ConnectionLost => "connection_lost",
            Failure::KeepaliveTimeout => "keepalive_timeout",
            Failure::ClosedByServer => "closed_by_server",
            Failure::KeyUnusable => "key_unusable",
            Failure::Locked => "locked",
            Failure::HostMissing => "host_missing",
            Failure::InvalidTarget => "invalid_target",
            Failure::JumpHosts => "jump_hosts",
            Failure::LocalShell => "local_shell",
            Failure::Other => "other",
        }
    }

    /// Every reason (tests).
    pub const ALL: [Failure; 21] = [
        Failure::NotFound,
        Failure::Refused,
        Failure::Timeout,
        Failure::Network,
        Failure::HostKeyRejected,
        Failure::HostKeyRevoked,
        Failure::AuthFailed,
        Failure::Cancelled,
        Failure::Protocol,
        Failure::ChannelFailed,
        Failure::Disconnected,
        Failure::ConnectionLost,
        Failure::KeepaliveTimeout,
        Failure::ClosedByServer,
        Failure::KeyUnusable,
        Failure::Locked,
        Failure::HostMissing,
        Failure::InvalidTarget,
        Failure::JumpHosts,
        Failure::LocalShell,
        Failure::Other,
    ];
}

/// How a session ended without a drop (UI 4.6).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExitStatus {
    /// The exit code; 0 is the normal end (the tab closes).
    Code(u32),
    /// The program was killed by a signal (shown like a non-zero code).
    Signal(SharedString),
}

impl ExitStatus {
    /// "Session ended with code N" / "... with signal TERM".
    pub fn text(&self) -> String {
        match self {
            ExitStatus::Code(code) => strings::ended_with_code(*code),
            ExitStatus::Signal(signal) => strings::ended_with_signal(signal),
        }
    }
}

/// The phase of a session tab.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Phase {
    /// The first attempt (or a Retry after a failure) runs: the connection
    /// screen (UI 4.8).
    Connecting(ConnectStep),
    /// The attempt failed: the reason with Retry and Close (UI 4.8).
    /// Automatic reconnection does not apply.
    Failed(Failure),
    /// The shell runs: the terminal (UI 4.1).
    Running,
    /// The session ended with a non-zero status or a signal: the output stays,
    /// "Session ended with code N" with Reconnect and Close; no automatic
    /// attempts (UI 4.6).
    Ended(ExitStatus),
    /// A transport drop: the output stays, the reason with Reconnect and
    /// Close; [`ReconnectInfo`] when automatic attempts run (UI 4.6).
    Dropped(Failure),
    /// A reconnect attempt runs over the kept output.
    Reconnecting(ConnectStep),
    /// Exit status 0 or closed: the tab closes.
    Closed,
}

impl Phase {
    /// The terminal (and its output) is on display under any overlay.
    pub fn shows_terminal(&self) -> bool {
        matches!(
            self,
            Phase::Running | Phase::Ended(_) | Phase::Dropped(_) | Phase::Reconnecting(_)
        )
    }

    /// Stable name for tests.
    pub fn name(&self) -> &'static str {
        match self {
            Phase::Connecting(_) => "connecting",
            Phase::Failed(_) => "failed",
            Phase::Running => "running",
            Phase::Ended(_) => "ended",
            Phase::Dropped(_) => "dropped",
            Phase::Reconnecting(_) => "reconnecting",
            Phase::Closed => "closed",
        }
    }
}

/// Automatic reconnection of a dropped session (UI 4.6), as the session
/// layer schedules it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReconnectInfo {
    /// 1-based number of the next (or running) attempt.
    pub attempt: u32,
    /// When the next attempt starts; `None` while one runs or while waiting
    /// for unlock.
    pub next_at: Option<Instant>,
    /// The app is locked: the attempt waits for unlock (UI 13.3).
    pub waiting_for_unlock: bool,
}

impl ReconnectInfo {
    /// Whole seconds until the next attempt (rounded up).
    pub fn seconds_left(&self, now: Instant) -> Option<u64> {
        let left = self.next_at?.saturating_duration_since(now);
        Some(left.as_millis().div_ceil(1000) as u64)
    }
}

/// The checkbox under a password or passphrase field (UI 4.8; D30).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Keep {
    /// No checkbox (a quick connect without a saved host).
    None,
    /// "Save": stored encrypted in the host or key record.
    Save,
    /// "Remember until Shell++ exits": a key file's passphrase, kept in
    /// memory only (wiped on lock).
    ThisSession,
}

/// A value the session needs from the user (UI 4.8).
#[derive(Clone, PartialEq, Eq)]
pub enum Prompt {
    /// The username, when nothing sets one.
    Username,
    /// The password of `username`. `error`: the previous one was refused.
    Password {
        username: String,
        error: bool,
        keep: Keep,
    },
    /// The passphrase of an encrypted key.
    Passphrase {
        key_label: String,
        error: bool,
        keep: Keep,
    },
    /// One keyboard-interactive question (asked one at a time). `name`,
    /// `instruction` and `prompt` come from the server.
    KeyboardInteractive {
        name: String,
        instruction: String,
        prompt: String,
        echo: bool,
        index: u32,
        count: u32,
        error: bool,
    },
}

impl std::fmt::Debug for Prompt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never the username, key label or server texts.
        match self {
            Prompt::Username => f.write_str("Prompt::Username"),
            Prompt::Password { error, keep, .. } => f
                .debug_struct("Prompt::Password")
                .field("error", error)
                .field("keep", keep)
                .finish_non_exhaustive(),
            Prompt::Passphrase { error, keep, .. } => f
                .debug_struct("Prompt::Passphrase")
                .field("error", error)
                .field("keep", keep)
                .finish_non_exhaustive(),
            Prompt::KeyboardInteractive {
                echo,
                index,
                count,
                error,
                ..
            } => f
                .debug_struct("Prompt::KeyboardInteractive")
                .field("echo", echo)
                .field("index", index)
                .field("count", count)
                .field("error", error)
                .finish_non_exhaustive(),
        }
    }
}

impl Prompt {
    /// The value is masked (passwords, passphrases, non-echoed answers).
    pub fn is_masked(&self) -> bool {
        match self {
            Prompt::Username => false,
            Prompt::Password { .. } | Prompt::Passphrase { .. } => true,
            Prompt::KeyboardInteractive { echo, .. } => !echo,
        }
    }

    /// The previous value was refused.
    pub fn has_error(&self) -> bool {
        match self {
            Prompt::Username => false,
            Prompt::Password { error, .. }
            | Prompt::Passphrase { error, .. }
            | Prompt::KeyboardInteractive { error, .. } => *error,
        }
    }

    /// The checkbox the form offers.
    pub fn keep(&self) -> Keep {
        match self {
            Prompt::Password { keep, .. } | Prompt::Passphrase { keep, .. } => *keep,
            Prompt::Username | Prompt::KeyboardInteractive { .. } => Keep::None,
        }
    }
}

/// The user's answer to a [`Prompt`].
pub enum PromptReply {
    /// The value; `keep` is the checkbox ([`Keep`]), false without one.
    Answer { value: SecretString, keep: bool },
    /// Cancel: the attempt stops (UI 4.8).
    Cancel,
}

impl std::fmt::Debug for PromptReply {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PromptReply::Answer { keep, .. } => f
                .debug_struct("PromptReply::Answer")
                .field("keep", keep)
                .finish_non_exhaustive(),
            PromptReply::Cancel => f.write_str("PromptReply::Cancel"),
        }
    }
}

/// A presented host key (UI 10.2).
#[derive(Clone, PartialEq, Eq)]
pub struct HostKeyInfo {
    /// The address in OpenSSH form: `host`, or `[host]:port` for a port
    /// other than 22.
    pub address: String,
    pub key_type: String,
    /// `SHA256:...`.
    pub fingerprint: String,
}

impl std::fmt::Debug for HostKeyInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HostKeyInfo")
            .field("key_type", &self.key_type)
            .finish_non_exhaustive()
    }
}

impl HostKeyInfo {
    /// `host` for port 22, else `[host]:port` (UI 10.1).
    pub fn openssh_address(host: &str, port: u16) -> String {
        if port == 22 {
            host.to_owned()
        } else {
            format!("[{host}]:{port}")
        }
    }
}

/// The host key dialogs of UI 10.2 that wait for the user. A revoked key
/// is not a question: the attempt fails ([`Failure::HostKeyRevoked`]) and the
/// failure screen shows the warning.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HostKeyAsk {
    /// First connection (TOFU): "Accept and save" (default), "Connect once",
    /// "Cancel".
    Unknown(HostKeyInfo),
    /// The key changed: blocked, a danger warning with both fingerprints;
    /// only "Replace saved key" continues.
    Changed {
        key: HostKeyInfo,
        old_key_type: String,
        old_fingerprint: String,
    },
}

/// The user's choice in a host key dialog.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostKeyReply {
    /// "Accept and save": a known host record is written, the attempt goes on.
    AcceptAndSave,
    /// "Connect once": the attempt goes on without saving.
    ConnectOnce,
    /// "Replace saved key": the changed key replaces the saved one.
    Replace,
    /// "Cancel": the attempt stops.
    Cancel,
}

/// What the tab sends to the session layer. Every method returns at once;
/// the layer reports the outcome through the tab's setters.
pub trait SessionControl: 'static {
    /// "Cancel" on the connection screen, on a reconnect attempt, or of the
    /// reconnect countdown: stops the attempt and the automatic attempts.
    fn cancel(&self, cx: &mut App);
    /// "Retry", "Reconnect", "Reconnect now": a new attempt in the same tab.
    fn reconnect(&self, cx: &mut App);
    /// The tab closes: end the session.
    fn close(&self, cx: &mut App);
    /// The answer to the pending [`Prompt`].
    fn reply_prompt(&self, reply: PromptReply, cx: &mut App);
    /// The answer to the pending [`HostKeyAsk`].
    fn reply_host_key(&self, reply: HostKeyReply, cx: &mut App);
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn exit_text_and_countdown() {
        assert_eq!(ExitStatus::Code(2).text(), "Session ended with code 2");
        assert_eq!(
            ExitStatus::Signal("KILL".into()).text(),
            "Session ended with signal KILL"
        );
        let now = Instant::now();
        let info = ReconnectInfo {
            attempt: 1,
            next_at: Some(now + Duration::from_millis(3200)),
            waiting_for_unlock: false,
        };
        assert_eq!(info.seconds_left(now), Some(4));
        assert_eq!(info.seconds_left(now + Duration::from_secs(9)), Some(0));
        assert_eq!(
            HostKeyInfo::openssh_address("example.test", 22),
            "example.test"
        );
        assert_eq!(
            HostKeyInfo::openssh_address("example.test", 2222),
            "[example.test]:2222"
        );
    }

    #[test]
    fn every_reason_has_a_text_and_a_code() {
        let mut codes: Vec<&str> = Failure::ALL.iter().map(|failure| failure.code()).collect();
        assert!(
            Failure::ALL
                .iter()
                .all(|failure| !failure.text().is_empty())
        );
        codes.sort_unstable();
        codes.dedup();
        assert_eq!(codes.len(), Failure::ALL.len());
    }

    #[test]
    fn debug_output_leaves_out_server_texts_and_names() {
        let prompt = Prompt::KeyboardInteractive {
            name: "srv-name".into(),
            instruction: "inst".into(),
            prompt: "Code:".into(),
            echo: false,
            index: 0,
            count: 1,
            error: false,
        };
        let text = format!("{prompt:?}");
        assert!(!text.contains("srv-name") && !text.contains("Code:"));
        let password = Prompt::Password {
            username: "alice".into(),
            error: false,
            keep: Keep::Save,
        };
        assert!(!format!("{password:?}").contains("alice"));
        let key = HostKeyInfo {
            address: "example.test".into(),
            key_type: "ssh-ed25519".into(),
            fingerprint: "SHA256:abc".into(),
        };
        assert!(!format!("{key:?}").contains("example.test"));
        let reply = PromptReply::Answer {
            value: SecretString::from("hunter2"),
            keep: false,
        };
        assert!(!format!("{reply:?}").contains("hunter2"));
    }
}
