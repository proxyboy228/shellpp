//! Port forwarding over one connection (UI 8; docs/data-model.md, section
//! 11; docs/security.md, sections 11.3 and 11.4):
//!
//! - Local: a listener on this computer; each accepted connection becomes a
//!   `direct-tcpip` channel to the destination as the SSH host sees it.
//! - Remote: `tcpip-forward` on the SSH host. Its `forwarded-tcpip` channels
//!   are accepted only while a rule with that address and port is active
//!   ([`crate::handler`]) and are connected to the destination as this
//!   computer sees it.
//! - Dynamic: a SOCKS5 proxy on this computer, without authentication (it
//!   listens on loopback by default, RR14). Each CONNECT becomes a
//!   `direct-tcpip` channel with the requested name, so names are resolved
//!   on the remote side.
//!
//! An empty bind address means `127.0.0.1`, never all interfaces
//! (docs/security.md, section 11.3). Stopping a rule closes its listener and
//! its open connections. When the connection is lost, every rule on it ends
//! with [`ForwardEnd::ConnectionLost`]; [`HostForwarder`] brings them up
//! again on a new connection.
//!
//! Nothing here logs addresses or ports (docs/security.md, section 9.1).

use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;
use tokio::task::JoinSet;

use crate::client::{BoxFuture, ConnCore, Connection};
use crate::error::{FailureKind, SshError};
use crate::reconnect::Backoff;

/// What a rule forwards (`forwarding_rule.type` with its destination).
#[derive(Clone, PartialEq, Eq)]
pub enum ForwardKind {
    /// Listens here, connects to `host:port` as the SSH host sees it.
    Local { host: String, port: u16 },
    /// Listens on the SSH host, connects to `host:port` as this computer sees it.
    Remote { host: String, port: u16 },
    /// A SOCKS5 proxy here; every request goes through the SSH host.
    Dynamic,
}

/// One forwarding rule.
#[derive(Clone, PartialEq, Eq)]
pub struct ForwardSpec {
    pub kind: ForwardKind,
    /// An IP literal or `localhost`; empty means `127.0.0.1`. For Remote the
    /// SSH host interprets it.
    pub bind_address: String,
    /// 0: any free port (Local, Dynamic: chosen here; Remote: by the server).
    pub bind_port: u16,
}

impl std::fmt::Debug for ForwardKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The destination is host data: never in logs.
        f.write_str(match self {
            Self::Local { .. } => "Local",
            Self::Remote { .. } => "Remote",
            Self::Dynamic => "Dynamic",
        })
    }
}

impl std::fmt::Debug for ForwardSpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let kind = match self.kind {
            ForwardKind::Local { .. } => "local",
            ForwardKind::Remote { .. } => "remote",
            ForwardKind::Dynamic => "dynamic",
        };
        f.debug_struct("ForwardSpec")
            .field("kind", &kind)
            .finish_non_exhaustive()
    }
}

/// Why a rule did not start.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ForwardError {
    /// The local port is in use (UI 8.4: a clear error with the port number).
    #[error("port {0} is already in use")]
    PortBusy(u16),
    /// The local port needs rights this process lacks (ports below 1024 on
    /// Unix).
    #[error("port {0} cannot be opened without administrator rights")]
    PortDenied(u16),
    /// The bind address is not an address of this computer, or not valid.
    #[error("the bind address is not available on this computer")]
    AddressUnavailable,
    /// The SSH host refused `tcpip-forward` (its port is in use, or
    /// forwarding is not allowed there).
    #[error("the server refused to listen on that port")]
    Refused,
    /// The connection is gone.
    #[error("the connection is closed")]
    Disconnected,
    /// The server did not answer in time.
    #[error("the server did not answer in time")]
    Timeout,
}

impl ForwardError {
    /// A stable code for logs and the interface.
    pub fn reason(self) -> &'static str {
        match self {
            Self::PortBusy(_) => "port_busy",
            Self::PortDenied(_) => "port_denied",
            Self::AddressUnavailable => "address_unavailable",
            Self::Refused => "refused",
            Self::Disconnected => "disconnected",
            Self::Timeout => "timeout",
        }
    }
}

/// How a running rule ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ForwardEnd {
    /// [`Forward::stop`] (or the handle was dropped).
    Stopped,
    /// The connection was lost.
    ConnectionLost,
}

/// Where a running rule listens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bound {
    /// The local listener (Local, Dynamic).
    Local(SocketAddr),
    /// The port on the SSH host (Remote).
    Remote(u16),
}

/// State of one running rule, shared with its tasks (and, for Remote, with
/// the connection's handler).
#[derive(Debug)]
pub(crate) struct ForwardState {
    stop: watch::Sender<bool>,
    end: watch::Sender<Option<ForwardEnd>>,
    active: AtomicUsize,
}

impl ForwardState {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            stop: watch::Sender::new(false),
            end: watch::Sender::new(None),
            active: AtomicUsize::new(0),
        })
    }

    fn finish(&self, end: ForwardEnd) {
        self.end.send_if_modified(|slot| {
            if slot.is_none() {
                *slot = Some(end);
                true
            } else {
                false
            }
        });
    }
}

/// Waits until `flag` is set (or its sender is gone). The `watch::Ref` of
/// `wait_for` is dropped here: held across an await it would make the
/// caller's future not `Send`.
async fn set(flag: &mut watch::Receiver<bool>) {
    let _ = flag.wait_for(|set| *set).await;
}

/// Counts an open forwarded connection while it lives.
struct ActiveGuard(Arc<ForwardState>);

impl ActiveGuard {
    fn new(state: &Arc<ForwardState>) -> Self {
        state.active.fetch_add(1, Ordering::SeqCst);
        Self(Arc::clone(state))
    }
}

impl Drop for ActiveGuard {
    fn drop(&mut self) {
        self.0.active.fetch_sub(1, Ordering::SeqCst);
    }
}

/// A Remote rule as the connection's handler matches `forwarded-tcpip`
/// channels against it.
pub(crate) struct RemoteRule {
    pub(crate) id: u64,
    /// The address as requested (the server reports it back).
    pub(crate) address: String,
    pub(crate) port: u32,
    pub(crate) dest_host: String,
    pub(crate) dest_port: u16,
    pub(crate) state: Arc<ForwardState>,
}

/// Remote rules of one connection.
#[derive(Default)]
pub(crate) struct RemoteRules {
    rules: Mutex<Vec<RemoteRule>>,
    next_id: AtomicU64,
}

impl RemoteRules {
    fn add(&self, rule: RemoteRule) {
        if let Ok(mut rules) = self.rules.lock() {
            rules.push(rule);
        }
    }

    fn remove(&self, id: u64) {
        if let Ok(mut rules) = self.rules.lock() {
            rules.retain(|rule| rule.id != id);
        }
    }

    fn set_port(&self, id: u64, port: u32) {
        if let Ok(mut rules) = self.rules.lock()
            && let Some(rule) = rules.iter_mut().find(|rule| rule.id == id)
        {
            rule.port = port;
        }
    }

    /// The destination of an incoming `forwarded-tcpip` channel: only for an
    /// active rule with that address and port (docs/security.md, 11.4).
    pub(crate) fn destination(
        &self,
        address: &str,
        port: u32,
    ) -> Option<(String, u16, Arc<ForwardState>)> {
        let rules = self.rules.lock().ok()?;
        rules
            .iter()
            .find(|rule| {
                rule.port == port
                    && rule.address.eq_ignore_ascii_case(address)
                    && !*rule.state.stop.borrow()
            })
            .map(|rule| {
                (
                    rule.dest_host.clone(),
                    rule.dest_port,
                    Arc::clone(&rule.state),
                )
            })
    }
}

/// A running rule. Dropping it stops the rule.
pub struct Forward {
    state: Arc<ForwardState>,
    bound: Bound,
    /// The listener task (Local, Dynamic): [`Forward::stop`] waits for it,
    /// so the port is free when it returns.
    listener: Option<tokio::task::JoinHandle<()>>,
    /// Remote: cancels `tcpip-forward` and removes the rule on stop.
    remote: Option<(Arc<ConnCore>, u64, String)>,
}

impl std::fmt::Debug for Forward {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Forward { .. }")
    }
}

impl Forward {
    /// Where the rule listens.
    pub fn bound(&self) -> Bound {
        self.bound
    }

    /// Forwarded connections open right now.
    pub fn active_connections(&self) -> usize {
        self.state.active.load(Ordering::SeqCst)
    }

    /// How the rule ended, once it did.
    pub fn end(&self) -> Option<ForwardEnd> {
        *self.state.end.borrow()
    }

    /// Waits until the rule ends.
    pub async fn ended(&self) -> ForwardEnd {
        let mut end = self.state.end.subscribe();
        loop {
            if let Some(end) = *end.borrow_and_update() {
                return end;
            }
            if end.changed().await.is_err() {
                return ForwardEnd::Stopped;
            }
        }
    }

    /// Stops the rule: closes the listener (Remote: cancels it on the
    /// server) and the open connections.
    pub async fn stop(mut self) {
        self.stop_now();
        if let Some(listener) = self.listener.take() {
            let _ = listener.await;
        }
        if let Some((core, id, address)) = self.remote.take() {
            core.remote_rules().remove(id);
            if let Bound::Remote(port) = self.bound
                && !core.is_closed()
            {
                let _ = tokio::time::timeout(
                    core.request_timeout(),
                    core.handle().cancel_tcpip_forward(address, u32::from(port)),
                )
                .await;
            }
        }
    }

    fn stop_now(&self) {
        self.state.stop.send_replace(true);
        self.state.finish(ForwardEnd::Stopped);
    }
}

impl Drop for Forward {
    fn drop(&mut self) {
        self.stop_now();
        if let Some((core, id, address)) = self.remote.take() {
            core.remote_rules().remove(id);
            if let (Bound::Remote(port), Ok(runtime)) =
                (self.bound, tokio::runtime::Handle::try_current())
            {
                runtime.spawn(async move {
                    let _ = tokio::time::timeout(
                        core.request_timeout(),
                        core.handle().cancel_tcpip_forward(address, u32::from(port)),
                    )
                    .await;
                });
            }
        }
    }
}

/// The address a local listener binds: empty and `localhost` are `127.0.0.1`.
fn local_bind_ip(address: &str) -> Result<IpAddr, ForwardError> {
    let address = address.trim();
    if address.is_empty() || address.eq_ignore_ascii_case("localhost") {
        return Ok(IpAddr::V4(Ipv4Addr::LOCALHOST));
    }
    let address = address.trim_start_matches('[').trim_end_matches(']');
    address
        .parse()
        .map_err(|_| ForwardError::AddressUnavailable)
}

fn bind_error(error: &std::io::Error, port: u16) -> ForwardError {
    match error.kind() {
        std::io::ErrorKind::AddrInUse => ForwardError::PortBusy(port),
        std::io::ErrorKind::PermissionDenied => ForwardError::PortDenied(port),
        _ => ForwardError::AddressUnavailable,
    }
}

/// Time a SOCKS5 client has for its handshake.
const SOCKS_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
/// Time a Remote rule's destination has to accept a connection.
const DEST_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Pause after a failed accept (out of file descriptors), so the loop does
/// not spin.
const ACCEPT_RETRY: Duration = Duration::from_millis(100);

/// Starts `spec` on the connection behind `core`.
pub(crate) async fn start(
    core: &Arc<ConnCore>,
    spec: ForwardSpec,
) -> Result<Forward, ForwardError> {
    if core.is_closed() {
        return Err(ForwardError::Disconnected);
    }
    match spec.kind {
        ForwardKind::Local { host, port } => {
            start_listener(core, &spec.bind_address, spec.bind_port, Some((host, port))).await
        }
        ForwardKind::Dynamic => {
            start_listener(core, &spec.bind_address, spec.bind_port, None).await
        }
        ForwardKind::Remote { host, port } => {
            start_remote(core, spec.bind_address, spec.bind_port, host, port).await
        }
    }
}

async fn start_listener(
    core: &Arc<ConnCore>,
    bind_address: &str,
    bind_port: u16,
    dest: Option<(String, u16)>,
) -> Result<Forward, ForwardError> {
    let ip = local_bind_ip(bind_address)?;
    let listener = TcpListener::bind(SocketAddr::new(ip, bind_port))
        .await
        .map_err(|error| bind_error(&error, bind_port))?;
    let local = listener
        .local_addr()
        .map_err(|_| ForwardError::AddressUnavailable)?;
    let state = ForwardState::new();
    let task = tokio::spawn(run_listener(
        listener,
        Arc::clone(core),
        dest,
        Arc::clone(&state),
    ));
    Ok(Forward {
        state,
        bound: Bound::Local(local),
        listener: Some(task),
        remote: None,
    })
}

async fn run_listener(
    listener: TcpListener,
    core: Arc<ConnCore>,
    dest: Option<(String, u16)>,
    state: Arc<ForwardState>,
) {
    let mut lost = core.subscribe_lost();
    let mut stop = state.stop.subscribe();
    let mut connections = JoinSet::new();
    loop {
        tokio::select! {
            accepted = listener.accept() => match accepted {
                Ok((socket, peer)) => {
                    let _ = socket.set_nodelay(true);
                    let core = Arc::clone(&core);
                    let state = Arc::clone(&state);
                    let dest = dest.clone();
                    connections.spawn(async move {
                        let _active = ActiveGuard::new(&state);
                        match dest {
                            Some((host, port)) => forward_local(socket, peer, &core, &host, port).await,
                            None => forward_socks(socket, peer, &core).await,
                        }
                    });
                }
                Err(_) => tokio::time::sleep(ACCEPT_RETRY).await,
            },
            // Reap finished connections so the set does not grow.
            Some(_) = connections.join_next(), if !connections.is_empty() => {}
            _ = set(&mut lost) => {
                state.finish(ForwardEnd::ConnectionLost);
                break;
            }
            _ = set(&mut stop) => {
                state.finish(ForwardEnd::Stopped);
                break;
            }
        }
    }
    // Dropping the set aborts the open connections; dropping the listener
    // frees the port.
    drop(connections);
    drop(listener);
}

/// Opens a `direct-tcpip` channel to `host:port` through the SSH host.
async fn open_direct(
    core: &ConnCore,
    host: &str,
    port: u16,
    peer: SocketAddr,
) -> Result<russh::Channel<russh::client::Msg>, russh::Error> {
    match tokio::time::timeout(
        core.request_timeout(),
        core.handle().channel_open_direct_tcpip(
            host,
            u32::from(port),
            peer.ip().to_string(),
            u32::from(peer.port()),
        ),
    )
    .await
    {
        Ok(result) => result,
        Err(_) => Err(russh::Error::ConnectionTimeout),
    }
}

/// How long a finished forwarded connection waits for the server to close
/// its channel before closing it itself.
const CLOSE_WAIT: Duration = Duration::from_secs(5);

/// Closes a channel when it is dropped unasked (a stopped rule aborts its
/// connections mid-copy): what `ChannelStream` did on drop.
struct CloseOnDrop(Option<russh::ChannelWriteHalf<russh::client::Msg>>);

impl Drop for CloseOnDrop {
    fn drop(&mut self) {
        if let Some(write) = self.0.take()
            && let Ok(runtime) = tokio::runtime::Handle::try_current()
        {
            runtime.spawn(async move {
                let _ = write.close().await;
            });
        }
    }
}

/// How the server's half of a forwarded connection ended.
enum Download {
    /// The server sent its EOF; the socket's write side is shut.
    Eof,
    /// The server closed the channel, or the SSH connection ended.
    Closed,
    /// The socket failed: the local side is gone.
    Failed,
}

/// Copies between `socket` and `channel` until both directions end, then
/// lets the SSH server close the channel first. russh 0.64 keeps the
/// client's entry of a channel the client closed first (the server's CLOSE
/// then finds the channel gone and returns before removing it), so with
/// every forwarded connection closed by us a long run grew by a few KB per
/// connection (run 6, the soak test). Only a server that does not close
/// within [`CLOSE_WAIT`] gets our CLOSE.
///
/// The channel is read message by message, not through russh's reader:
/// that reader closes the channel's queue at the EOF, so the server's CLOSE
/// could not be seen. It is read until the CLOSE, also after the server's
/// EOF: russh hands the CLOSE over only when the queue has room, and holds
/// the whole SSH connection until then. Our EOF goes out when the socket
/// ends or fails. A failed socket does not wait for the server's half to
/// end, and the server's CLOSE does not wait for ours.
async fn relay(socket: &mut TcpStream, channel: russh::Channel<russh::client::Msg>) {
    let (mut read, write) = channel.split();
    let mut guard = CloseOnDrop(Some(write));
    let Some(half) = guard.0.as_ref() else {
        return;
    };
    let mut to_channel = half.make_writer();
    let (mut from_socket, mut to_socket) = socket.split();
    // `false` when the socket failed.
    let upload = async {
        let copied = tokio::io::copy(&mut from_socket, &mut to_channel).await;
        let _ = to_channel.shutdown().await;
        copied.is_ok()
    };
    tokio::pin!(upload);
    // `Some` once our EOF went out.
    let mut uploaded = None;
    let ended = {
        let download = async {
            loop {
                match read.wait().await {
                    Some(russh::ChannelMsg::Data { data }) => {
                        if to_socket.write_all(&data).await.is_err() {
                            return Download::Failed;
                        }
                    }
                    Some(russh::ChannelMsg::Eof) => {
                        let _ = to_socket.shutdown().await;
                        return Download::Eof;
                    }
                    Some(russh::ChannelMsg::Close) | None => {
                        let _ = to_socket.shutdown().await;
                        return Download::Closed;
                    }
                    Some(_) => {}
                }
            }
        };
        tokio::pin!(download);
        loop {
            tokio::select! {
                copied = &mut upload, if uploaded.is_none() => {
                    uploaded = Some(copied);
                    if !copied {
                        break Download::Failed;
                    }
                }
                ended = &mut download => break ended,
            }
        }
    };
    let closed_by_server = match ended {
        Download::Closed => true,
        Download::Eof | Download::Failed => {
            if uploaded.is_none() && matches!(ended, Download::Failed) {
                // The upload stops here.
                let _ = half.eof().await;
                uploaded = Some(false);
            }
            // After the server's EOF the upload goes on; the wait for the
            // server's CLOSE starts when it ends.
            let mut deadline = uploaded.map(|_| tokio::time::Instant::now() + CLOSE_WAIT);
            loop {
                let wait = tokio::time::sleep_until(
                    deadline.unwrap_or_else(|| tokio::time::Instant::now() + CLOSE_WAIT),
                );
                tokio::select! {
                    copied = &mut upload, if uploaded.is_none() => {
                        uploaded = Some(copied);
                        deadline = Some(tokio::time::Instant::now() + CLOSE_WAIT);
                    }
                    message = read.wait() => {
                        if matches!(message, Some(russh::ChannelMsg::Close) | None) {
                            break true;
                        }
                    }
                    () = wait, if deadline.is_some() => break false,
                }
            }
        }
    };
    if closed_by_server {
        // Nothing left to close.
        guard.0 = None;
    }
}

/// Ends a channel nothing went through: our EOF, then the server's CLOSE as
/// in [`relay`].
async fn end_channel(channel: russh::Channel<russh::client::Msg>) {
    let (mut read, write) = channel.split();
    let mut guard = CloseOnDrop(Some(write));
    if let Some(half) = guard.0.as_ref() {
        let _ = half.eof().await;
    }
    let closed = tokio::time::timeout(CLOSE_WAIT, async {
        while let Some(message) = read.wait().await {
            if matches!(message, russh::ChannelMsg::Close) {
                return;
            }
        }
    })
    .await
    .is_ok();
    if closed {
        guard.0 = None;
    }
}

async fn forward_local(
    mut socket: TcpStream,
    peer: SocketAddr,
    core: &ConnCore,
    host: &str,
    port: u16,
) {
    match open_direct(core, host, port, peer).await {
        Ok(channel) => relay(&mut socket, channel).await,
        Err(_) => tracing::debug!("forwarded connection refused by the ssh host"),
    }
}

// SOCKS5 server constants (RFC 1928).
const SOCKS_VERSION: u8 = 5;
const METHOD_NONE: u8 = 0x00;
const METHOD_REFUSED: u8 = 0xFF;
const CMD_CONNECT: u8 = 0x01;
const ATYP_IPV4: u8 = 0x01;
const ATYP_DOMAIN: u8 = 0x03;
const ATYP_IPV6: u8 = 0x04;
const REP_SUCCEEDED: u8 = 0x00;
const REP_GENERAL_FAILURE: u8 = 0x01;
const REP_NOT_ALLOWED: u8 = 0x02;
const REP_HOST_UNREACHABLE: u8 = 0x04;
const REP_REFUSED: u8 = 0x05;
const REP_COMMAND_NOT_SUPPORTED: u8 = 0x07;
const REP_ADDRESS_NOT_SUPPORTED: u8 = 0x08;

/// A SOCKS5 CONNECT request: the destination as the client named it.
#[derive(Debug, PartialEq, Eq)]
struct SocksRequest {
    host: String,
    port: u16,
}

/// Why a SOCKS5 handshake ended without a request.
#[derive(Debug, PartialEq, Eq)]
enum SocksRefusal {
    /// The stream ended or did not speak SOCKS5: close it.
    Close,
    /// A request we answer with this reply code, then close.
    Reply(u8),
}

/// Reads the client's greeting and request (no authentication offered).
async fn socks_handshake<S>(stream: &mut S) -> Result<SocksRequest, SocksRefusal>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let mut head = [0u8; 2];
    stream
        .read_exact(&mut head)
        .await
        .map_err(|_| SocksRefusal::Close)?;
    if head[0] != SOCKS_VERSION || head[1] == 0 {
        return Err(SocksRefusal::Close);
    }
    let mut methods = vec![0u8; usize::from(head[1])];
    stream
        .read_exact(&mut methods)
        .await
        .map_err(|_| SocksRefusal::Close)?;
    if !methods.contains(&METHOD_NONE) {
        let _ = stream.write_all(&[SOCKS_VERSION, METHOD_REFUSED]).await;
        return Err(SocksRefusal::Close);
    }
    stream
        .write_all(&[SOCKS_VERSION, METHOD_NONE])
        .await
        .map_err(|_| SocksRefusal::Close)?;

    let mut request = [0u8; 4];
    stream
        .read_exact(&mut request)
        .await
        .map_err(|_| SocksRefusal::Close)?;
    if request[0] != SOCKS_VERSION {
        return Err(SocksRefusal::Close);
    }
    let host = match request[3] {
        ATYP_IPV4 => {
            let mut octets = [0u8; 4];
            stream
                .read_exact(&mut octets)
                .await
                .map_err(|_| SocksRefusal::Close)?;
            Ipv4Addr::from(octets).to_string()
        }
        ATYP_IPV6 => {
            let mut octets = [0u8; 16];
            stream
                .read_exact(&mut octets)
                .await
                .map_err(|_| SocksRefusal::Close)?;
            std::net::Ipv6Addr::from(octets).to_string()
        }
        ATYP_DOMAIN => {
            let mut len = [0u8; 1];
            stream
                .read_exact(&mut len)
                .await
                .map_err(|_| SocksRefusal::Close)?;
            let mut name = vec![0u8; usize::from(len[0])];
            stream
                .read_exact(&mut name)
                .await
                .map_err(|_| SocksRefusal::Close)?;
            match String::from_utf8(name) {
                Ok(name) if !name.is_empty() && !name.chars().any(char::is_control) => name,
                _ => return Err(SocksRefusal::Reply(REP_HOST_UNREACHABLE)),
            }
        }
        _ => return Err(SocksRefusal::Reply(REP_ADDRESS_NOT_SUPPORTED)),
    };
    let mut port = [0u8; 2];
    stream
        .read_exact(&mut port)
        .await
        .map_err(|_| SocksRefusal::Close)?;
    if request[1] != CMD_CONNECT {
        return Err(SocksRefusal::Reply(REP_COMMAND_NOT_SUPPORTED));
    }
    Ok(SocksRequest {
        host,
        port: u16::from_be_bytes(port),
    })
}

/// A SOCKS5 reply with the unspecified bound address `0.0.0.0:0`.
fn socks_reply(code: u8) -> [u8; 10] {
    [SOCKS_VERSION, code, 0, ATYP_IPV4, 0, 0, 0, 0, 0, 0]
}

/// The SOCKS5 reply code for a channel the SSH host did not open.
fn socks_failure(error: &russh::Error) -> u8 {
    match error {
        russh::Error::ChannelOpenFailure(russh::ChannelOpenFailure::ConnectFailed) => REP_REFUSED,
        russh::Error::ChannelOpenFailure(russh::ChannelOpenFailure::AdministrativelyProhibited) => {
            REP_NOT_ALLOWED
        }
        _ => REP_GENERAL_FAILURE,
    }
}

async fn forward_socks(mut socket: TcpStream, peer: SocketAddr, core: &ConnCore) {
    let request =
        match tokio::time::timeout(SOCKS_HANDSHAKE_TIMEOUT, socks_handshake(&mut socket)).await {
            Ok(Ok(request)) => request,
            Ok(Err(SocksRefusal::Reply(code))) => {
                let _ = socket.write_all(&socks_reply(code)).await;
                return;
            }
            Ok(Err(SocksRefusal::Close)) | Err(_) => return,
        };
    match open_direct(core, &request.host, request.port, peer).await {
        Ok(channel) => {
            if socket.write_all(&socks_reply(REP_SUCCEEDED)).await.is_err() {
                end_channel(channel).await;
                return;
            }
            relay(&mut socket, channel).await;
        }
        Err(error) => {
            tracing::debug!("socks request refused by the ssh host");
            let _ = socket.write_all(&socks_reply(socks_failure(&error))).await;
        }
    }
}

async fn start_remote(
    core: &Arc<ConnCore>,
    bind_address: String,
    bind_port: u16,
    dest_host: String,
    dest_port: u16,
) -> Result<Forward, ForwardError> {
    let address = if bind_address.trim().is_empty() {
        "127.0.0.1".to_owned()
    } else {
        bind_address.trim().to_owned()
    };
    let rules = core.remote_rules();
    let id = rules.next_id.fetch_add(1, Ordering::SeqCst);
    let state = ForwardState::new();
    // Registered before the request: the server may open a channel as soon
    // as it answered.
    rules.add(RemoteRule {
        id,
        address: address.clone(),
        port: u32::from(bind_port),
        dest_host,
        dest_port,
        state: Arc::clone(&state),
    });
    let requested = tokio::time::timeout(
        core.request_timeout(),
        core.handle()
            .tcpip_forward(address.clone(), u32::from(bind_port)),
    )
    .await;
    let port = match requested {
        Ok(Ok(assigned)) => {
            let port = if bind_port == 0 {
                u16::try_from(assigned).unwrap_or(0)
            } else {
                bind_port
            };
            rules.set_port(id, u32::from(port));
            port
        }
        Ok(Err(error)) => {
            rules.remove(id);
            return Err(match error {
                russh::Error::RequestDenied => ForwardError::Refused,
                _ if core.is_closed() => ForwardError::Disconnected,
                _ => ForwardError::Refused,
            });
        }
        Err(_) => {
            rules.remove(id);
            return Err(ForwardError::Timeout);
        }
    };
    // Ends the rule when the connection goes.
    let watch_core = Arc::clone(core);
    let watch_state = Arc::clone(&state);
    tokio::spawn(async move {
        let mut lost = watch_core.subscribe_lost();
        let mut stop = watch_state.stop.subscribe();
        tokio::select! {
            _ = set(&mut lost) => watch_state.finish(ForwardEnd::ConnectionLost),
            _ = set(&mut stop) => watch_state.finish(ForwardEnd::Stopped),
        }
    });
    Ok(Forward {
        state,
        bound: Bound::Remote(port),
        listener: None,
        remote: Some((Arc::clone(core), id, address)),
    })
}

/// Serves one accepted `forwarded-tcpip` channel of a Remote rule: connects
/// to the destination here and copies both ways until either side closes
/// or the rule stops.
pub(crate) async fn serve_remote_channel(
    channel: russh::Channel<russh::client::Msg>,
    dest_host: String,
    dest_port: u16,
    state: Arc<ForwardState>,
) {
    let _active = ActiveGuard::new(&state);
    let mut stop = state.stop.subscribe();
    let connect = tokio::time::timeout(
        DEST_CONNECT_TIMEOUT,
        TcpStream::connect((dest_host.as_str(), dest_port)),
    );
    let mut socket = match connect.await {
        Ok(Ok(socket)) => socket,
        _ => {
            tracing::debug!("remote forward destination unreachable");
            let _ = channel.close().await;
            return;
        }
    };
    let _ = socket.set_nodelay(true);
    // The relay first: it holds the channel's close guard once polled, so a
    // rule that stops right away still closes the channel.
    tokio::select! {
        biased;
        () = relay(&mut socket, channel) => {}
        _ = set(&mut stop) => {}
    }
}

impl Connection {
    /// Starts a forwarding rule on this connection.
    pub async fn forward(&self, spec: ForwardSpec) -> Result<Forward, ForwardError> {
        start(self.core(), spec).await
    }
}

// -- Rules of one host on one connection, with reconnects -----------------------------

/// Opens the connection of a [`HostForwarder`] (the desktop host resolves the
/// spec and the interaction; reconnects call it again).
pub type ConnectFn =
    Arc<dyn Fn() -> BoxFuture<'static, Result<Arc<Connection>, SshError>> + Send + Sync>;

/// The state of one rule of a [`HostForwarder`] (UI 8.2: running, stopped,
/// error).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RuleStatus {
    /// Connecting (first start or after a drop).
    Starting,
    Running(Bound),
    /// The rule could not start; it stays down until started again.
    Failed(ForwardError),
    /// The connection could not be opened.
    ConnectFailed(FailureKind),
    /// The connection was lost; a reconnect follows (`attempt` from 1).
    Reconnecting {
        attempt: u32,
    },
}

struct HostState {
    connection: Option<Arc<Connection>>,
    /// Rules that should run, by id, with their live handles.
    rules: BTreeMap<u64, (ForwardSpec, Option<Forward>)>,
    statuses: BTreeMap<u64, RuleStatus>,
    supervising: bool,
    /// The supervisor is reconnecting: new rules wait for it.
    reconnecting: Option<u32>,
}

/// Failures a reconnect attempt retries: the server could not be reached
/// or the connection was lost. Others (authentication, a host key, a
/// refused jump host) end the attempts: retrying them could trip the
/// server's intrusion detection, as for sessions (UI 4.6).
fn transport_failure(kind: FailureKind) -> bool {
    matches!(
        kind,
        FailureKind::NotFound
            | FailureKind::Refused
            | FailureKind::Timeout
            | FailureKind::Network
            | FailureKind::Disconnected
    )
}

/// Keeps the rules of one host running on one shared connection (UI 8:
/// several rules of a host use one SSH connection; after a reconnect the
/// rules come up again). Rules start in the order they were added.
///
/// The connection is opened outside the state lock (it may wait for a
/// host key decision or a password), and a rule started while the
/// supervisor reconnects waits for that reconnect.
pub struct HostForwarder {
    connect: ConnectFn,
    state: Arc<tokio::sync::Mutex<HostState>>,
    statuses: watch::Sender<BTreeMap<u64, RuleStatus>>,
    shutdown: watch::Sender<bool>,
}

impl std::fmt::Debug for HostForwarder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("HostForwarder { .. }")
    }
}

/// Starts every rule that has no live handle on `connection`.
async fn bring_up_waiting(
    state: &mut HostState,
    connection: &Arc<Connection>,
    statuses: &watch::Sender<BTreeMap<u64, RuleStatus>>,
) {
    let waiting: Vec<(u64, ForwardSpec)> = state
        .rules
        .iter()
        .filter(|(_, (_, forward))| forward.is_none())
        .map(|(id, (spec, _))| (*id, spec.clone()))
        .collect();
    for (id, spec) in waiting {
        let status = match connection.forward(spec).await {
            Ok(forward) => {
                let status = RuleStatus::Running(forward.bound());
                if let Some(entry) = state.rules.get_mut(&id) {
                    entry.1 = Some(forward);
                }
                status
            }
            Err(error) => RuleStatus::Failed(error),
        };
        state.statuses.insert(id, status);
    }
    statuses.send_replace(state.statuses.clone());
}

impl HostForwarder {
    pub fn new(connect: ConnectFn) -> Self {
        Self {
            connect,
            state: Arc::new(tokio::sync::Mutex::new(HostState {
                connection: None,
                rules: BTreeMap::new(),
                statuses: BTreeMap::new(),
                supervising: false,
                reconnecting: None,
            })),
            statuses: watch::Sender::new(BTreeMap::new()),
            shutdown: watch::Sender::new(false),
        }
    }

    /// The status of every rule, updated as they change.
    pub fn subscribe(&self) -> watch::Receiver<BTreeMap<u64, RuleStatus>> {
        self.statuses.subscribe()
    }

    /// The current status of rule `id`.
    pub fn status(&self, id: u64) -> Option<RuleStatus> {
        self.statuses.borrow().get(&id).cloned()
    }

    /// The shared connection, while there is one.
    pub async fn connection(&self) -> Option<Arc<Connection>> {
        self.state.lock().await.connection.clone()
    }

    /// Starts rule `id` (replacing a rule with that id), connecting first
    /// when there is no connection. Returns the rule's status.
    pub async fn start(&self, id: u64, spec: ForwardSpec) -> RuleStatus {
        let live = {
            let mut state = self.state.lock().await;
            if let Some((_, Some(old))) = state.rules.remove(&id) {
                old.stop().await;
            }
            state.rules.insert(id, (spec, None));
            if let Some(attempt) = state.reconnecting {
                let status = RuleStatus::Reconnecting { attempt };
                self.set_status(&mut state, id, status.clone());
                return status;
            }
            self.set_status(&mut state, id, RuleStatus::Starting);
            state
                .connection
                .clone()
                .filter(|connection| !connection.is_closed())
        };
        let connection = match live {
            Some(connection) => connection,
            None => match (self.connect)().await {
                Ok(connection) => connection,
                Err(error) => {
                    let mut state = self.state.lock().await;
                    let status = RuleStatus::ConnectFailed(error.kind);
                    if state.rules.contains_key(&id) {
                        self.set_status(&mut state, id, status.clone());
                    }
                    return status;
                }
            },
        };
        let mut state = self.state.lock().await;
        // While this start connected, the rule may have been stopped, the
        // forwarder shut down, or the supervisor begun a reconnect: then
        // this connection is not needed.
        if *self.shutdown.borrow() || !state.rules.contains_key(&id) {
            drop(state);
            connection.disconnect().await;
            return RuleStatus::ConnectFailed(FailureKind::Cancelled);
        }
        if let Some(attempt) = state.reconnecting {
            let status = RuleStatus::Reconnecting { attempt };
            self.set_status(&mut state, id, status.clone());
            drop(state);
            connection.disconnect().await;
            return status;
        }
        // Another start may have connected meanwhile: keep one connection.
        let connection = match &state.connection {
            Some(current) if !current.is_closed() && !Arc::ptr_eq(current, &connection) => {
                connection.disconnect().await;
                Arc::clone(current)
            }
            _ => {
                state.connection = Some(Arc::clone(&connection));
                connection
            }
        };
        bring_up_waiting(&mut state, &connection, &self.statuses).await;
        self.supervise(&mut state);
        state
            .statuses
            .get(&id)
            .cloned()
            .unwrap_or(RuleStatus::Starting)
    }

    /// Stops rule `id`; the connection closes with the last rule.
    pub async fn stop(&self, id: u64) {
        let mut state = self.state.lock().await;
        if let Some((_, Some(forward))) = state.rules.remove(&id) {
            forward.stop().await;
        }
        state.statuses.remove(&id);
        self.statuses.send_replace(state.statuses.clone());
        if state.rules.is_empty()
            && let Some(connection) = state.connection.take()
        {
            connection.disconnect().await;
        }
    }

    /// Stops every rule and closes the connection.
    pub async fn shutdown(&self) {
        self.shutdown.send_replace(true);
        let mut state = self.state.lock().await;
        let rules = std::mem::take(&mut state.rules);
        for (_, (_, forward)) in rules {
            if let Some(forward) = forward {
                forward.stop().await;
            }
        }
        state.statuses.clear();
        self.statuses.send_replace(BTreeMap::new());
        if let Some(connection) = state.connection.take() {
            connection.disconnect().await;
        }
    }

    fn set_status(&self, state: &mut HostState, id: u64, status: RuleStatus) {
        state.statuses.insert(id, status);
        self.statuses.send_replace(state.statuses.clone());
    }

    /// Starts the supervisor once: it waits for the connection it watches
    /// to drop (a connection replaced or closed meanwhile is not a drop)
    /// and brings every rule up again on a new one, with the reconnect
    /// backoff; a failure other than the transport's ends the attempts
    /// with `ConnectFailed`.
    fn supervise(&self, state: &mut HostState) {
        if state.supervising {
            return;
        }
        state.supervising = true;
        let shared = Arc::clone(&self.state);
        let statuses = self.statuses.clone();
        let connect = Arc::clone(&self.connect);
        let mut shutdown = self.shutdown.subscribe();
        tokio::spawn(async move {
            loop {
                let watched = {
                    let mut state = shared.lock().await;
                    match state.connection.clone() {
                        Some(connection) => connection,
                        None => {
                            state.supervising = false;
                            return;
                        }
                    }
                };
                tokio::select! {
                    _ = watched.lost_signal() => {}
                    _ = set(&mut shutdown) => return,
                }
                {
                    let mut state = shared.lock().await;
                    let current = state
                        .connection
                        .as_ref()
                        .is_some_and(|connection| Arc::ptr_eq(connection, &watched));
                    if !current {
                        // Replaced or closed by `stop`: watch what is there now.
                        continue;
                    }
                    state.connection = None;
                    if state.rules.is_empty() {
                        state.supervising = false;
                        return;
                    }
                    state.reconnecting = Some(1);
                    for (_, forward) in state.rules.values_mut() {
                        *forward = None;
                    }
                    let ids: Vec<u64> = state.rules.keys().copied().collect();
                    for id in ids {
                        state
                            .statuses
                            .insert(id, RuleStatus::Reconnecting { attempt: 1 });
                    }
                    statuses.send_replace(state.statuses.clone());
                }
                let mut backoff = Backoff::new();
                loop {
                    let delay = backoff.next_delay();
                    tokio::select! {
                        _ = tokio::time::sleep(delay) => {}
                        _ = set(&mut shutdown) => return,
                    }
                    let attempt = (connect)().await;
                    let mut state = shared.lock().await;
                    if state.rules.is_empty() {
                        if let Ok(connection) = attempt {
                            connection.disconnect().await;
                        }
                        state.reconnecting = None;
                        state.supervising = false;
                        return;
                    }
                    match attempt {
                        Ok(connection) => {
                            state.reconnecting = None;
                            // A live connection stored meanwhile wins.
                            let connection = match &state.connection {
                                Some(current) if !current.is_closed() => {
                                    connection.disconnect().await;
                                    Arc::clone(current)
                                }
                                _ => {
                                    state.connection = Some(Arc::clone(&connection));
                                    connection
                                }
                            };
                            bring_up_waiting(&mut state, &connection, &statuses).await;
                            break;
                        }
                        Err(error) if transport_failure(error.kind) => {
                            let next = backoff.attempt() + 1;
                            state.reconnecting = Some(next);
                            let ids: Vec<u64> = state.rules.keys().copied().collect();
                            for id in ids {
                                state
                                    .statuses
                                    .insert(id, RuleStatus::Reconnecting { attempt: next });
                            }
                            statuses.send_replace(state.statuses.clone());
                        }
                        Err(error) => {
                            state.reconnecting = None;
                            let ids: Vec<u64> = state.rules.keys().copied().collect();
                            for id in ids {
                                state
                                    .statuses
                                    .insert(id, RuleStatus::ConnectFailed(error.kind));
                            }
                            statuses.send_replace(state.statuses.clone());
                            state.supervising = false;
                            return;
                        }
                    }
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn handshake(bytes: &[u8]) -> (Result<SocksRequest, SocksRefusal>, Vec<u8>) {
        let (mut client, mut server) = tokio::io::duplex(4096);
        client.write_all(bytes).await.unwrap();
        // The client sends nothing more: a short request ends in EOF.
        client.shutdown().await.unwrap();
        let result = socks_handshake(&mut server).await;
        drop(server);
        let mut written = Vec::new();
        client.read_to_end(&mut written).await.unwrap();
        (result, written)
    }

    #[tokio::test]
    async fn socks_requests_keep_names_for_the_remote_side() {
        let mut bytes = vec![5, 1, METHOD_NONE, 5, CMD_CONNECT, 0, ATYP_DOMAIN, 7];
        bytes.extend_from_slice(b"db.lan.");
        bytes.extend_from_slice(&5432u16.to_be_bytes());
        let (result, written) = handshake(&bytes).await;
        assert_eq!(
            result,
            Ok(SocksRequest {
                host: "db.lan.".to_owned(),
                port: 5432
            })
        );
        assert_eq!(written, [5, METHOD_NONE]);
        let mut bytes = vec![
            5,
            2,
            2,
            METHOD_NONE,
            5,
            CMD_CONNECT,
            0,
            ATYP_IPV4,
            10,
            0,
            0,
            1,
        ];
        bytes.extend_from_slice(&80u16.to_be_bytes());
        let (result, _) = handshake(&bytes).await;
        assert_eq!(result.unwrap().host, "10.0.0.1");
        let mut bytes = vec![5, 1, METHOD_NONE, 5, CMD_CONNECT, 0, ATYP_IPV6];
        bytes.extend_from_slice(&std::net::Ipv6Addr::LOCALHOST.octets());
        bytes.extend_from_slice(&22u16.to_be_bytes());
        let (result, _) = handshake(&bytes).await;
        assert_eq!(result.unwrap().host, "::1");
    }

    #[tokio::test]
    async fn socks_refusals() {
        // Only username/password offered: no acceptable method.
        let (result, written) = handshake(&[5, 1, 2]).await;
        assert_eq!(result, Err(SocksRefusal::Close));
        assert_eq!(written, [5, METHOD_REFUSED]);
        // SOCKS4 is not spoken.
        let (result, written) = handshake(&[4, 1, 0, 80, 10, 0, 0, 1, 0]).await;
        assert_eq!(result, Err(SocksRefusal::Close));
        assert!(written.is_empty());
        // BIND is not supported.
        let mut bytes = vec![5, 1, METHOD_NONE, 5, 2, 0, ATYP_IPV4, 10, 0, 0, 1];
        bytes.extend_from_slice(&80u16.to_be_bytes());
        let (result, _) = handshake(&bytes).await;
        assert_eq!(result, Err(SocksRefusal::Reply(REP_COMMAND_NOT_SUPPORTED)));
        // An unknown address type.
        let (result, _) = handshake(&[5, 1, METHOD_NONE, 5, 1, 0, 9]).await;
        assert_eq!(result, Err(SocksRefusal::Reply(REP_ADDRESS_NOT_SUPPORTED)));
        // A name with control characters.
        let mut bytes = vec![
            5,
            1,
            METHOD_NONE,
            5,
            1,
            0,
            ATYP_DOMAIN,
            3,
            b'a',
            b'\n',
            b'b',
        ];
        bytes.extend_from_slice(&80u16.to_be_bytes());
        let (result, _) = handshake(&bytes).await;
        assert_eq!(result, Err(SocksRefusal::Reply(REP_HOST_UNREACHABLE)));
        // A truncated stream.
        let (result, _) = handshake(&[5]).await;
        assert_eq!(result, Err(SocksRefusal::Close));
    }

    #[test]
    fn bind_addresses_default_to_loopback() {
        assert_eq!(local_bind_ip(""), Ok(IpAddr::V4(Ipv4Addr::LOCALHOST)));
        assert_eq!(
            local_bind_ip("localhost"),
            Ok(IpAddr::V4(Ipv4Addr::LOCALHOST))
        );
        assert_eq!(
            local_bind_ip("0.0.0.0"),
            Ok(IpAddr::V4(Ipv4Addr::UNSPECIFIED))
        );
        assert_eq!(
            local_bind_ip("[::1]"),
            Ok(IpAddr::V6(std::net::Ipv6Addr::LOCALHOST))
        );
        assert_eq!(
            local_bind_ip("db.example"),
            Err(ForwardError::AddressUnavailable)
        );
    }

    #[tokio::test]
    async fn a_busy_port_names_the_port() {
        let taken = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = taken.local_addr().unwrap().port();
        let error = TcpListener::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port))
            .await
            .expect_err("in use");
        assert_eq!(bind_error(&error, port), ForwardError::PortBusy(port));
        assert_eq!(
            ForwardError::PortBusy(port).to_string(),
            format!("port {port} is already in use")
        );
    }

    #[test]
    fn remote_channels_match_only_active_rules() {
        let rules = RemoteRules::default();
        let state = ForwardState::new();
        rules.add(RemoteRule {
            id: 1,
            address: "127.0.0.1".to_owned(),
            port: 8080,
            dest_host: "localhost".to_owned(),
            dest_port: 3000,
            state: Arc::clone(&state),
        });
        assert!(rules.destination("127.0.0.1", 8080).is_some());
        assert!(rules.destination("127.0.0.1", 8081).is_none());
        assert!(rules.destination("0.0.0.0", 8080).is_none());
        state.stop.send_replace(true);
        assert!(rules.destination("127.0.0.1", 8080).is_none(), "stopped");
        rules.remove(1);
        assert!(rules.destination("127.0.0.1", 8080).is_none());
    }

    #[test]
    fn debug_output_has_no_addresses() {
        let spec = ForwardSpec {
            kind: ForwardKind::Local {
                host: "db.internal".to_owned(),
                port: 5432,
            },
            bind_address: "127.0.0.1".to_owned(),
            bind_port: 15432,
        };
        let text = format!("{spec:?}");
        assert!(!text.contains("db.internal") && !text.contains("15432"));
    }
}
