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
    /// The server violated the protocol.
    Protocol,
    /// The server and Shell++ have no algorithm in common for one of the
    /// negotiations (run 4, plan 9.1, D80): what it offered, by name from
    /// a fixed table only.
    NoCommonAlgorithm(AlgorithmMismatch),
    /// The server refused a channel, PTY, shell or exec request.
    ChannelFailed,
    /// The connection was closed (transport error, keepalive timeout).
    Disconnected,
    /// The private key could not be decoded (not after a passphrase prompt:
    /// a wrong passphrase is asked again).
    KeyUnusable,
    /// The proxy refused the tunnel or did not answer as a proxy.
    ProxyFailed,
    /// The proxy refused the credentials (or wants some and none are set).
    ProxyAuthFailed,
    /// A jump host refused to open a channel to the next host of the route
    /// (OpenSSH `AllowTcpForwarding no`).
    ForwardingRefused,
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
            Self::NoCommonAlgorithm(_) => "no_common_algorithm",
            Self::ChannelFailed => "channel_failed",
            Self::Disconnected => "disconnected",
            Self::KeyUnusable => "key_unusable",
            Self::ProxyFailed => "proxy_failed",
            Self::ProxyAuthFailed => "proxy_auth_failed",
            Self::ForwardingRefused => "forwarding_refused",
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
            E::NoCommonAlgo { kind, theirs, .. } => {
                Self::NoCommonAlgorithm(AlgorithmMismatch::from_offer(kind.into(), theirs))
            }
            _ => Self::Protocol,
        }
    }
}

/// One of the negotiations of the SSH handshake.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AlgorithmKind {
    KeyExchange,
    HostKey,
    Cipher,
    Mac,
    Compression,
}

impl From<&russh::AlgorithmKind> for AlgorithmKind {
    fn from(kind: &russh::AlgorithmKind) -> Self {
        match kind {
            russh::AlgorithmKind::Kex => Self::KeyExchange,
            russh::AlgorithmKind::Key => Self::HostKey,
            russh::AlgorithmKind::Cipher => Self::Cipher,
            russh::AlgorithmKind::Mac => Self::Mac,
            russh::AlgorithmKind::Compression => Self::Compression,
        }
    }
}

/// What the server offered in a negotiation without a common algorithm.
/// Only names of [`crate::algorithms::KNOWN`] are kept (as bits): never the
/// server's text (section 9.1 of docs/security.md); the others are counted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AlgorithmMismatch {
    kind: AlgorithmKind,
    offered: u128,
    others: u8,
}

impl AlgorithmMismatch {
    /// A mismatch in `kind` where the server named nothing known.
    pub const fn new(kind: AlgorithmKind) -> Self {
        Self {
            kind,
            offered: 0,
            others: 0,
        }
    }

    pub fn from_offer(kind: AlgorithmKind, theirs: &[String]) -> Self {
        let mut offered = 0u128;
        let mut others = 0u8;
        for name in theirs {
            match crate::algorithms::KNOWN
                .iter()
                .position(|known| known == name)
            {
                Some(bit) => offered |= 1 << bit,
                None => others = others.saturating_add(1),
            }
        }
        Self {
            kind,
            offered,
            others,
        }
    }

    pub fn kind(&self) -> AlgorithmKind {
        self.kind
    }

    /// The known names the server offered, in the table's order.
    pub fn offered(&self) -> Vec<&'static str> {
        crate::algorithms::KNOWN
            .iter()
            .enumerate()
            .filter(|(bit, _)| self.offered & (1 << bit) != 0)
            .map(|(_, name)| *name)
            .collect()
    }

    /// How many offered names are not in the table.
    pub fn others(&self) -> usize {
        usize::from(self.others)
    }

    /// Whether the host setting "Allow legacy algorithms" would let the
    /// negotiation succeed: the server offers one of them.
    pub fn legacy_would_help(&self) -> bool {
        self.offered()
            .iter()
            .any(|name| crate::algorithms::is_legacy(self.kind, name))
    }
}

/// Where on a route a failure happened (stage 5: jump hosts and a proxy).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FailurePlace {
    /// The host being connected (the last one of the route).
    #[default]
    Target,
    /// The jump host at this index of [`crate::ConnectSpec::jump_hosts`]
    /// (0 = the first one).
    JumpHost(usize),
    /// The proxy of the first connection.
    Proxy,
}

/// An SSH failure: its kind and its place on the route.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("ssh failure: {}{}", .kind.reason(), place_suffix(.place))]
pub struct SshError {
    pub kind: FailureKind,
    pub place: FailurePlace,
}

fn place_suffix(place: &FailurePlace) -> String {
    match place {
        FailurePlace::Target => String::new(),
        FailurePlace::JumpHost(index) => format!(" (jump host {})", index + 1),
        FailurePlace::Proxy => " (proxy)".to_owned(),
    }
}

impl SshError {
    /// A failure of the host being connected.
    pub fn new(kind: FailureKind) -> Self {
        Self {
            kind,
            place: FailurePlace::Target,
        }
    }

    /// Moves a failure of "this connection" ([`FailurePlace::Target`]) to
    /// `place`; a failure placed already (the proxy, an earlier jump host)
    /// keeps its place.
    pub fn placed_at(self, place: FailurePlace) -> Self {
        match self.place {
            FailurePlace::Target => Self { place, ..self },
            _ => self,
        }
    }
}

impl From<FailureKind> for SshError {
    fn from(kind: FailureKind) -> Self {
        Self::new(kind)
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
    fn a_mismatch_keeps_known_names_only() {
        let theirs = [
            "diffie-hellman-group14-sha1".to_owned(),
            "diffie-hellman-group1-sha1".to_owned(),
            "made-up-kex@example.com\u{1b}[31m".to_owned(),
        ];
        let kind = FailureKind::from_russh(&russh::Error::NoCommonAlgo {
            kind: russh::AlgorithmKind::Kex,
            ours: Vec::new(),
            theirs: theirs.to_vec(),
        });
        let FailureKind::NoCommonAlgorithm(mismatch) = kind else {
            panic!("{kind:?}");
        };
        assert_eq!(kind.reason(), "no_common_algorithm");
        assert_eq!(mismatch.kind(), AlgorithmKind::KeyExchange);
        assert_eq!(
            mismatch.offered(),
            ["diffie-hellman-group14-sha1", "diffie-hellman-group1-sha1"]
        );
        assert_eq!(mismatch.others(), 1);
        assert!(mismatch.legacy_would_help());
        let modern = AlgorithmMismatch::from_offer(
            AlgorithmKind::Cipher,
            &["3des-cbc".to_owned(), "arcfour".to_owned()],
        );
        assert!(
            !modern.legacy_would_help(),
            "3DES and RC4 are not offered at all"
        );
    }

    #[test]
    fn display_shows_the_reason_only() {
        assert_eq!(
            SshError::new(FailureKind::AuthFailed).to_string(),
            "ssh failure: auth_failed"
        );
        assert_eq!(
            SshError::new(FailureKind::AuthFailed)
                .placed_at(FailurePlace::JumpHost(1))
                .to_string(),
            "ssh failure: auth_failed (jump host 2)"
        );
    }

    #[test]
    fn a_placed_failure_keeps_its_place() {
        let proxy = SshError::new(FailureKind::ProxyFailed).placed_at(FailurePlace::Proxy);
        assert_eq!(
            proxy.placed_at(FailurePlace::JumpHost(0)).place,
            FailurePlace::Proxy
        );
        let hop = SshError::new(FailureKind::Refused).placed_at(FailurePlace::JumpHost(0));
        assert_eq!(
            hop.placed_at(FailurePlace::JumpHost(1)).place,
            FailurePlace::JumpHost(0)
        );
    }
}
