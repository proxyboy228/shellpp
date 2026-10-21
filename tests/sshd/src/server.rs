//! The server: listener, russh server handler, authentication, sessions,
//! host key rotation and connection drops.
//!
//! Layout:
//! - [`TestServer`] owns the accept loop (a task holding the listener, so
//!   `set_accepting(false)` really closes the port) and a registry of live
//!   connections, each with a [`Switch`] that can drop or freeze it at the
//!   TCP level.
//! - One [`ConnHandler`] per connection answers authentication and channel
//!   requests and records [`ServerEvent`]s.
//! - One task per session channel runs the built-in [`Shell`] or answers an
//!   exec request; output goes through the russh session [`Handle`].
//! - Keepalives: russh answers `keepalive@openssh.com` itself; the stream's
//!   [`Trace`] detects those replies (see its documentation) and the server
//!   records [`ServerEvent::Keepalive`]. Every handler callback therefore
//!   starts with `self.trace.callback()`, and session tasks announce their
//!   output through [`Outbound`].
//!
//! Nothing secret or identifying is logged: log lines carry event names only.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, RwLock};
use std::time::Duration;

use russh::keys::agent::client::AgentClient;
use russh::keys::{PrivateKey, PublicKey};
use russh::server::{Auth, ChannelOpenHandle, Handle, Msg, Response, Session};
use russh::{Channel, ChannelId, ChannelMsg, MethodKind, MethodSet, SshId};
use secrecy::{ExposeSecret, SecretString};
use tokio::net::{TcpListener, TcpSocket};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::config::{ExecAnswer, HostKeySource, KiQuestion, ServerConfig};
use crate::conn::{Switch, SwitchedStream, Trace};
use crate::keygen::{self, KeyType};
use crate::shell::{self, Action, PtyInfo, Shell};

/// Identification string without the comment.
pub const SERVER_IDENT: &str = "SSH-2.0-shellpp_test_sshd";

/// Rounds of wrong keyboard-interactive answers before the method is
/// withdrawn for the connection.
const MAX_KI_ROUNDS: u32 = 3;

/// Rejected authentication requests before russh disconnects.
const MAX_AUTH_ATTEMPTS: usize = 10;

/// How long `agent-keys` waits for the forwarded agent.
const AGENT_TIMEOUT: Duration = Duration::from_secs(10);

/// How long `set_accepting(true)` retries a busy port.
const REBIND_TIMEOUT: Duration = Duration::from_secs(5);

/// How long `shutdown` waits for connection tasks to finish.
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(3);

/// Something the server observed, for test assertions. Never contains
/// passwords, answers or key material.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ServerEvent {
    /// An authentication attempt: `method` is `none`, `password`,
    /// `publickey`, `keyboard-interactive`; `ok` is the result. A public key
    /// probe that the server would accept is not an attempt (no event); a
    /// probe with a key that is not authorized is a failed attempt.
    Auth {
        user: String,
        method: String,
        ok: bool,
    },
    /// A PTY request with TERM and size.
    Pty { term: String, cols: u32, rows: u32 },
    /// A window-change request.
    WindowChange { cols: u32, rows: u32 },
    /// An env request (name only).
    Env { name: String },
    /// A shell request.
    Shell,
    /// An exec request with its command text.
    Exec { command: String },
    /// An `auth-agent-req@openssh.com` request.
    AgentForward,
    /// A `direct-tcpip` channel request (opened only with forwarding on and
    /// a loopback destination).
    DirectTcpip { host: String, port: u32 },
    /// A `tcpip-forward` request.
    TcpipForward { address: String, port: u32 },
    /// A `cancel-tcpip-forward` request.
    CancelTcpipForward { address: String, port: u32 },
    /// A keepalive (`keepalive@openssh.com`) global request, answered with a
    /// failure reply as OpenSSH does.
    ///
    /// russh answers such requests without calling the handler, so the
    /// server infers them from the connection's traffic (a reply written
    /// after authenticated inbound traffic that reached no handler callback
    /// and while no session produced output). Any other request russh
    /// answers on its own (unknown global or channel requests with a reply,
    /// a client-initiated rekey) is recorded the same way; keepalives that
    /// coincide with shell output may be missed.
    Keepalive,
    /// A session ended with this exit status (none when dropped or closed
    /// by the client).
    Closed { exit_status: Option<u32> },
}

/// One configured account with parsed keys.
struct Account {
    name: String,
    password: Option<SecretString>,
    keys: Vec<PublicKey>,
    questions: Vec<KiQuestion>,
    ki_name: String,
    ki_instruction: String,
}

/// State shared by the accept loop, the handlers and the session tasks.
struct Shared {
    accounts: Vec<Account>,
    methods: MethodSet,
    exec: Option<ExecAnswer>,
    motd: Option<String>,
    auth_banner: Option<String>,
    server_id: String,
    host_key_type: KeyType,
    russh_config: RwLock<Arc<russh::server::Config>>,
    events: Mutex<Vec<ServerEvent>>,
    connections: Mutex<HashMap<u64, Arc<Switch>>>,
    next_connection: AtomicU64,
    /// Extension: new connections are accepted and frozen at once (the
    /// client waits for the server's identification forever).
    hold_new: AtomicBool,
    forwarding: bool,
    accept_env: Option<Vec<String>>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Shared {
    fn record(&self, event: ServerEvent) {
        lock(&self.events).push(event);
    }

    fn account(&self, name: &str) -> Option<&Account> {
        self.accounts.iter().find(|account| account.name == name)
    }

    fn current_config(&self) -> Arc<russh::server::Config> {
        self.russh_config
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn host_key(&self) -> PublicKey {
        let config = self.current_config();
        config
            .keys
            .first()
            .map(|key| key.public_key().clone())
            .expect("the server config always has one host key")
    }

    fn build_config(&self, key: PrivateKey) -> russh::server::Config {
        russh::server::Config {
            server_id: SshId::Standard(self.server_id.clone().into()),
            methods: self.methods.clone(),
            // A test server: no artificial delay on rejections.
            auth_rejection_time: Duration::ZERO,
            auth_rejection_time_initial: Some(Duration::ZERO),
            keys: vec![key],
            max_auth_attempts: MAX_AUTH_ATTEMPTS,
            // Like OpenSSH defaults: no idle timeout, no server keepalives.
            inactivity_timeout: None,
            keepalive_interval: None,
            event_buffer_size: 64,
            nodelay: true,
            ..Default::default()
        }
    }

    /// Applies `action` to every live connection; returns how many changed.
    fn each_connection(&self, action: impl Fn(&Switch) -> bool) -> usize {
        lock(&self.connections)
            .values()
            .filter(|switch| action(switch))
            .count()
    }
}

/// A running test server. Dropping it stops the listener and every session.
pub struct TestServer {
    addr: SocketAddr,
    shared: Arc<Shared>,
    acceptor: tokio::sync::Mutex<Option<JoinHandle<()>>>,
}

impl std::fmt::Debug for TestServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TestServer { .. }")
    }
}

impl TestServer {
    /// Binds the listener (127.0.0.1, a free port unless configured),
    /// generates or loads the host key and starts accepting connections.
    pub async fn start(config: ServerConfig) -> Result<TestServer, crate::Error> {
        config.validate()?;
        let accounts = parse_accounts(&config)?;
        let methods = offered_methods(&accounts);
        let (host_key, host_key_type) = initial_host_key(config.host_key.clone()).await?;
        let server_id = match &config.banner_comment {
            Some(comment) => format!("{SERVER_IDENT} {comment}"),
            None => SERVER_IDENT.to_owned(),
        };
        let mut shared = Shared {
            accounts,
            methods,
            exec: config.exec.clone(),
            motd: config.motd.clone(),
            auth_banner: config.auth_banner.clone(),
            server_id,
            host_key_type,
            russh_config: RwLock::new(Arc::new(russh::server::Config::default())),
            events: Mutex::new(Vec::new()),
            connections: Mutex::new(HashMap::new()),
            next_connection: AtomicU64::new(1),
            hold_new: AtomicBool::new(false),
            forwarding: config.forwarding,
            accept_env: config.accept_env.clone(),
        };
        let russh_config = Arc::new(shared.build_config(host_key));
        shared.russh_config = RwLock::new(russh_config);
        let shared = Arc::new(shared);

        let listener = bind_listener(config.bind)?;
        let addr = listener.local_addr()?;
        let acceptor = tokio::spawn(accept_loop(listener, Arc::clone(&shared)));
        tracing::debug!("test server listening");
        Ok(TestServer {
            addr,
            shared,
            acceptor: tokio::sync::Mutex::new(Some(acceptor)),
        })
    }

    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    pub fn port(&self) -> u16 {
        self.addr().port()
    }

    /// Algorithm of the current host key, for example `ssh-ed25519`.
    pub fn host_key_type(&self) -> String {
        keygen::openssh_parts(&self.shared.host_key()).0
    }

    /// Base64 blob of the current host public key, as in a known_hosts line.
    pub fn host_key_base64(&self) -> String {
        keygen::openssh_parts(&self.shared.host_key()).1
    }

    /// `SHA256:` fingerprint of the current host key.
    pub fn host_key_fingerprint(&self) -> String {
        keygen::fingerprint(&self.shared.host_key())
    }

    /// The current host public key.
    pub fn host_public_key(&self) -> PublicKey {
        self.shared.host_key()
    }

    /// "Restart with a new host key": closes every connection, generates a
    /// new host key of the same type and keeps listening on the same port.
    pub async fn rotate_host_key(&self) -> Result<(), crate::Error> {
        let key = generate_host_key(self.shared.host_key_type).await?;
        let config = Arc::new(self.shared.build_config(key));
        *self
            .shared
            .russh_config
            .write()
            .unwrap_or_else(PoisonError::into_inner) = config;
        self.shared.each_connection(Switch::kill);
        tracing::debug!("host key rotated");
        Ok(())
    }

    /// Closes every open connection at the TCP level, without exit status
    /// or SSH disconnect message (a drop). Returns the number closed.
    pub fn drop_connections(&self) -> usize {
        let count = self.shared.each_connection(Switch::kill);
        tracing::debug!(count, "connections dropped");
        count
    }

    /// Extension: freezes every open connection. The sockets stay open but
    /// the server stops reading and writing, so clients see no answer to
    /// keepalives (keepalive timeout tests). Frozen connections end with
    /// [`TestServer::drop_connections`], a rotation or the shutdown.
    /// Returns the number frozen.
    pub fn freeze_connections(&self) -> usize {
        let count = self.shared.each_connection(Switch::freeze);
        tracing::debug!(count, "connections frozen");
        count
    }

    /// Extension: while on, new TCP connections are accepted but frozen at
    /// once, before the server's identification (a connection attempt hangs
    /// in the handshake until the client gives up or cancels). Held
    /// connections end with [`TestServer::drop_connections`].
    pub fn set_holding(&self, hold: bool) {
        self.shared.hold_new.store(hold, Ordering::SeqCst);
        tracing::debug!(hold, "holding new connections");
    }

    /// Extension: the number of TCP connections the server currently holds
    /// (including ones being closed).
    pub fn connection_count(&self) -> usize {
        lock(&self.shared.connections).len()
    }

    /// `false` closes the listener (new connections are refused) until
    /// `true` reopens it on the same port. Open connections are kept.
    pub async fn set_accepting(&self, accepting: bool) -> Result<(), crate::Error> {
        let mut acceptor = self.acceptor.lock().await;
        match (accepting, acceptor.take()) {
            (false, Some(task)) => {
                task.abort();
                // Resolves once the task (and with it the listener) is dropped.
                let _ = task.await;
                tracing::debug!("listener closed");
            }
            (true, None) => {
                let listener = rebind(self.addr).await?;
                *acceptor = Some(tokio::spawn(accept_loop(
                    listener,
                    Arc::clone(&self.shared),
                )));
                tracing::debug!("listener reopened");
            }
            (_, current) => *acceptor = current,
        }
        Ok(())
    }

    /// Events observed so far, oldest first.
    pub fn events(&self) -> Vec<ServerEvent> {
        lock(&self.shared.events).clone()
    }

    /// Stops the listener and every session.
    pub async fn shutdown(self) {
        let _ = self.set_accepting(false).await;
        self.shared.each_connection(Switch::kill);
        let deadline = tokio::time::Instant::now() + SHUTDOWN_TIMEOUT;
        while self.connection_count() > 0 && tokio::time::Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        tracing::debug!("test server stopped");
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        if let Ok(mut acceptor) = self.acceptor.try_lock()
            && let Some(task) = acceptor.take()
        {
            task.abort();
        }
        self.shared.each_connection(Switch::kill);
    }
}

fn parse_accounts(config: &ServerConfig) -> Result<Vec<Account>, crate::Error> {
    config
        .users
        .iter()
        .enumerate()
        .map(|(user_index, user)| {
            let keys = user
                .authorized_keys
                .iter()
                .enumerate()
                .map(|(key_index, line)| {
                    PublicKey::from_openssh(line.trim()).map_err(|_| {
                        crate::Error::Config(format!(
                            "users[{user_index}].authorized_keys[{key_index}] is not an OpenSSH public key"
                        ))
                    })
                })
                .collect::<Result<Vec<_>, _>>()?;
            Ok(Account {
                name: user.name.clone(),
                password: user.password.clone(),
                keys,
                questions: user.keyboard_interactive.clone(),
                ki_name: user.ki_name.clone(),
                ki_instruction: user.ki_instruction.clone(),
            })
        })
        .collect()
}

/// The methods configured for any user, in the order OpenSSH lists them.
fn offered_methods(accounts: &[Account]) -> MethodSet {
    let mut methods = Vec::new();
    if accounts.iter().any(|account| !account.keys.is_empty()) {
        methods.push(MethodKind::PublicKey);
    }
    if accounts.iter().any(|account| account.password.is_some()) {
        methods.push(MethodKind::Password);
    }
    if accounts.iter().any(|account| !account.questions.is_empty()) {
        methods.push(MethodKind::KeyboardInteractive);
    }
    MethodSet::from(methods.as_slice())
}

async fn initial_host_key(source: HostKeySource) -> Result<(PrivateKey, KeyType), crate::Error> {
    match source {
        HostKeySource::Generate(key_type) => Ok((generate_host_key(key_type).await?, key_type)),
        HostKeySource::File(path) => {
            let text = std::fs::read_to_string(&path)?;
            let key = PrivateKey::from_openssh(text.trim()).map_err(|_| {
                crate::Error::Key("the host key file is not an OpenSSH private key".to_owned())
            })?;
            if key.is_encrypted() {
                return Err(crate::Error::Key(
                    "the host key file must not be encrypted".to_owned(),
                ));
            }
            let key_type = KeyType::of(&key).ok_or_else(|| {
                crate::Error::Key(
                    "unsupported host key type (ed25519, ecdsa-p256 or rsa)".to_owned(),
                )
            })?;
            Ok((key, key_type))
        }
    }
}

/// Key generation off the async threads (RSA takes a while in debug builds).
async fn generate_host_key(key_type: KeyType) -> Result<PrivateKey, crate::Error> {
    tokio::task::spawn_blocking(move || keygen::random_private_key(key_type))
        .await
        .map_err(|_| crate::Error::Key("host key generation was interrupted".to_owned()))?
}

fn bind_listener(addr: SocketAddr) -> std::io::Result<TcpListener> {
    let socket = if addr.is_ipv4() {
        TcpSocket::new_v4()?
    } else {
        TcpSocket::new_v6()?
    };
    // Lets `set_accepting(true)` rebind while old connections of the port
    // linger in TIME_WAIT (on Windows the option would allow port sharing,
    // so it stays off there).
    #[cfg(unix)]
    socket.set_reuseaddr(true)?;
    socket.bind(addr)?;
    socket.listen(128)
}

async fn rebind(addr: SocketAddr) -> Result<TcpListener, crate::Error> {
    let deadline = tokio::time::Instant::now() + REBIND_TIMEOUT;
    loop {
        match bind_listener(addr) {
            Ok(listener) => return Ok(listener),
            Err(error)
                if error.kind() == std::io::ErrorKind::AddrInUse
                    && tokio::time::Instant::now() < deadline =>
            {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            Err(error) => return Err(error.into()),
        }
    }
}

async fn accept_loop(listener: TcpListener, shared: Arc<Shared>) {
    loop {
        let stream = match listener.accept().await {
            Ok((stream, _peer)) => stream,
            Err(error) => {
                tracing::debug!(kind = ?error.kind(), "accept failed");
                tokio::time::sleep(Duration::from_millis(20)).await;
                continue;
            }
        };
        let _ = stream.set_nodelay(true);
        let id = shared.next_connection.fetch_add(1, Ordering::Relaxed);
        let switch = Switch::new();
        if shared.hold_new.load(Ordering::SeqCst) {
            switch.freeze();
        }
        let trace = Trace::new();
        lock(&shared.connections).insert(id, Arc::clone(&switch));
        let config = shared.current_config();
        let handler = ConnHandler {
            shared: Arc::clone(&shared),
            switch: Arc::clone(&switch),
            trace: Arc::clone(&trace),
            agent_forwarded: Arc::new(AtomicBool::new(false)),
            user: None,
            ki_failures: 0,
            forwards: HashMap::new(),
        };
        let recorder = Arc::clone(&shared);
        let on_silent_reply = Box::new(move || recorder.record(ServerEvent::Keepalive));
        let shared = Arc::clone(&shared);
        tokio::spawn(async move {
            let stream = SwitchedStream::new(stream, switch, trace, on_silent_reply);
            if let Ok(session) = russh::server::run_stream(config, stream, handler).await {
                let _ = session.await;
            }
            lock(&shared.connections).remove(&id);
            tracing::debug!("connection closed");
        });
    }
}

/// Handler of one connection.
struct ConnHandler {
    shared: Arc<Shared>,
    switch: Arc<Switch>,
    trace: Arc<Trace>,
    agent_forwarded: Arc<AtomicBool>,
    /// The authenticated user (the shell prompt shows it).
    user: Option<String>,
    ki_failures: u32,
    /// Listeners of `tcpip-forward` requests, by address and port.
    forwards: HashMap<(String, u32), JoinHandle<()>>,
}

impl Drop for ConnHandler {
    fn drop(&mut self) {
        for (_, task) in self.forwards.drain() {
            task.abort();
        }
    }
}

/// A loopback destination or bind address: `localhost` or a loopback IP.
fn loopback(address: &str) -> Option<std::net::IpAddr> {
    if address.eq_ignore_ascii_case("localhost") {
        return Some(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST));
    }
    address
        .trim_start_matches('[')
        .trim_end_matches(']')
        .parse::<std::net::IpAddr>()
        .ok()
        .filter(|ip| ip.is_loopback())
}

/// Whether an env request may set `name` (see `ServerConfig::accept_env`).
fn env_accepted(patterns: Option<&[String]>, name: &str) -> bool {
    let Some(patterns) = patterns else {
        return true;
    };
    patterns
        .iter()
        .any(|pattern| match pattern.strip_suffix('*') {
            Some(prefix) => name.starts_with(prefix),
            None => pattern == name,
        })
}

/// Copies between a forwarded TCP connection and its channel until either
/// side closes.
async fn relay(mut socket: tokio::net::TcpStream, channel: Channel<Msg>) {
    let mut stream = channel.into_stream();
    let _ = tokio::io::copy_bidirectional(&mut socket, &mut stream).await;
}

impl ConnHandler {
    fn record_auth(&self, user: &str, method: &str, ok: bool) {
        self.shared.record(ServerEvent::Auth {
            user: user.to_owned(),
            method: method.to_owned(),
            ok,
        });
    }

    /// The methods still offered on this connection.
    fn methods_left(&self) -> MethodSet {
        let mut methods = self.shared.methods.clone();
        if self.ki_failures >= MAX_KI_ROUNDS {
            methods.remove(MethodKind::KeyboardInteractive);
        }
        methods
    }

    /// A rejection that keeps offering the methods (russh would otherwise
    /// withdraw `password` after one wrong password).
    fn reject(&self) -> Auth {
        Auth::Reject {
            proceed_with_methods: Some(self.methods_left()),
            partial_success: false,
        }
    }

    fn key_allowed(&self, user: &str, key: &PublicKey) -> bool {
        self.shared.account(user).is_some_and(|account| {
            account
                .keys
                .iter()
                .any(|allowed| allowed.key_data() == key.key_data())
        })
    }

    fn accept(&mut self, user: &str) -> Auth {
        self.user = Some(user.to_owned());
        self.trace.authenticated();
        Auth::Accept
    }
}

impl russh::server::Handler for ConnHandler {
    type Error = russh::Error;

    async fn auth_none(&mut self, user: &str) -> Result<Auth, Self::Error> {
        self.trace.callback();
        self.record_auth(user, "none", false);
        Ok(self.reject())
    }

    async fn auth_password(&mut self, user: &str, password: &str) -> Result<Auth, Self::Error> {
        self.trace.callback();
        let ok = self
            .shared
            .account(user)
            .and_then(|account| account.password.as_ref())
            .is_some_and(|expected| expected.expose_secret() == password);
        self.record_auth(user, "password", ok);
        Ok(if ok { self.accept(user) } else { self.reject() })
    }

    async fn auth_publickey_offered(
        &mut self,
        user: &str,
        public_key: &PublicKey,
    ) -> Result<Auth, Self::Error> {
        self.trace.callback();
        if self.key_allowed(user, public_key) {
            Ok(Auth::Accept)
        } else {
            self.record_auth(user, "publickey", false);
            Ok(self.reject())
        }
    }

    async fn auth_publickey(
        &mut self,
        user: &str,
        public_key: &PublicKey,
    ) -> Result<Auth, Self::Error> {
        self.trace.callback();
        let ok = self.key_allowed(user, public_key);
        self.record_auth(user, "publickey", ok);
        Ok(if ok { self.accept(user) } else { self.reject() })
    }

    async fn auth_keyboard_interactive<'a>(
        &'a mut self,
        user: &str,
        _submethods: &str,
        response: Option<Response<'a>>,
    ) -> Result<Auth, Self::Error> {
        self.trace.callback();
        let shared = Arc::clone(&self.shared);
        let account = shared
            .account(user)
            .filter(|account| !account.questions.is_empty())
            .filter(|_| self.ki_failures < MAX_KI_ROUNDS);
        let Some(account) = account else {
            self.ki_failures += 1;
            self.record_auth(user, "keyboard-interactive", false);
            return Ok(self.reject());
        };
        let Some(response) = response else {
            // Every question in one round.
            let prompts: Vec<_> = account
                .questions
                .iter()
                .map(|question| (question.prompt.clone().into(), question.echo))
                .collect();
            return Ok(Auth::Partial {
                name: account.ki_name.clone().into(),
                instructions: account.ki_instruction.clone().into(),
                prompts: prompts.into(),
            });
        };
        let answers: Vec<_> = response.collect();
        let ok = answers.len() == account.questions.len()
            && answers
                .iter()
                .zip(&account.questions)
                .all(|(answer, question)| {
                    &answer[..] == question.answer.expose_secret().as_bytes()
                });
        self.record_auth(user, "keyboard-interactive", ok);
        if ok {
            Ok(self.accept(user))
        } else {
            self.ki_failures += 1;
            Ok(self.reject())
        }
    }

    async fn auth_succeeded(&mut self, _session: &mut Session) -> Result<(), Self::Error> {
        self.trace.callback();
        Ok(())
    }

    async fn authentication_banner(&mut self) -> Result<Option<String>, Self::Error> {
        self.trace.callback();
        Ok(self.shared.auth_banner.clone())
    }

    async fn channel_open_session(
        &mut self,
        channel: Channel<Msg>,
        reply: ChannelOpenHandle,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.trace.callback();
        reply.accept().await;
        let context = SessionContext {
            shared: Arc::clone(&self.shared),
            switch: Arc::clone(&self.switch),
            agent_forwarded: Arc::clone(&self.agent_forwarded),
            user: self.user.clone().unwrap_or_default(),
            out: Outbound {
                handle: session.handle(),
                trace: Arc::clone(&self.trace),
            },
        };
        tokio::spawn(run_session_channel(channel, context));
        Ok(())
    }

    // Channel types other than "session" are refused (dropping the reply
    // handle rejects the open request).

    async fn channel_open_x11(
        &mut self,
        _channel: Channel<Msg>,
        _originator_address: &str,
        _originator_port: u32,
        _reply: ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.trace.callback();
        Ok(())
    }

    async fn channel_open_direct_tcpip(
        &mut self,
        channel: Channel<Msg>,
        host_to_connect: &str,
        port_to_connect: u32,
        _originator_address: &str,
        _originator_port: u32,
        reply: ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.trace.callback();
        self.shared.record(ServerEvent::DirectTcpip {
            host: host_to_connect.to_owned(),
            port: port_to_connect,
        });
        let destination = loopback(host_to_connect)
            .zip(u16::try_from(port_to_connect).ok())
            .filter(|_| self.shared.forwarding);
        let Some((ip, port)) = destination else {
            reply
                .reject(russh::ChannelOpenFailure::AdministrativelyProhibited)
                .await;
            return Ok(());
        };
        let connect = tokio::net::TcpStream::connect(SocketAddr::new(ip, port));
        match tokio::time::timeout(Duration::from_secs(5), connect).await {
            Ok(Ok(socket)) => {
                reply.accept().await;
                tokio::spawn(relay(socket, channel));
            }
            _ => reply.reject(russh::ChannelOpenFailure::ConnectFailed).await,
        }
        Ok(())
    }

    async fn channel_open_forwarded_tcpip(
        &mut self,
        _channel: Channel<Msg>,
        _host_to_connect: &str,
        _port_to_connect: u32,
        _originator_address: &str,
        _originator_port: u32,
        _reply: ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.trace.callback();
        Ok(())
    }

    async fn channel_open_direct_streamlocal(
        &mut self,
        _channel: Channel<Msg>,
        _socket_path: &str,
        _reply: ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.trace.callback();
        Ok(())
    }

    async fn channel_open_confirmation(
        &mut self,
        _id: ChannelId,
        _max_packet_size: u32,
        _window_size: u32,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.trace.callback();
        Ok(())
    }

    async fn channel_eof(
        &mut self,
        _channel: ChannelId,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.trace.callback();
        Ok(())
    }

    async fn channel_close(
        &mut self,
        _channel: ChannelId,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.trace.callback();
        Ok(())
    }

    // Input reaches the session tasks through their `Channel`; these
    // callbacks only mark the activity.

    async fn data(
        &mut self,
        _channel: ChannelId,
        _data: &[u8],
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.trace.callback();
        Ok(())
    }

    async fn extended_data(
        &mut self,
        _channel: ChannelId,
        _code: u32,
        _data: &[u8],
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.trace.callback();
        Ok(())
    }

    async fn window_adjusted(
        &mut self,
        _channel: ChannelId,
        _new_size: u32,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.trace.callback();
        Ok(())
    }

    async fn signal(
        &mut self,
        _channel: ChannelId,
        _signal: russh::Sig,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.trace.callback();
        Ok(())
    }

    // Remote forwarding on loopback addresses, with forwarding on.

    async fn tcpip_forward(
        &mut self,
        address: &str,
        port: &mut u32,
        session: &mut Session,
    ) -> Result<bool, Self::Error> {
        self.trace.callback();
        self.shared.record(ServerEvent::TcpipForward {
            address: address.to_owned(),
            port: *port,
        });
        let bind = loopback(address)
            .zip(u16::try_from(*port).ok())
            .filter(|_| self.shared.forwarding);
        let Some((ip, requested)) = bind else {
            return Ok(false);
        };
        let Ok(listener) = TcpListener::bind(SocketAddr::new(ip, requested)).await else {
            return Ok(false);
        };
        let Ok(bound) = listener.local_addr() else {
            return Ok(false);
        };
        *port = u32::from(bound.port());
        let handle = session.handle();
        let reported = (address.to_owned(), *port);
        let task = tokio::spawn({
            let (address, port) = reported.clone();
            async move {
                while let Ok((socket, peer)) = listener.accept().await {
                    let handle = handle.clone();
                    let address = address.clone();
                    tokio::spawn(async move {
                        if let Ok(channel) = handle
                            .channel_open_forwarded_tcpip(
                                address,
                                port,
                                peer.ip().to_string(),
                                u32::from(peer.port()),
                            )
                            .await
                        {
                            relay(socket, channel).await;
                        }
                    });
                }
            }
        });
        if let Some(old) = self.forwards.insert(reported, task) {
            old.abort();
        }
        Ok(true)
    }

    async fn cancel_tcpip_forward(
        &mut self,
        address: &str,
        port: u32,
        _session: &mut Session,
    ) -> Result<bool, Self::Error> {
        self.trace.callback();
        self.shared.record(ServerEvent::CancelTcpipForward {
            address: address.to_owned(),
            port,
        });
        match self.forwards.remove(&(address.to_owned(), port)) {
            Some(task) => {
                task.abort();
                Ok(true)
            }
            None => Ok(false),
        }
    }

    async fn streamlocal_forward(
        &mut self,
        _socket_path: &str,
        _session: &mut Session,
    ) -> Result<bool, Self::Error> {
        self.trace.callback();
        Ok(false)
    }

    async fn cancel_streamlocal_forward(
        &mut self,
        _socket_path: &str,
        _session: &mut Session,
    ) -> Result<bool, Self::Error> {
        self.trace.callback();
        Ok(false)
    }

    async fn pty_request(
        &mut self,
        channel: ChannelId,
        term: &str,
        col_width: u32,
        row_height: u32,
        _pix_width: u32,
        _pix_height: u32,
        _modes: &[(russh::Pty, u32)],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.trace.callback();
        self.shared.record(ServerEvent::Pty {
            term: term.to_owned(),
            cols: col_width,
            rows: row_height,
        });
        session.channel_success(channel)
    }

    async fn window_change_request(
        &mut self,
        channel: ChannelId,
        col_width: u32,
        row_height: u32,
        _pix_width: u32,
        _pix_height: u32,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.trace.callback();
        self.shared.record(ServerEvent::WindowChange {
            cols: col_width,
            rows: row_height,
        });
        session.channel_success(channel)
    }

    async fn env_request(
        &mut self,
        channel: ChannelId,
        variable_name: &str,
        _variable_value: &str,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.trace.callback();
        self.shared.record(ServerEvent::Env {
            name: variable_name.to_owned(),
        });
        if env_accepted(self.shared.accept_env.as_deref(), variable_name) {
            session.channel_success(channel)
        } else {
            session.channel_failure(channel)
        }
    }

    async fn shell_request(
        &mut self,
        channel: ChannelId,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.trace.callback();
        self.shared.record(ServerEvent::Shell);
        session.channel_success(channel)
    }

    async fn exec_request(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.trace.callback();
        self.shared.record(ServerEvent::Exec {
            command: String::from_utf8_lossy(data).into_owned(),
        });
        if self.shared.exec.is_some() {
            session.channel_success(channel)
        } else {
            session.channel_failure(channel)
        }
    }

    async fn agent_request(
        &mut self,
        _channel: ChannelId,
        _session: &mut Session,
    ) -> Result<bool, Self::Error> {
        self.trace.callback();
        self.shared.record(ServerEvent::AgentForward);
        self.agent_forwarded.store(true, Ordering::SeqCst);
        Ok(true)
    }

    async fn subsystem_request(
        &mut self,
        channel: ChannelId,
        _name: &str,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.trace.callback();
        // No SFTP in stage 2.
        session.channel_failure(channel)
    }

    async fn x11_request(
        &mut self,
        channel: ChannelId,
        _single_connection: bool,
        _x11_auth_protocol: &str,
        _x11_auth_cookie: &str,
        _x11_screen_number: u32,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.trace.callback();
        session.channel_failure(channel)
    }
}

/// The session handle of a connection; every call first tells the trace
/// that the burst it causes is session output.
#[derive(Clone)]
struct Outbound {
    handle: Handle,
    trace: Arc<Trace>,
}

impl Outbound {
    async fn data(&self, id: ChannelId, bytes: Vec<u8>) {
        self.trace.output();
        let _ = self.handle.data(id, bytes).await;
    }

    /// Exit status, EOF and close.
    async fn finish(&self, id: ChannelId, exit_status: u32) {
        self.trace.output();
        let _ = self.handle.exit_status_request(id, exit_status).await;
        self.trace.output();
        let _ = self.handle.eof(id).await;
        self.trace.output();
        let _ = self.handle.close(id).await;
    }

    async fn open_agent_channel(&self) -> Option<Channel<Msg>> {
        self.trace.output();
        self.handle.channel_open_agent().await.ok()
    }
}

/// What a session channel task needs.
struct SessionContext {
    shared: Arc<Shared>,
    switch: Arc<Switch>,
    agent_forwarded: Arc<AtomicBool>,
    user: String,
    out: Outbound,
}

/// Events the session task sends to itself from timers.
enum Internal {
    Later(String),
    BusyDone {
        generation: u64,
        line: Option<String>,
    },
}

/// How a session channel ended.
#[derive(Clone, Copy, PartialEq, Eq)]
enum End {
    Running,
    /// Exit status sent (recorded already).
    Exited,
    /// Ended without exit status (drop, client close, connection gone).
    WithoutStatus,
}

async fn run_session_channel(mut channel: Channel<Msg>, context: SessionContext) {
    let id = channel.id();
    let (internal_tx, mut internal_rx) = mpsc::unbounded_channel::<Internal>();
    let mut pty: Option<PtyInfo> = None;
    let mut env: Vec<(String, String)> = Vec::new();
    let mut shell: Option<Shell> = None;
    let mut started = false;
    let mut end = End::Running;
    let mut timers: Vec<JoinHandle<()>> = Vec::new();
    let mut busy: Option<(u64, JoinHandle<()>)> = None;
    let mut generation = 0_u64;

    while end == End::Running {
        let actions = tokio::select! {
            message = channel.wait() => {
                let Some(message) = message else {
                    end = End::WithoutStatus;
                    break;
                };
                match message {
                    ChannelMsg::RequestPty { term, col_width, row_height, .. } => {
                        pty = Some(PtyInfo { term, cols: col_width, rows: row_height });
                        Vec::new()
                    }
                    ChannelMsg::WindowChange { col_width, row_height, .. } => {
                        let current = pty.get_or_insert_with(PtyInfo::default);
                        current.cols = col_width;
                        current.rows = row_height;
                        if let Some(shell) = shell.as_mut() {
                            shell.resize(col_width, row_height);
                        }
                        Vec::new()
                    }
                    ChannelMsg::SetEnv { variable_name, variable_value, .. } => {
                        if let Some(shell) = shell.as_mut() {
                            shell.set_env(&variable_name, &variable_value);
                        }
                        env.push((variable_name, variable_value));
                        Vec::new()
                    }
                    ChannelMsg::RequestShell { .. } if !started => {
                        started = true;
                        let mut new_shell =
                            Shell::new(&context.user, context.shared.motd.as_deref(), pty.clone());
                        for (name, value) in &env {
                            new_shell.set_env(name, value);
                        }
                        let actions = new_shell.start();
                        shell = Some(new_shell);
                        actions
                    }
                    ChannelMsg::Exec { .. } if !started => {
                        if let Some(answer) = context.shared.exec.clone() {
                            started = true;
                            answer_exec(&context.out, id, &answer).await;
                            context.shared.record(ServerEvent::Closed {
                                exit_status: Some(answer.exit_status),
                            });
                            end = End::Exited;
                        }
                        Vec::new()
                    }
                    ChannelMsg::Data { data } => shell
                        .as_mut()
                        .map(|shell| shell.input(&data))
                        .unwrap_or_default(),
                    ChannelMsg::Close => {
                        end = End::WithoutStatus;
                        Vec::new()
                    }
                    _ => Vec::new(),
                }
            }
            Some(internal) = internal_rx.recv() => match internal {
                Internal::Later(text) => shell
                    .as_mut()
                    .map(|shell| shell.later(&text))
                    .unwrap_or_default(),
                Internal::BusyDone { generation: done, line } => {
                    if busy.as_ref().is_some_and(|(current, _)| *current == done) {
                        busy = None;
                        shell
                            .as_mut()
                            .map(|shell| shell.resume(line.as_deref()))
                            .unwrap_or_default()
                    } else {
                        Vec::new()
                    }
                }
            },
        };

        for action in actions {
            match action {
                Action::Output(bytes) => context.out.data(id, bytes).await,
                Action::Exit(status) => {
                    context.out.finish(id, status).await;
                    context.shared.record(ServerEvent::Closed {
                        exit_status: Some(status),
                    });
                    end = End::Exited;
                    break;
                }
                Action::Drop => {
                    // Let the echo of the command reach the client first.
                    tokio::time::sleep(Duration::from_millis(50)).await;
                    context.switch.kill();
                    end = End::WithoutStatus;
                    break;
                }
                Action::Sleep(delay) => {
                    generation += 1;
                    let done = generation;
                    let sender = internal_tx.clone();
                    let task = tokio::spawn(async move {
                        tokio::time::sleep(delay).await;
                        let _ = sender.send(Internal::BusyDone {
                            generation: done,
                            line: None,
                        });
                    });
                    busy = Some((done, task));
                }
                Action::AgentKeys | Action::AgentSign => {
                    let sign = matches!(action, Action::AgentSign);
                    generation += 1;
                    let done = generation;
                    let sender = internal_tx.clone();
                    let out = context.out.clone();
                    let forwarded = context.agent_forwarded.load(Ordering::SeqCst);
                    let task = tokio::spawn(async move {
                        let line = if sign {
                            agent_sign_line(out, forwarded).await
                        } else {
                            agent_keys_line(out, forwarded).await
                        };
                        let _ = sender.send(Internal::BusyDone {
                            generation: done,
                            line: Some(line),
                        });
                    });
                    busy = Some((done, task));
                }
                Action::CancelBusy => {
                    if let Some((_, task)) = busy.take() {
                        task.abort();
                    }
                }
                Action::Later(delay, text) => {
                    timers.retain(|timer| !timer.is_finished());
                    let sender = internal_tx.clone();
                    timers.push(tokio::spawn(async move {
                        tokio::time::sleep(delay).await;
                        let _ = sender.send(Internal::Later(text));
                    }));
                }
            }
        }
    }

    for timer in timers {
        timer.abort();
    }
    if let Some((_, task)) = busy {
        task.abort();
    }
    if started && end == End::WithoutStatus {
        context
            .shared
            .record(ServerEvent::Closed { exit_status: None });
    }
}

async fn answer_exec(out: &Outbound, id: ChannelId, answer: &ExecAnswer) {
    if !answer.stdout.is_empty() {
        out.data(id, answer.stdout.clone().into_bytes()).await;
    }
    out.finish(id, answer.exit_status).await;
}

/// The line `agent-sign` prints: asks the client's forwarded agent to sign
/// with its first key ("agent signed: ok", or "agent signed: refused" when
/// the agent refuses). Waits longer than `agent-keys`: the client may ask
/// its user first.
async fn agent_sign_line(out: Outbound, forwarded: bool) -> String {
    if !forwarded {
        return shell::AGENT_NOT_FORWARDED.to_owned();
    }
    let query = async {
        let channel = out.open_agent_channel().await?;
        let mut client = AgentClient::connect(channel.into_stream());
        out.trace.output();
        let identities = client.request_identities().await.ok()?;
        let first = identities.first()?;
        let signed = client
            .sign_request(first, None, b"shellpp test data".to_vec())
            .await
            .is_ok();
        out.trace.output();
        drop(client);
        Some(signed)
    };
    match tokio::time::timeout(AGENT_TIMEOUT * 6, query).await {
        Ok(Some(true)) => "agent signed: ok".to_owned(),
        Ok(Some(false)) => "agent signed: refused".to_owned(),
        _ => shell::AGENT_ERROR.to_owned(),
    }
}

/// The line `agent-keys` prints: asks the client's forwarded agent for its
/// identities over an `auth-agent@openssh.com` channel.
async fn agent_keys_line(out: Outbound, forwarded: bool) -> String {
    if !forwarded {
        return shell::AGENT_NOT_FORWARDED.to_owned();
    }
    let query = async {
        let channel = out.open_agent_channel().await?;
        let mut client = AgentClient::connect(channel.into_stream());
        out.trace.output();
        let identities = client.request_identities().await.ok()?;
        // Dropping the client closes the agent channel (more output).
        out.trace.output();
        drop(client);
        Some(identities.len())
    };
    match tokio::time::timeout(AGENT_TIMEOUT, query).await {
        Ok(Some(count)) => format!("agent keys: {count}"),
        _ => shell::AGENT_ERROR.to_owned(),
    }
}
