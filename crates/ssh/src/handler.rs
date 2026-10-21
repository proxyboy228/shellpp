//! The russh client handler of one connection attempt: captures the server
//! identification line, routes the host key decision to
//! [`Interaction::verify_host_key`], passes the authentication banner on,
//! accepts forwarded agent channels only after forwarding was requested on
//! this connection and rejects every other server-opened channel
//! (docs/security.md, sections 11.1 and 11.4).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use russh::ChannelOpenFailure;
use russh::client::{ChannelOpenHandle, DisconnectReason, Msg, Session};
use russh::keys::PublicKeyOrCertificate;
use russh::keys::ssh_key::{HashAlg, PublicKey};
use tokio::sync::watch;

use crate::agent::{ForwardedAgent, SystemAgent};
use crate::client::{ConnectStep, HostKeyDecision, HostKeyInfo, Interaction};

/// State shared between the handler (running in the russh session task)
/// and the connection.
pub(crate) struct Shared {
    pub(crate) interaction: Arc<dyn Interaction>,
    server_ident: Mutex<Option<String>>,
    host_key: Mutex<HostKeyRecord>,
    /// A key the user accepted in the previous attempt of the same
    /// [`crate::connect`] call (reconnect after the server closed the
    /// connection during the decision): accepted again without asking.
    auto_accept: Option<HostKeyInfo>,
    /// `true` once the host key decision started; the handshake timeout does
    /// not run while the user decides.
    verifying: watch::Sender<bool>,
    /// The system agent (authentication; forwarded channels when no
    /// `forwarded` agent is set).
    agent: Option<SystemAgent>,
    /// What forwarded agent channels offer (stage 5: the built-in agent).
    forwarded: Option<ForwardedAgent>,
    /// Agent forwarding was requested on a session channel of this connection.
    forwarding_requested: AtomicBool,
    /// The russh session ended (transport error or disconnect).
    transport_closed: AtomicBool,
    /// The transport is gone: the russh session ended, keepalives went
    /// unanswered or the stream was cut locally.
    lost: watch::Sender<bool>,
    /// Why the transport was lost (the first reason wins): `Timeout` after
    /// unanswered keepalives, else from the russh session's end.
    lost_reason: Mutex<Option<crate::FailureKind>>,
    /// The attempt was abandoned (the [`crate::connect`] future was
    /// dropped): a pending host key decision ends as a reject.
    cancelled: watch::Sender<bool>,
    /// Active Remote forwarding rules: the only `forwarded-tcpip` channels
    /// accepted (docs/security.md, section 11.4).
    remote_rules: crate::forward::RemoteRules,
}

#[derive(Default)]
struct HostKeyRecord {
    presented: Option<HostKeyInfo>,
    decision: Option<HostKeyDecision>,
    /// The decision came from [`Interaction::verify_host_key`] (not from
    /// `auto_accept`).
    asked: bool,
}

impl std::fmt::Debug for Shared {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Shared")
            .field("agent", &self.agent.is_some())
            .field("forwarding_requested", &self.forwarding_requested)
            .field("transport_closed", &self.transport_closed)
            .finish_non_exhaustive()
    }
}

impl Shared {
    pub(crate) fn new(
        interaction: Arc<dyn Interaction>,
        auto_accept: Option<HostKeyInfo>,
        agent: Option<SystemAgent>,
        forwarded: Option<ForwardedAgent>,
    ) -> Self {
        Self {
            interaction,
            server_ident: Mutex::new(None),
            host_key: Mutex::new(HostKeyRecord::default()),
            auto_accept,
            verifying: watch::Sender::new(false),
            agent,
            forwarded,
            forwarding_requested: AtomicBool::new(false),
            transport_closed: AtomicBool::new(false),
            lost: watch::Sender::new(false),
            lost_reason: Mutex::new(None),
            cancelled: watch::Sender::new(false),
            remote_rules: crate::forward::RemoteRules::default(),
        }
    }

    pub(crate) fn remote_rules(&self) -> &crate::forward::RemoteRules {
        &self.remote_rules
    }

    /// Abandons the attempt (see `cancelled`).
    pub(crate) fn cancel(&self) {
        self.cancelled.send_replace(true);
    }

    pub(crate) fn server_ident(&self) -> String {
        self.server_ident
            .lock()
            .ok()
            .and_then(|ident| ident.clone())
            .unwrap_or_default()
    }

    /// The decision taken in this attempt, if any.
    pub(crate) fn host_key_decision(&self) -> Option<HostKeyDecision> {
        self.host_key.lock().ok().and_then(|record| record.decision)
    }

    /// The key the user accepted in this attempt through
    /// [`Interaction::verify_host_key`] (not an automatic acceptance).
    pub(crate) fn key_accepted_by_user(&self) -> Option<HostKeyInfo> {
        let record = self.host_key.lock().ok()?;
        if record.asked && record.decision == Some(HostKeyDecision::Accept) {
            record.presented.clone()
        } else {
            None
        }
    }

    pub(crate) fn subscribe_verifying(&self) -> watch::Receiver<bool> {
        self.verifying.subscribe()
    }

    /// What a forwarded agent channel serves: the forwarded agent, else
    /// the system agent alone; `None` when there is nothing to forward.
    pub(crate) fn forwarded_agent(&self) -> Option<ForwardedAgent> {
        self.forwarded
            .clone()
            .or_else(|| self.agent.clone().map(ForwardedAgent::system_only))
    }

    pub(crate) fn set_forwarding_requested(&self, requested: bool) {
        self.forwarding_requested.store(requested, Ordering::SeqCst);
    }

    pub(crate) fn transport_closed(&self) -> bool {
        self.transport_closed.load(Ordering::SeqCst)
    }

    pub(crate) fn mark_lost(&self) {
        self.lost.send_replace(true);
    }

    /// Marks the transport lost for `reason` (kept when a reason is known already).
    pub(crate) fn mark_lost_with(&self, reason: crate::FailureKind) {
        if let Ok(mut slot) = self.lost_reason.lock() {
            slot.get_or_insert(reason);
        }
        self.mark_lost();
    }

    /// Why the transport was lost; `Disconnected` when nothing more is known.
    pub(crate) fn lost_reason(&self) -> crate::FailureKind {
        self.lost_reason
            .lock()
            .ok()
            .and_then(|slot| *slot)
            .unwrap_or(crate::FailureKind::Disconnected)
    }

    pub(crate) fn is_lost(&self) -> bool {
        *self.lost.borrow()
    }

    pub(crate) fn subscribe_lost(&self) -> watch::Receiver<bool> {
        self.lost.subscribe()
    }

    fn record(&self, presented: HostKeyInfo, decision: HostKeyDecision, asked: bool) {
        if let Ok(mut record) = self.host_key.lock() {
            record.presented = Some(presented);
            record.decision = Some(decision);
            record.asked = asked;
        }
    }
}

/// The host key as the store and the dialogs see it.
pub(crate) fn host_key_info(key: &PublicKey) -> Option<HostKeyInfo> {
    let blob = key.to_bytes().ok()?;
    Some(HostKeyInfo {
        key_type: key.algorithm().as_str().to_owned(),
        public_key_base64: data_encoding::BASE64.encode(&blob),
        fingerprint: key.fingerprint(HashAlg::Sha256).to_string(),
    })
}

/// The identification line as printable text (it is server text).
pub(crate) fn sanitize_ident(raw: &[u8]) -> String {
    String::from_utf8_lossy(raw)
        .chars()
        .filter(|c| !c.is_control())
        .take(255)
        .collect::<String>()
        .trim()
        .to_owned()
}

/// The russh handler.
pub(crate) struct ClientHandler {
    shared: Arc<Shared>,
}

impl std::fmt::Debug for ClientHandler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ClientHandler { .. }")
    }
}

impl ClientHandler {
    pub(crate) fn new(shared: Arc<Shared>) -> Self {
        Self { shared }
    }
}

async fn reject(reply: ChannelOpenHandle, kind: &'static str) {
    tracing::debug!(kind, "server-opened channel rejected");
    reply
        .reject(ChannelOpenFailure::AdministrativelyProhibited)
        .await;
}

impl russh::client::Handler for ClientHandler {
    type Error = russh::Error;

    async fn auth_banner(
        &mut self,
        banner: &str,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.shared.interaction.banner(banner);
        Ok(())
    }

    async fn kex_done(
        &mut self,
        _shared_secret: Option<&[u8]>,
        _names: &russh::Names,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        if let Ok(mut ident) = self.shared.server_ident.lock()
            && ident.is_none()
        {
            *ident = Some(sanitize_ident(session.remote_sshid()));
        }
        Ok(())
    }

    async fn check_server_key(
        &mut self,
        server_public_key: &PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        let key = match server_public_key {
            PublicKeyOrCertificate::PublicKey { key, .. } => key,
            // Host certificates are not trusted (docs/security.md, 11.1); none
            // are negotiated, so this is a protocol violation.
            PublicKeyOrCertificate::Certificate(_) => return Ok(false),
        };
        let Some(info) = host_key_info(key) else {
            return Ok(false);
        };
        self.shared.verifying.send_replace(true);
        self.shared.interaction.step(ConnectStep::HostKey);
        let (decision, asked) = if self.shared.auto_accept.as_ref() == Some(&info) {
            (HostKeyDecision::Accept, false)
        } else {
            let interaction = Arc::clone(&self.shared.interaction);
            let mut cancelled = self.shared.cancelled.subscribe();
            tokio::select! {
                decision = interaction.verify_host_key(&info) => (decision, true),
                _ = cancelled.wait_for(|cancelled| *cancelled) => {
                    tracing::debug!("host key decision abandoned");
                    (HostKeyDecision::Reject, false)
                }
            }
        };
        tracing::debug!(?decision, asked, "host key decided");
        self.shared.record(info, decision, asked);
        Ok(decision == HostKeyDecision::Accept)
    }

    async fn server_channel_open_agent_forward(
        &mut self,
        channel: russh::Channel<Msg>,
        reply: ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        let allowed = self.shared.forwarding_requested.load(Ordering::SeqCst);
        match (allowed, self.shared.forwarded_agent()) {
            (true, Some(agent)) => {
                reply.accept().await;
                tracing::debug!("forwarded agent channel accepted");
                tokio::spawn(crate::agent::serve_forwarded_channel(
                    channel.into_stream(),
                    agent,
                ));
            }
            _ => reject(reply, "auth-agent").await,
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    async fn server_channel_open_forwarded_tcpip(
        &mut self,
        channel: russh::Channel<Msg>,
        connected_address: &str,
        connected_port: u32,
        _originator_address: &str,
        _originator_port: u32,
        reply: ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        // Only for an active Remote rule with that address and port.
        match self
            .shared
            .remote_rules
            .destination(connected_address, connected_port)
        {
            Some((host, port, state)) => {
                reply.accept().await;
                tokio::spawn(crate::forward::serve_remote_channel(
                    channel, host, port, state,
                ));
            }
            None => reject(reply, "forwarded-tcpip").await,
        }
        Ok(())
    }

    async fn server_channel_open_forwarded_streamlocal(
        &mut self,
        _channel: russh::Channel<Msg>,
        _socket_path: &str,
        reply: ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        reject(reply, "forwarded-streamlocal").await;
        Ok(())
    }

    async fn server_channel_open_session(
        &mut self,
        _channel: russh::Channel<Msg>,
        reply: ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        reject(reply, "session").await;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    async fn server_channel_open_direct_tcpip(
        &mut self,
        _channel: russh::Channel<Msg>,
        _host_to_connect: &str,
        _port_to_connect: u32,
        _originator_address: &str,
        _originator_port: u32,
        reply: ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        reject(reply, "direct-tcpip").await;
        Ok(())
    }

    async fn server_channel_open_direct_streamlocal(
        &mut self,
        _channel: russh::Channel<Msg>,
        _socket_path: &str,
        reply: ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        reject(reply, "direct-streamlocal").await;
        Ok(())
    }

    async fn server_channel_open_x11(
        &mut self,
        _channel: russh::Channel<Msg>,
        _originator_address: &str,
        _originator_port: u32,
        reply: ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        reject(reply, "x11").await;
        Ok(())
    }

    async fn disconnected(
        &mut self,
        reason: DisconnectReason<Self::Error>,
    ) -> Result<(), Self::Error> {
        self.shared.transport_closed.store(true, Ordering::SeqCst);
        let kind = match reason {
            DisconnectReason::ReceivedDisconnect(_) => {
                tracing::debug!("server sent disconnect");
                crate::FailureKind::Disconnected
            }
            DisconnectReason::Error(error) => {
                tracing::debug!(
                    kind = crate::FailureKind::from_russh(&error).reason(),
                    "transport closed"
                );
                drop_kind(&error)
            }
        };
        self.shared.mark_lost_with(kind);
        Ok(())
    }
}

/// Why a transport error ended the connection. A socket error (reset,
/// unreachable, and an OS-level timeout on a dead link) is a network
/// failure; an end of stream or a protocol error is shown as
/// "disconnected". `Timeout` is left to the keepalive loop, whose reason
/// wins because it marks the loss first: only it means "the server stopped
/// answering keepalives", which must not show when keepalive is off.
fn drop_kind(error: &russh::Error) -> crate::FailureKind {
    match error {
        russh::Error::IO(io) if io.kind() != std::io::ErrorKind::UnexpectedEof => {
            crate::FailureKind::Network
        }
        _ => crate::FailureKind::Disconnected,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ident_is_printable_and_bounded() {
        assert_eq!(
            sanitize_ident(b"SSH-2.0-OpenSSH_9.6p1 Ubuntu-3ubuntu13\r"),
            "SSH-2.0-OpenSSH_9.6p1 Ubuntu-3ubuntu13"
        );
        assert_eq!(sanitize_ident(b"SSH-2.0-x\x1b[31m"), "SSH-2.0-x[31m");
        assert_eq!(sanitize_ident(&[b'a'; 1000]).len(), 255);
    }

    #[test]
    fn socket_errors_are_network_drops_never_keepalive_timeouts() {
        use std::io::{Error, ErrorKind};
        let io = |kind| russh::Error::IO(Error::from(kind));
        for kind in [
            ErrorKind::TimedOut,
            ErrorKind::ConnectionReset,
            ErrorKind::ConnectionAborted,
            ErrorKind::BrokenPipe,
        ] {
            assert_eq!(
                drop_kind(&io(kind)),
                crate::FailureKind::Network,
                "{kind:?}"
            );
        }
        assert_eq!(
            drop_kind(&io(ErrorKind::UnexpectedEof)),
            crate::FailureKind::Disconnected
        );
        assert_eq!(
            drop_kind(&russh::Error::HUP),
            crate::FailureKind::Disconnected
        );
    }
}
