//! Connections, authentication and channels (docs/architecture.md, section
//! 4.2, steps 3-8).
//!
//! Flow of [`connect`]: resolve and open TCP (step `Resolving`, then
//! `Connecting`) -> SSH handshake; the server ident line is captured ->
//! [`Interaction::verify_host_key`] (step `HostKey`; nothing secret is sent
//! before it returns `Accept`) -> authentication (step `Authenticating`) in
//! the order of docs/data-model.md, section 8.6: the effective key
//! ([`AuthPlan::key`], decrypted with its passphrase or a `Passphrase`
//! prompt), the system agent's keys, the user's key files ([`FileKey`],
//! decrypted only after the server accepted the public key), the password
//! (or a `Password` prompt), keyboard-interactive (each question a
//! `KeyboardInteractive` prompt). A
//! missing username is asked first (`Username` prompt). The desktop host then
//! calls [`Connection::open_shell`] (step `OpeningSession` is the host's) and,
//! for OS detection, [`Connection::exec`].
//!
//! Timeouts: [`ConnectSpec::connect_timeout`] covers name resolution, the TCP
//! connection and the handshake up to the host key decision (the user's
//! decision time does not count); afterwards every server round trip
//! (authentication requests, channel requests) has the same limit.
//!
//! If the server closes the connection while the user decides about the host
//! key (for example its login grace time ran out), [`connect`] reconnects
//! once after an accept and accepts exactly that key again without asking
//! (docs/architecture.md, section 4.2, step 4).
//!
//! Keepalive: after authentication a task sends `keepalive@openssh.com`
//! requests every [`ConnectSpec::keepalive`] interval; three consecutive
//! unanswered intervals mark the connection lost and cut the transport, and
//! an open shell ends with [`SessionEnd::Dropped`]. (russh's own keepalive
//! timer is not used: before authentication it busy-polls once elapsed.)

use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Weak};
use std::time::Duration;

use russh::client::{Handle, Msg};
use russh::{ChannelMsg, ChannelReadHalf, ChannelWriteHalf, Disconnect, Sig};
use secrecy::{ExposeSecret, SecretString};
use tokio::sync::watch;
use tokio::time::Instant;

use crate::agent::SystemAgent;
use crate::auth::{AuthContext, AuthError};
use crate::error::{FailureKind, SshError};
use crate::handler::{ClientHandler, Shared};
use crate::transport::{KillSwitch, KillableStream};

/// A boxed, sendable future (dyn-compatible async trait methods without an extra crate).
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Credentials to try, resolved by the host from the core (docs/data-model.md,
/// section 8.3) or typed at prompts.
#[derive(Clone, Default)]
pub struct AuthPlan {
    /// The effective key of the host (stage 2: `ssh.key_id`, inherited or own).
    pub key: Option<KeyCredential>,
    /// Try the keys of [`ConnectSpec::agent`] (stage 2: always when an agent exists).
    pub use_agent: bool,
    /// The effective password (`ssh.password`), if saved.
    pub password: Option<SecretString>,
    /// Try keyboard-interactive when the server offers it.
    pub keyboard_interactive: bool,
    /// Prompts may offer "Save" for a password (saved personal-vault host).
    pub can_save_password: bool,
    /// Prompts may offer "Save" for the key passphrase (writable key).
    pub can_save_passphrase: bool,
    /// The user's key files (`~/.ssh/id_*`), tried after the agent's keys
    /// and before the password, in this order (see [`FileKey`]).
    pub file_keys: Vec<FileKey>,
}

impl std::fmt::Debug for AuthPlan {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuthPlan")
            .field("key", &self.key.is_some())
            .field("use_agent", &self.use_agent)
            .field("password", &self.password.is_some())
            .field("keyboard_interactive", &self.keyboard_interactive)
            .field("file_keys", &self.file_keys.len())
            .finish_non_exhaustive()
    }
}

/// A private key file of the user (`~/.ssh/id_*`). It is offered by its
/// public key first and decrypted only after the server accepted that key,
/// so a passphrase is asked only for a key the server would take (the
/// `FileKeyPassphrase` prompt; a wrong passphrase is asked again).
#[derive(Clone)]
pub struct FileKey {
    /// Shown in the passphrase prompt: the file name, for example `id_ed25519`.
    pub label: String,
    /// The private key text, possibly encrypted.
    pub private_key: SecretString,
    /// The public key line of the `.pub` file next to it, if any. Used when
    /// the private key text does not carry its public key (legacy PEM keys).
    pub public_key: Option<String>,
    /// A passphrase remembered earlier, tried before asking.
    pub passphrase: Option<SecretString>,
}

impl std::fmt::Debug for FileKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FileKey")
            .field("passphrase", &self.passphrase.is_some())
            .finish_non_exhaustive()
    }
}

/// A private key from a `key` record.
#[derive(Clone)]
pub struct KeyCredential {
    /// The key record's label, shown in the passphrase prompt.
    pub label: String,
    /// OpenSSH private key text, possibly encrypted.
    pub private_key: SecretString,
    /// The saved passphrase, if any.
    pub passphrase: Option<SecretString>,
}

impl std::fmt::Debug for KeyCredential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("KeyCredential { .. }")
    }
}

/// Everything needed to open one connection.
#[derive(Clone)]
pub struct ConnectSpec {
    /// Host name or IP address (IPv6 without brackets).
    pub address: String,
    pub port: u16,
    /// `None`: ask with a `Username` prompt.
    pub username: Option<String>,
    pub auth: AuthPlan,
    /// The system agent for authentication and forwarding, if any.
    pub agent: Option<SystemAgent>,
    /// SSH keepalive interval; `None` = off. Three unanswered keepalives drop the connection.
    pub keepalive: Option<Duration>,
    /// TCP connect plus handshake timeout (the host passes 15 s).
    pub connect_timeout: Duration,
    /// Host key algorithms already saved for this host in known hosts, most
    /// preferred first, so that the server presents a key the store knows.
    pub preferred_host_key_types: Vec<String>,
}

impl std::fmt::Debug for ConnectSpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConnectSpec")
            .field("auth", &self.auth)
            .field("agent", &self.agent.is_some())
            .field("keepalive", &self.keepalive)
            .finish_non_exhaustive()
    }
}

/// The host key the server presented.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostKeyInfo {
    /// Algorithm name, for example `ssh-ed25519`.
    pub key_type: String,
    /// Base64 key blob as in a known_hosts line.
    pub public_key_base64: String,
    /// `SHA256:` fingerprint as OpenSSH prints it.
    pub fingerprint: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostKeyDecision {
    Accept,
    Reject,
}

/// Connection steps shown on the connection screen (UI 4.8).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConnectStep {
    Resolving,
    Connecting,
    HostKey,
    Authenticating,
}

/// A value the user must provide (UI 4.8). Texts from the server (`name`,
/// `instruction`, `prompt`) are untrusted and shown as plain text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Prompt {
    Username,
    /// `error`: the previous password was refused.
    Password {
        username: String,
        error: bool,
        can_save: bool,
    },
    /// `error`: the previous passphrase did not open the key.
    Passphrase {
        key_label: String,
        error: bool,
        can_save: bool,
    },
    /// The passphrase of a [`FileKey`] the server accepted. `error`: the
    /// previous passphrase did not open it. The answer's `save` means
    /// "remember while the app runs" (key files have no record to save to).
    FileKeyPassphrase {
        key_label: String,
        error: bool,
    },
    /// One keyboard-interactive question (asked one at a time).
    KeyboardInteractive {
        name: String,
        instruction: String,
        prompt: String,
        echo: bool,
        /// 0-based question index and the number of questions in this round.
        index: u32,
        count: u32,
        /// The previous round was refused.
        error: bool,
    },
}

/// The user's answer; `save` is the "Save" checkbox (always `false` for
/// usernames and keyboard-interactive answers).
pub struct PromptAnswer {
    pub value: SecretString,
    pub save: bool,
}

impl std::fmt::Debug for PromptAnswer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PromptAnswer")
            .field("save", &self.save)
            .finish_non_exhaustive()
    }
}

/// Callbacks of one connection attempt, implemented by the desktop host.
pub trait Interaction: Send + Sync {
    /// Progress for the connection screen.
    fn step(&self, step: ConnectStep);
    /// Decides the presented host key (known hosts lookup, TOFU dialog,
    /// changed or revoked key). Called once per handshake.
    fn verify_host_key<'a>(&'a self, key: &'a HostKeyInfo) -> BoxFuture<'a, HostKeyDecision>;
    /// Asks the user. `None` = cancelled (the attempt fails with `Cancelled`).
    fn prompt<'a>(&'a self, prompt: Prompt) -> BoxFuture<'a, Option<PromptAnswer>>;
    /// The server's authentication banner (untrusted text).
    fn banner(&self, _text: &str) {}
}

/// How authentication succeeded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthMethod {
    /// `none` was accepted.
    None,
    PublicKey,
    Agent,
    Password,
    KeyboardInteractive,
}

/// The result of authentication.
pub struct AuthOutcome {
    /// The username used (from the spec or the `Username` prompt).
    pub username: String,
    pub method: AuthMethod,
}

impl std::fmt::Debug for AuthOutcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuthOutcome")
            .field("method", &self.method)
            .finish_non_exhaustive()
    }
}

/// Secrets typed at prompts during a successful authentication. A
/// [`Connection`] hands them out once ([`Connection::take_prompted_secrets`]):
/// afterwards the host holds the only copy, which it wipes on lock.
#[derive(Default)]
pub struct PromptedSecrets {
    /// A password typed at a prompt and accepted by the server (kept by the
    /// host for reconnects while unlocked; saved when `save` is set).
    pub password: Option<PromptAnswer>,
    /// A passphrase typed at a prompt that opened the key whose signature
    /// the server accepted.
    pub passphrase: Option<PromptAnswer>,
    /// A passphrase typed at a `FileKeyPassphrase` prompt that opened the
    /// file key at this index of [`AuthPlan::file_keys`], whose signature
    /// the server accepted.
    pub file_passphrase: Option<(usize, PromptAnswer)>,
}

impl std::fmt::Debug for PromptedSecrets {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PromptedSecrets")
            .field("password", &self.password.is_some())
            .field("passphrase", &self.passphrase.is_some())
            .field(
                "file_passphrase",
                &self.file_passphrase.as_ref().map(|(index, _)| *index),
            )
            .finish()
    }
}

/// The SSH identification string this client sends.
const CLIENT_ID: &str = concat!("SSH-2.0-shellpp_", env!("CARGO_PKG_VERSION"));
/// Lower bound of the per-request timeout after the handshake.
const MIN_REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
/// Keepalive intervals without an answer before the connection is lost.
const KEEPALIVE_MAX_MISSES: u32 = 3;
/// After an exit status, how long the channel may stay open before the
/// shell is reported ended anyway.
const EXIT_GRACE: Duration = Duration::from_secs(2);
/// How long [`Connection::disconnect`] waits for an orderly close.
const DISCONNECT_GRACE: Duration = Duration::from_secs(1);
/// After the last reference to a connection is dropped, how long the
/// transport may take to close in order before it is cut.
const DROP_GRACE: Duration = Duration::from_secs(1);

/// Why one attempt of [`connect`] failed.
enum AttemptError {
    /// The transport closed before the first authentication reply, after the
    /// user accepted this host key: reconnect once.
    LostAfterAccept(HostKeyInfo),
    Failed(FailureKind),
}

impl From<FailureKind> for AttemptError {
    fn from(kind: FailureKind) -> Self {
        Self::Failed(kind)
    }
}

/// Opens a connection: TCP, handshake, host key decision, authentication.
pub async fn connect(
    spec: ConnectSpec,
    interaction: Arc<dyn Interaction>,
) -> Result<Connection, SshError> {
    let mut username = spec
        .username
        .clone()
        .map(|user| user.trim().to_owned())
        .filter(|user| !user.is_empty());
    let mut auto_accept: Option<HostKeyInfo> = None;
    let mut reconnected = false;
    loop {
        match attempt(&spec, &interaction, auto_accept.take(), &mut username).await {
            Ok(connection) => return Ok(connection),
            Err(AttemptError::LostAfterAccept(key)) if !reconnected => {
                tracing::debug!(
                    "connection closed during the host key decision; reconnecting once"
                );
                reconnected = true;
                auto_accept = Some(key);
            }
            Err(AttemptError::LostAfterAccept(_)) => {
                return Err(SshError::new(FailureKind::Disconnected));
            }
            Err(AttemptError::Failed(kind)) => {
                tracing::debug!(kind = kind.reason(), "connect failed");
                return Err(SshError::new(kind));
            }
        }
    }
}

fn client_config(spec: &ConnectSpec) -> russh::client::Config {
    russh::client::Config {
        client_id: russh::SshId::Standard(std::borrow::Cow::Borrowed(CLIENT_ID)),
        preferred: crate::algorithms::preferred(&spec.preferred_host_key_types),
        // Own keepalive task after authentication (module documentation).
        keepalive_interval: None,
        inactivity_timeout: None,
        nodelay: true,
        ..Default::default()
    }
}

fn request_timeout(spec: &ConnectSpec) -> Duration {
    spec.connect_timeout.max(MIN_REQUEST_TIMEOUT)
}

async fn attempt(
    spec: &ConnectSpec,
    interaction: &Arc<dyn Interaction>,
    auto_accept: Option<HostKeyInfo>,
    username: &mut Option<String>,
) -> Result<Connection, AttemptError> {
    let deadline = Instant::now() + spec.connect_timeout;
    let shared = Arc::new(Shared::new(
        Arc::clone(interaction),
        auto_accept,
        spec.agent.clone(),
    ));

    interaction.step(ConnectStep::Resolving);
    let addrs = tokio::time::timeout_at(
        deadline,
        crate::transport::resolve(&spec.address, spec.port),
    )
    .await
    .map_err(|_| FailureKind::Timeout)??;

    interaction.step(ConnectStep::Connecting);
    let tcp = tokio::time::timeout_at(deadline, crate::transport::connect_any(&addrs, deadline))
        .await
        .map_err(|_| FailureKind::Timeout)??;

    let switch = Arc::new(KillSwitch::default());
    let stream = KillableStream::new(tcp, Arc::clone(&switch));
    // Dropping this future (the host cancelled the attempt) ends a pending
    // host key decision and cuts the transport.
    let mut guard = AttemptGuard {
        shared: Arc::clone(&shared),
        switch: Arc::clone(&switch),
        armed: true,
    };
    let config = Arc::new(client_config(spec));
    let handshake =
        russh::client::connect_stream(config, stream, ClientHandler::new(Arc::clone(&shared)));
    tokio::pin!(handshake);
    let mut verifying = shared.subscribe_verifying();
    let handshake_result = loop {
        let decision_started = *verifying.borrow();
        tokio::select! {
            result = &mut handshake => break result,
            _ = tokio::time::sleep_until(deadline), if !decision_started => {
                switch.kill();
                return Err(FailureKind::Timeout.into());
            }
            _ = verifying.changed(), if !decision_started => {}
        }
    };
    let mut handle = match handshake_result {
        Ok(handle) => handle,
        Err(error) => {
            switch.kill();
            if shared.host_key_decision() == Some(HostKeyDecision::Reject) {
                return Err(FailureKind::HostKeyRejected.into());
            }
            return Err(FailureKind::from_russh(&error).into());
        }
    };

    interaction.step(ConnectStep::Authenticating);
    let timeout = request_timeout(spec);
    let context = AuthContext {
        plan: &spec.auth,
        agent: spec.agent.as_ref(),
        interaction: interaction.as_ref(),
        timeout,
    };
    let authenticated = match crate::auth::authenticate(&mut handle, &context, username).await {
        Ok(authenticated) => authenticated,
        Err(error) => {
            let reason = match error {
                AuthError::Failed(FailureKind::Cancelled) => Disconnect::AuthCancelledByUser,
                _ => Disconnect::ByApplication,
            };
            let _ = handle.disconnect(reason, "", "en").await;
            switch.kill();
            return Err(match error {
                AuthError::LostBeforeFirstReply => match shared.key_accepted_by_user() {
                    Some(key) => AttemptError::LostAfterAccept(key),
                    None => AttemptError::Failed(FailureKind::Disconnected),
                },
                AuthError::Failed(kind) => AttemptError::Failed(kind),
            });
        }
    };

    guard.armed = false;
    let server_ident = shared.server_ident();
    let core = Arc::new(ConnCore {
        handle,
        shared,
        switch,
        request_timeout: timeout,
        keepalive: std::sync::Mutex::new(None),
    });
    if let Some(interval) = spec.keepalive.filter(|interval| !interval.is_zero()) {
        let task = tokio::spawn(keepalive_loop(Arc::downgrade(&core), interval));
        if let Ok(mut slot) = core.keepalive.lock() {
            *slot = Some(task);
        }
    }
    Ok(Connection {
        core,
        server_ident,
        auth: authenticated.outcome,
        prompted: std::sync::Mutex::new(authenticated.prompted),
    })
}

/// Cancels an unfinished attempt when dropped (see [`attempt`]).
struct AttemptGuard {
    shared: Arc<Shared>,
    switch: Arc<KillSwitch>,
    armed: bool,
}

impl Drop for AttemptGuard {
    fn drop(&mut self) {
        if self.armed {
            self.shared.cancel();
            self.switch.kill();
        }
    }
}

/// Sends a keepalive every `interval` while the connection exists (an
/// answer has the rest of the interval to arrive); marks the connection
/// lost (`Timeout`) and cuts the transport after [`KEEPALIVE_MAX_MISSES`]
/// consecutive unanswered keepalives, about three intervals after the link
/// died.
async fn keepalive_loop(core: Weak<ConnCore>, interval: Duration) {
    let mut tick = tokio::time::interval_at(Instant::now() + interval, interval);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut misses = 0u32;
    loop {
        tick.tick().await;
        let Some(strong) = core.upgrade() else {
            return;
        };
        if strong.is_closed() {
            return;
        }
        match tokio::time::timeout(interval, strong.handle.send_ping()).await {
            Ok(Ok(())) => misses = 0,
            Ok(Err(_)) => return,
            Err(_) => {
                misses += 1;
                tracing::debug!(misses, "keepalive unanswered");
                if misses >= KEEPALIVE_MAX_MISSES {
                    tracing::debug!("keepalive timeout; connection lost");
                    strong.shared.mark_lost_with(FailureKind::Timeout);
                    strong.switch.kill();
                    return;
                }
            }
        }
    }
}

/// What a connection and its channels share.
struct ConnCore {
    handle: Handle<ClientHandler>,
    shared: Arc<Shared>,
    switch: Arc<KillSwitch>,
    request_timeout: Duration,
    keepalive: std::sync::Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl ConnCore {
    fn is_closed(&self) -> bool {
        self.handle.is_closed()
            || self.shared.transport_closed()
            || self.shared.is_lost()
            || self.switch.is_killed()
    }

    /// The error of a failed request: `Disconnected` when the transport is
    /// gone, else `fallback`.
    fn failure(&self, fallback: FailureKind) -> SshError {
        if self.is_closed() {
            SshError::new(FailureKind::Disconnected)
        } else {
            SshError::new(fallback)
        }
    }

    async fn open_session_channel(&self) -> Result<russh::Channel<Msg>, SshError> {
        match tokio::time::timeout(self.request_timeout, self.handle.channel_open_session()).await {
            Ok(Ok(channel)) => Ok(channel),
            Ok(Err(_)) => Err(self.failure(FailureKind::ChannelFailed)),
            Err(_) => Err(SshError::new(FailureKind::Timeout)),
        }
    }
}

impl Drop for ConnCore {
    fn drop(&mut self) {
        if let Ok(mut slot) = self.keepalive.lock()
            && let Some(task) = slot.take()
        {
            task.abort();
        }
        // Nothing can use the connection any more. The russh session ends by
        // itself once its handle and channels are gone; cutting the stream a
        // little later also ends it when a write is stuck on a dead link.
        let switch = Arc::clone(&self.switch);
        match tokio::runtime::Handle::try_current() {
            Ok(runtime) => {
                runtime.spawn(async move {
                    tokio::time::sleep(DROP_GRACE).await;
                    switch.kill();
                });
            }
            Err(_) => switch.kill(),
        }
    }
}

/// An authenticated connection.
pub struct Connection {
    core: Arc<ConnCore>,
    server_ident: String,
    auth: AuthOutcome,
    /// Taken once by [`Connection::take_prompted_secrets`].
    prompted: std::sync::Mutex<PromptedSecrets>,
}

impl std::fmt::Debug for Connection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Connection { .. }")
    }
}

/// PTY and session setup of [`Connection::open_shell`] (architecture 4.2, step 6).
#[derive(Clone)]
pub struct ShellRequest {
    /// TERM, for example `xterm-256color`.
    pub term: String,
    pub cols: u32,
    pub rows: u32,
    /// Environment variables sent as `env` requests before the shell.
    pub env: Vec<(String, SecretString)>,
    /// Request agent forwarding (needs [`ConnectSpec::agent`]).
    pub agent_forwarding: bool,
}

impl std::fmt::Debug for ShellRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ShellRequest")
            .field("cols", &self.cols)
            .field("rows", &self.rows)
            .field("env", &self.env.len())
            .field("agent_forwarding", &self.agent_forwarding)
            .finish_non_exhaustive()
    }
}

/// Output of an exec channel (OS detection).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExecOutput {
    /// Stdout, cut at the requested maximum.
    pub stdout: Vec<u8>,
    pub exit_status: Option<u32>,
}

/// Most bytes of channel data (stdout and stderr) a server may send on a
/// channel before it answers the requests that set the channel up (pty,
/// agent forwarding, shell, exec). Real servers send none. Until the shell
/// is open nothing reads this data, and russh widens the window as data
/// arrives, so without a cap a server could fill memory at link speed.
const MAX_EARLY_DATA: usize = 1024 * 1024;
/// Most messages kept while waiting for those answers (the same reason).
const MAX_EARLY_MESSAGES: usize = 4096;

/// Waits for the reply to a channel request sent with `want_reply`. Other
/// messages that arrive meanwhile are kept in `pending`, in order. More
/// than [`MAX_EARLY_DATA`] bytes of data or [`MAX_EARLY_MESSAGES`]
/// messages in `pending` close the channel with [`FailureKind::Protocol`].
async fn wait_reply(
    channel: &mut russh::Channel<Msg>,
    pending: &mut VecDeque<ChannelMsg>,
    timeout: Duration,
) -> Result<bool, FailureKind> {
    let deadline = Instant::now() + timeout;
    let mut data_bytes: usize = pending.iter().map(data_len).sum();
    loop {
        match tokio::time::timeout_at(deadline, channel.wait()).await {
            Err(_) => return Err(FailureKind::Timeout),
            Ok(None) => return Err(FailureKind::Disconnected),
            Ok(Some(ChannelMsg::Success)) => return Ok(true),
            Ok(Some(ChannelMsg::Failure)) => return Ok(false),
            Ok(Some(ChannelMsg::Close)) => return Err(FailureKind::ChannelFailed),
            Ok(Some(other)) => {
                data_bytes = data_bytes.saturating_add(data_len(&other));
                if data_bytes > MAX_EARLY_DATA || pending.len() >= MAX_EARLY_MESSAGES {
                    tracing::debug!("too much channel output before the reply");
                    let _ = channel.close().await;
                    return Err(FailureKind::Protocol);
                }
                pending.push_back(other);
            }
        }
    }
}

/// The payload size of a data message (0 for other messages).
fn data_len(message: &ChannelMsg) -> usize {
    match message {
        ChannelMsg::Data { data } | ChannelMsg::ExtendedData { data, .. } => data.len(),
        _ => 0,
    }
}

/// Clamps a terminal dimension to what the protocol and servers accept.
fn dimension(value: u32) -> u32 {
    value.clamp(1, 10_000)
}

impl Connection {
    /// The server identification line, for example
    /// `SSH-2.0-OpenSSH_9.6p1 Ubuntu-3ubuntu13` (OS detection from the banner).
    pub fn server_ident(&self) -> &str {
        &self.server_ident
    }

    pub fn auth(&self) -> &AuthOutcome {
        &self.auth
    }

    /// The secrets typed at prompts during authentication, handed out once:
    /// later calls return none. The connection keeps no copy, so a host
    /// that wipes its copy on lock leaves none in memory (docs/security.md,
    /// section 9.5).
    pub fn take_prompted_secrets(&self) -> PromptedSecrets {
        match self.prompted.lock() {
            Ok(mut prompted) => std::mem::take(&mut *prompted),
            Err(poisoned) => std::mem::take(&mut *poisoned.into_inner()),
        }
    }

    /// Session channel with a PTY, env requests, agent forwarding if
    /// requested, then the shell.
    pub async fn open_shell(&self, request: ShellRequest) -> Result<Shell, SshError> {
        let core = &self.core;
        let timeout = core.request_timeout;
        let mut channel = core.open_session_channel().await?;
        let mut pending = VecDeque::new();

        channel
            .request_pty(
                true,
                &request.term,
                dimension(request.cols),
                dimension(request.rows),
                0,
                0,
                &[],
            )
            .await
            .map_err(|_| core.failure(FailureKind::ChannelFailed))?;
        if !wait_reply(&mut channel, &mut pending, timeout)
            .await
            .map_err(|kind| core.failure(kind))?
        {
            tracing::debug!("pty request refused");
            return Err(SshError::new(FailureKind::ChannelFailed));
        }

        for (name, value) in &request.env {
            // Servers refuse names they do not accept (OpenSSH `AcceptEnv`);
            // refusals are ignored, so no reply is requested.
            channel
                .set_env(false, name.as_str(), value.expose_secret())
                .await
                .map_err(|_| core.failure(FailureKind::ChannelFailed))?;
        }

        if request.agent_forwarding && core.shared.agent().is_some() {
            // Allowed before the request goes out: the server may open the
            // agent channel as soon as it accepted.
            core.shared.set_forwarding_requested(true);
            channel
                .agent_forward(true)
                .await
                .map_err(|_| core.failure(FailureKind::ChannelFailed))?;
            // A refused (or unanswered) forwarding request does not stop the
            // shell; only a lost connection does.
            match wait_reply(&mut channel, &mut pending, timeout).await {
                Ok(true) => {}
                Ok(false) => {
                    tracing::debug!("agent forwarding refused by the server");
                    core.shared.set_forwarding_requested(false);
                }
                Err(kind @ (FailureKind::Disconnected | FailureKind::Protocol)) => {
                    return Err(SshError::new(kind));
                }
                Err(kind) => {
                    tracing::debug!(kind = kind.reason(), "agent forwarding not confirmed");
                    core.shared.set_forwarding_requested(false);
                }
            }
        }

        channel
            .request_shell(true)
            .await
            .map_err(|_| core.failure(FailureKind::ChannelFailed))?;
        if !wait_reply(&mut channel, &mut pending, timeout)
            .await
            .map_err(|kind| core.failure(kind))?
        {
            tracing::debug!("shell request refused");
            return Err(SshError::new(FailureKind::ChannelFailed));
        }

        let (read, write) = channel.split();
        let writer = ShellWriter {
            inner: Arc::new(WriterInner {
                write,
                core: Arc::clone(core),
                ended: watch::Sender::new(false),
            }),
        };
        Ok(Shell {
            read,
            pending,
            writer,
            lost: core.shared.subscribe_lost(),
            exit: None,
            exit_at: None,
            ended: None,
        })
    }

    /// Runs `command` in a separate exec channel (never in the shell), with
    /// a timeout and an output limit; used for OS detection (UI 12.4).
    pub async fn exec(
        &self,
        command: &str,
        timeout: Duration,
        max_output: usize,
    ) -> Result<ExecOutput, SshError> {
        let core = &self.core;
        let deadline = Instant::now() + timeout;
        let mut channel = match tokio::time::timeout_at(deadline, core.open_session_channel()).await
        {
            Ok(result) => result?,
            Err(_) => return Err(SshError::new(FailureKind::Timeout)),
        };
        let mut pending = VecDeque::new();
        channel
            .exec(true, command)
            .await
            .map_err(|_| core.failure(FailureKind::ChannelFailed))?;
        let remaining = deadline.saturating_duration_since(Instant::now());
        match wait_reply(&mut channel, &mut pending, remaining).await {
            Ok(true) => {}
            Ok(false) => {
                let _ = channel.close().await;
                return Err(SshError::new(FailureKind::ChannelFailed));
            }
            Err(FailureKind::Timeout) => {
                let _ = channel.close().await;
                return Err(SshError::new(FailureKind::Timeout));
            }
            Err(kind) => return Err(core.failure(kind)),
        }

        let mut output = ExecOutput {
            stdout: Vec::new(),
            exit_status: None,
        };
        let mut eof = false;
        loop {
            let message = match pending.pop_front() {
                Some(message) => Some(message),
                None => match tokio::time::timeout_at(deadline, channel.wait()).await {
                    Ok(message) => message,
                    Err(_) => break,
                },
            };
            match message {
                Some(ChannelMsg::Data { data }) => {
                    let room = max_output.saturating_sub(output.stdout.len());
                    output
                        .stdout
                        .extend_from_slice(&data[..data.len().min(room)]);
                }
                Some(ChannelMsg::ExitStatus { exit_status }) => {
                    output.exit_status = Some(exit_status);
                    if eof {
                        break;
                    }
                }
                Some(ChannelMsg::Eof) => {
                    eof = true;
                    if output.exit_status.is_some() {
                        break;
                    }
                }
                Some(ChannelMsg::Close) | None => break,
                Some(_) => {}
            }
        }
        let _ = channel.close().await;
        Ok(output)
    }

    /// `true` once the transport is gone.
    pub fn is_closed(&self) -> bool {
        self.core.is_closed()
    }

    /// Sends an SSH disconnect and closes the transport.
    pub async fn disconnect(&self) {
        let core = &self.core;
        let _ = core
            .handle
            .disconnect(Disconnect::ByApplication, "", "en")
            .await;
        let deadline = Instant::now() + DISCONNECT_GRACE;
        while !core.handle.is_closed() && Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        core.switch.kill();
        if let Ok(mut slot) = core.keepalive.lock()
            && let Some(task) = slot.take()
        {
            task.abort();
        }
    }
}

/// How a shell ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SessionEnd {
    /// The server sent an exit status (0 = normal end, UI 4.6).
    ExitStatus(u32),
    /// The server sent an exit signal (shown like a non-zero status).
    ExitSignal(String),
    /// The channel closed without an exit status: a drop (UI 4.6).
    ClosedWithoutStatus,
    /// Transport error or keepalive timeout: a drop.
    Dropped(FailureKind),
}

/// Events of an open shell, in order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ShellEvent {
    /// Output bytes (stdout and stderr in arrival order).
    Data(Vec<u8>),
    /// The shell ended; no event follows.
    End(SessionEnd),
}

/// An open shell channel.
pub struct Shell {
    read: ChannelReadHalf,
    /// Messages received while the setup requests were answered.
    pending: VecDeque<ChannelMsg>,
    writer: ShellWriter,
    lost: watch::Receiver<bool>,
    /// Exit status or signal received; the end is reported at the channel
    /// close (or [`EXIT_GRACE`] later).
    exit: Option<SessionEnd>,
    exit_at: Option<Instant>,
    ended: Option<SessionEnd>,
}

impl std::fmt::Debug for Shell {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Shell { .. }")
    }
}

fn signal_name(signal: &Sig) -> String {
    let name = match signal {
        Sig::ABRT => "ABRT",
        Sig::ALRM => "ALRM",
        Sig::FPE => "FPE",
        Sig::HUP => "HUP",
        Sig::ILL => "ILL",
        Sig::INT => "INT",
        Sig::KILL => "KILL",
        Sig::PIPE => "PIPE",
        Sig::QUIT => "QUIT",
        Sig::SEGV => "SEGV",
        Sig::TERM => "TERM",
        Sig::USR1 => "USR1",
        Sig::Custom(name) => name.as_str(),
    };
    // Server text: keep it short and printable.
    name.chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-' || *c == '@' || *c == '.')
        .take(32)
        .collect()
}

impl Shell {
    /// A handle for input and resizes; cheap to clone.
    pub fn writer(&self) -> ShellWriter {
        self.writer.clone()
    }

    /// The next event; after `End` it keeps returning `End`.
    pub async fn next(&mut self) -> ShellEvent {
        loop {
            if let Some(end) = &self.ended {
                return ShellEvent::End(end.clone());
            }
            let message = match self.pending.pop_front() {
                Some(message) => Incoming::Message(Some(message)),
                None => self.receive().await,
            };
            match message {
                Incoming::Message(Some(ChannelMsg::Data { data }))
                | Incoming::Message(Some(ChannelMsg::ExtendedData { data, .. })) => {
                    if !data.is_empty() {
                        return ShellEvent::Data(data.to_vec());
                    }
                }
                Incoming::Message(Some(ChannelMsg::ExitStatus { exit_status })) => {
                    self.set_exit(SessionEnd::ExitStatus(exit_status));
                }
                Incoming::Message(Some(ChannelMsg::ExitSignal {
                    signal_name: signal,
                    ..
                })) => {
                    self.set_exit(SessionEnd::ExitSignal(signal_name(&signal)));
                }
                Incoming::Message(Some(ChannelMsg::Close)) => {
                    self.finish(SessionEnd::ClosedWithoutStatus);
                }
                Incoming::Message(None) | Incoming::Lost => {
                    let reason = self.writer.inner.core.shared.lost_reason();
                    self.finish(SessionEnd::Dropped(reason));
                }
                Incoming::ExitGraceOver => {
                    self.finish(SessionEnd::ClosedWithoutStatus);
                }
                // EOF: the exit status (if any) and the close follow.
                Incoming::Message(Some(_)) => {}
            }
        }
    }

    fn set_exit(&mut self, end: SessionEnd) {
        if self.exit.is_none() {
            self.exit = Some(end);
            self.exit_at = Some(Instant::now() + EXIT_GRACE);
        }
    }

    /// Ends the shell: a received exit status or signal wins over `fallback`
    /// (an exit status is never a drop, D18).
    fn finish(&mut self, fallback: SessionEnd) {
        let end = self.exit.take().unwrap_or(fallback);
        tracing::debug!(
            end = match &end {
                SessionEnd::ExitStatus(_) => "exit_status",
                SessionEnd::ExitSignal(_) => "exit_signal",
                SessionEnd::ClosedWithoutStatus => "closed_without_status",
                SessionEnd::Dropped(_) => "dropped",
            },
            "shell ended"
        );
        self.ended = Some(end);
        self.writer.inner.ended.send_replace(true);
    }

    async fn receive(&mut self) -> Incoming {
        let exit_at = self.exit_at;
        let lost = &mut self.lost;
        tokio::select! {
            biased;
            message = self.read.wait() => Incoming::Message(message),
            _ = lost.wait_for(|lost| *lost) => Incoming::Lost,
            _ = async {
                match exit_at {
                    Some(at) => tokio::time::sleep_until(at).await,
                    None => std::future::pending::<()>().await,
                }
            } => Incoming::ExitGraceOver,
        }
    }
}

enum Incoming {
    Message(Option<ChannelMsg>),
    Lost,
    ExitGraceOver,
}

/// Input side of a [`Shell`].
#[derive(Clone)]
pub struct ShellWriter {
    inner: Arc<WriterInner>,
}

struct WriterInner {
    write: ChannelWriteHalf<Msg>,
    core: Arc<ConnCore>,
    /// Set when the [`Shell`] reported its end: a write waiting for channel
    /// window space then fails instead of waiting forever.
    ended: watch::Sender<bool>,
}

impl std::fmt::Debug for ShellWriter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ShellWriter { .. }")
    }
}

impl ShellWriter {
    pub async fn write(&self, data: &[u8]) -> Result<(), SshError> {
        if data.is_empty() {
            return Ok(());
        }
        let inner = &self.inner;
        if inner.core.is_closed() {
            return Err(SshError::new(FailureKind::Disconnected));
        }
        if *inner.ended.borrow() {
            return Err(SshError::new(FailureKind::ChannelFailed));
        }
        let mut ended = inner.ended.subscribe();
        let mut lost = inner.core.shared.subscribe_lost();
        tokio::select! {
            result = inner.write.data_bytes(data.to_vec()) => {
                result.map_err(|_| inner.core.failure(FailureKind::ChannelFailed))
            }
            _ = ended.wait_for(|ended| *ended) => Err(inner.core.failure(FailureKind::ChannelFailed)),
            _ = lost.wait_for(|lost| *lost) => Err(SshError::new(FailureKind::Disconnected)),
        }
    }

    /// Window-change request.
    pub async fn resize(&self, cols: u32, rows: u32) -> Result<(), SshError> {
        if self.inner.core.is_closed() {
            return Err(SshError::new(FailureKind::Disconnected));
        }
        self.inner
            .write
            .window_change(dimension(cols), dimension(rows), 0, 0)
            .await
            .map_err(|_| self.inner.core.failure(FailureKind::ChannelFailed))
    }

    /// Sends EOF and closes the channel.
    pub async fn close(&self) {
        let _ = self.inner.write.eof().await;
        let _ = self.inner.write.close().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_send<T: Send>(_: &T) {}
    fn assert_send_sync<T: Send + Sync>() {}

    /// The desktop host spawns connects and moves channels between tasks.
    #[test]
    fn futures_and_handles_cross_tasks() {
        assert_send_sync::<Connection>();
        assert_send_sync::<ShellWriter>();
        assert_send_sync::<SystemAgent>();
        assert_send::<Option<Shell>>(&None);
        struct Nothing;
        impl Interaction for Nothing {
            fn step(&self, _step: ConnectStep) {}
            fn verify_host_key<'a>(
                &'a self,
                _key: &'a HostKeyInfo,
            ) -> BoxFuture<'a, HostKeyDecision> {
                Box::pin(async { HostKeyDecision::Reject })
            }
            fn prompt<'a>(&'a self, _prompt: Prompt) -> BoxFuture<'a, Option<PromptAnswer>> {
                Box::pin(async { None })
            }
        }
        let spec = ConnectSpec {
            address: String::new(),
            port: 22,
            username: None,
            auth: AuthPlan::default(),
            agent: None,
            keepalive: None,
            connect_timeout: Duration::from_secs(1),
            preferred_host_key_types: vec![],
        };
        let future = connect(spec, Arc::new(Nothing));
        assert_send(&future);
        drop(future);
    }

    #[test]
    fn dimensions_are_clamped() {
        assert_eq!(dimension(0), 1);
        assert_eq!(dimension(80), 80);
        assert_eq!(dimension(u32::MAX), 10_000);
    }

    #[test]
    fn signal_names_are_sanitized() {
        assert_eq!(signal_name(&Sig::TERM), "TERM");
        assert_eq!(
            signal_name(&Sig::Custom("X\u{1b}[31mY@example".to_owned())),
            "X31mY@example"
        );
    }

    #[test]
    fn client_id_names_the_app() {
        assert!(CLIENT_ID.starts_with("SSH-2.0-shellpp_"));
        assert!(!CLIENT_ID.contains(' '));
    }

    #[test]
    fn debug_output_has_no_host_data() {
        let spec = ConnectSpec {
            address: "host.example.com".to_owned(),
            port: 22,
            username: Some("alice".to_owned()),
            auth: AuthPlan {
                password: Some(SecretString::from("hunter2".to_owned())),
                ..AuthPlan::default()
            },
            agent: None,
            keepalive: None,
            connect_timeout: Duration::from_secs(15),
            preferred_host_key_types: vec![],
        };
        let text = format!("{spec:?}");
        assert!(!text.contains("example.com"));
        assert!(!text.contains("alice"));
        assert!(!text.contains("hunter2"));
    }
}
