//! Errors. A failure carries only its kind: never an address, a username,
//! a secret or server text (docs/security.md, section 9.1).

/// Why a connection, authentication or channel failed. The desktop host maps
/// each kind to the IPC error `ssh_error` with `details.reason` =
/// [`FailureKind::reason`] (or to `cancelled`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FailureKind {
    /// The host name did not resolve.
    NotFound,
    /// The TCP connection was refused.
    Refused,
    /// The TCP connection or the SSH handshake timed out.
    Timeout,
    /// Any other network error (unreachable, reset during the handshake).
    Network,
    /// [`crate::Interaction::verify_host_key`] rejected the key.
    HostKeyRejected,
    /// Every authentication method failed.
    AuthFailed,
    /// The user cancelled a prompt (or the attempt was cancelled).
    Cancelled,
    /// The server violated the protocol or offered no common algorithm.
    Protocol,
    /// The server refused a channel, PTY, shell or exec request.
    ChannelFailed,
    /// The connection was closed (transport error, keepalive timeout).
    Disconnected,
    /// The private key could not be decoded (not after a passphrase prompt:
    /// a wrong passphrase is asked again).
    KeyUnusable,
}

impl FailureKind {
    /// `details.reason` of the IPC error `ssh_error` (stage 2 plan, section 5.3).
    pub fn reason(self) -> &'static str {
        match self {
            Self::NotFound => "not_found",
            Self::Refused => "refused",
            Self::Timeout => "timeout",
            Self::Network => "network",
            Self::HostKeyRejected => "host_key_rejected",
            Self::AuthFailed => "auth_failed",
            Self::Cancelled => "cancelled",
            Self::Protocol => "protocol",
            Self::ChannelFailed => "channel_failed",
            Self::Disconnected => "disconnected",
            Self::KeyUnusable => "key_unusable",
        }
    }

    /// The kind of a russh error raised while connecting (handshake) or on
    /// an established connection. The error text is never kept: it may
    /// contain server-provided names.
    pub(crate) fn from_russh(error: &russh::Error) -> FailureKind {
        use russh::Error as E;
        match error {
            E::UnknownKey => Self::HostKeyRejected,
            E::IO(io) => crate::transport::io_failure(io),
            E::ConnectionTimeout | E::Elapsed(_) => Self::Timeout,
            E::KeepaliveTimeout | E::InactivityTimeout => Self::Disconnected,
            E::Disconnect | E::HUP | E::SendError | E::RecvError => Self::Network,
            E::ChannelOpenFailure(_) | E::WrongChannel => Self::ChannelFailed,
            _ => Self::Protocol,
        }
    }
}

/// An SSH failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("ssh failure: {}", .kind.reason())]
pub struct SshError {
    pub kind: FailureKind,
}

impl SshError {
    pub fn new(kind: FailureKind) -> Self {
        Self { kind }
    }
}

impl From<FailureKind> for SshError {
    fn from(kind: FailureKind) -> Self {
        Self { kind }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn russh_errors_map_to_kinds() {
        assert_eq!(
            FailureKind::from_russh(&russh::Error::UnknownKey),
            FailureKind::HostKeyRejected
        );
        assert_eq!(
            FailureKind::from_russh(&russh::Error::IO(std::io::Error::from(
                std::io::ErrorKind::ConnectionRefused
            ))),
            FailureKind::Refused
        );
        assert_eq!(
            FailureKind::from_russh(&russh::Error::Version),
            FailureKind::Protocol
        );
        assert_eq!(
            FailureKind::from_russh(&russh::Error::KeepaliveTimeout),
            FailureKind::Disconnected
        );
        assert_eq!(
            FailureKind::from_russh(&russh::Error::Disconnect),
            FailureKind::Network
        );
    }

    #[test]
    fn display_shows_the_reason_only() {
        assert_eq!(
            SshError::new(FailureKind::AuthFailed).to_string(),
            "ssh failure: auth_failed"
        );
    }
}
