//! English texts of sessions (UI 0; the meaning comes from
//! docs/ui-behavior-spec.md, sections 4.6, 4.8 and 10.2): connection steps,
//! failure and drop reasons, the end of a session and the reconnect
//! countdown. Views show these instead of literals.

use shellpp_ssh::FailureKind;

use super::types::{ConnectStep, SessionError};

/// The connection screen's current step (UI 4.8).
pub fn step_text(step: ConnectStep) -> &'static str {
    match step {
        ConnectStep::Resolving => "Looking up the address",
        ConnectStep::Connecting => "Connecting",
        ConnectStep::HostKey => "Checking the host key",
        ConnectStep::Authenticating => "Authenticating",
        ConnectStep::OpeningSession => "Opening the session",
    }
}

/// "Session ended with code N" (UI 4.6).
pub fn ended_with_code(code: u32) -> String {
    format!("Session ended with code {code}")
}

/// The end of a shell killed by a signal.
pub fn ended_by_signal(signal: &str) -> String {
    format!("Session ended by signal {signal}")
}

/// The countdown of automatic reconnection (UI 4.6).
pub fn reconnecting_in(seconds: u64) -> String {
    if seconds == 1 {
        "Reconnecting in 1 second".to_owned()
    } else {
        format!("Reconnecting in {seconds} seconds")
    }
}

/// Shown while an automatic attempt runs.
pub const RECONNECTING: &str = "Reconnecting";
/// Shown while automatic attempts wait for unlock (UI 13.3).
pub const RECONNECT_AFTER_UNLOCK: &str = "Reconnects after unlock";

// Buttons of the overlays and the failure screen (UI 4.6, 4.8).
pub const RECONNECT: &str = "Reconnect";
pub const RETRY: &str = "Retry";
pub const CLOSE: &str = "Close";
pub const CANCEL: &str = "Cancel";
pub const CONTINUE: &str = "Continue";

// Prompts (UI 4.8).
pub const USERNAME: &str = "Username";
pub const PASSWORD: &str = "Password";
pub const PASSPHRASE: &str = "Passphrase";
pub const SAVE: &str = "Save";
pub const REMEMBER_FOR_SESSION: &str = "Remember for this session";
pub const WRONG_PASSWORD: &str = "Wrong password. Try again.";
pub const WRONG_PASSPHRASE: &str = "Wrong passphrase. Try again.";
pub const WRONG_ANSWER: &str = "The answer was not accepted. Try again.";

/// "Password for alice".
pub fn password_for(username: &str) -> String {
    format!("Password for {username}")
}

/// "Passphrase for id_ed25519".
pub fn passphrase_for(key_label: &str) -> String {
    format!("Passphrase for {key_label}")
}

// Host key dialogs (UI 10.2).
pub const HOST_KEY_UNKNOWN_TITLE: &str = "Unknown host key";
pub const HOST_KEY_UNKNOWN_BODY: &str = "This is the first connection to this host. Check that the fingerprint matches the server's key before you continue.";
pub const ACCEPT_AND_SAVE: &str = "Accept and save";
pub const CONNECT_ONCE: &str = "Connect once";
pub const HOST_KEY_CHANGED_TITLE: &str = "The host key has changed";
pub const HOST_KEY_CHANGED_BODY: &str = "The server presented a different key than the one saved for this host. Someone may be intercepting the connection, or the server was reinstalled. Continue only if you know why the key changed.";
pub const REPLACE_SAVED_KEY: &str = "Replace saved key";
pub const OLD_FINGERPRINT: &str = "Saved fingerprint";
pub const NEW_FINGERPRINT: &str = "Presented fingerprint";
pub const HOST_KEY_REVOKED_TITLE: &str = "The host key is revoked";

/// The text of a failure or drop reason.
pub fn error_message(error: SessionError) -> &'static str {
    match error {
        SessionError::Ssh(kind) => match kind {
            FailureKind::NotFound => "The address could not be found.",
            FailureKind::Refused => "The server refused the connection.",
            FailureKind::Timeout => "The connection timed out.",
            FailureKind::Network => "The server could not be reached. Check the network.",
            FailureKind::HostKeyRejected => "The host key was not accepted.",
            FailureKind::AuthFailed => "Authentication failed.",
            FailureKind::Cancelled => "The connection was cancelled.",
            FailureKind::Protocol => "The SSH handshake with the server failed.",
            FailureKind::ChannelFailed => "The server refused to open a session.",
            FailureKind::Disconnected => "The server closed the connection.",
            FailureKind::KeyUnusable => "The private key could not be used.",
            FailureKind::ProxyFailed => "The proxy did not open the connection.",
            FailureKind::ProxyAuthFailed => "The proxy did not accept the username or password.",
            FailureKind::ForwardingRefused => {
                "A jump host does not allow connections to the next host."
            }
        },
        SessionError::ConnectionLost => "The connection was lost.",
        SessionError::KeepaliveTimeout => "The server stopped responding.",
        SessionError::ClosedByServer => "The server closed the connection.",
        SessionError::HostKeyRevoked => {
            "The server presented a revoked host key. The connection was blocked."
        }
        SessionError::Cancelled => "The connection was cancelled.",
        SessionError::Locked => "Shell++ is locked.",
        SessionError::HostMissing => "The host no longer exists.",
        SessionError::InvalidTarget => "The address is not valid.",
        SessionError::RouteInvalid => {
            "A jump host of this host is missing or listed twice. Check Host Chaining."
        }
        SessionError::ShellUnavailable => "The local shell could not be started.",
        SessionError::Internal => "Something went wrong. Try again.",
    }
}
