//! Terminal sessions (UI 4.1, 4.6, 4.8, 10.2; lead decisions 1, 4-8, 10):
//! SSH sessions through `shellpp-ssh` and local terminals through
//! `shellpp-term-local`, driven on a small Tokio runtime of their own, off
//! the UI thread.
//!
//! # Using it
//!
//! ```text
//! let manager = SessionManager::start(core, ManagerOptions::production());
//! let opened = manager.open(OpenRequest::Ssh(target), (cols, rows), terminal)?;
//! // opened.handle: commands (write, resize, reply, cancel, reconnect, close)
//! // opened.events: every change as SessionEvent::Changed(SessionView)
//! ```
//!
//! - The terminal ([`SessionTerminal`], the terminal view's handle) gets the
//!   session's output, decoded from the host charset to UTF-8, through
//!   [`SessionTerminal::feed`], which waits while the terminal's queue is
//!   full: a flood (`seq 1 3000000`) slows the SSH channel or the PTY reader
//!   instead of growing memory (flow control). Its input, replies to terminal
//!   queries and size changes come back through [`SessionHandle::write`] and
//!   [`SessionHandle::resize`]; [`SessionTerminal::attach`] hands it the
//!   handle before any output flows.
//! - One task per session drives it: connecting (steps, host key decision,
//!   prompts), the running terminal (output pump, input writer), the end and
//!   the reconnect loop (`ssh`, `local`). Every change emits the whole
//!   [`SessionView`]; prompts and host key dialogs are part of it and are
//!   answered with [`SessionHandle::reply`].
//! - End of a session (UI 4.6): exit status 0 closes the session (state
//!   `Closed`, the tab closes); a non-zero status or a signal is `Ended`
//!   (output stays, no automatic reconnect); a transport error, a keepalive
//!   timeout or a close without exit status is `Dropped`, and with
//!   Autoreconnect on, attempts follow after 1, 2, 4, 8, 16 and 32 seconds,
//!   then every 60 seconds. A failed first attempt is `Failed` (no automatic
//!   reconnection).
//! - Lock (UI 13.3, [`SessionManager::set_unlocked`]): pending prompts are
//!   cancelled, credentials kept for reconnects and remembered key file
//!   passphrases are wiped, connection attempts in progress are aborted and
//!   automatic attempts wait for unlock; running terminals keep running.
//! - Every established SSH session writes a connection log entry (host,
//!   user, start, end, device) through the core; terminal output is never
//!   recorded.
//!
//! Logs carry session ids, states, steps, error kinds and durations only;
//! never addresses, usernames, prompts, server texts or terminal data.

mod backend;
mod background;
pub mod charset;
pub mod keys;
pub mod local;
pub mod serial;
mod ssh;
pub mod strings;
pub mod telnet;
pub mod transport;
pub mod types;
pub mod ui;

use std::collections::BTreeMap;
use std::ops::RangeInclusive;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use secrecy::SecretString;
use shellpp_core::error::CoreError;
use shellpp_core::service::dto::{Backspace, RecordRef};
use shellpp_core::service::host_api::ConnectionEndReason;
use tokio::sync::{mpsc, oneshot, watch};
use uuid::Uuid;

pub use backend::{DEFAULT_TERM, DEFAULT_THEME, SessionCore, SessionSettings};
pub use keys::{AgentSource, KeySources, PassphraseCache};
pub use local::{LocalBackend, PtyBackend};
pub use serial::{PortBackend, SerialBackend};
pub use transport::{BoxFuture, Connector, SshConnector};
pub use types::{
    ConnectStep, HostKeyChoice, HostKeyInfo, OpenRequest, PromptKind, PromptReply, ReconnectView,
    SaveOption, SessionError, SessionEvent, SessionId, SessionKind, SessionPrompt, SessionState,
    SessionTarget, SessionView, StartupAsk, WriteError,
};

use charset::InputEncoder;
use transport::{TermEnd, TermEvent, TermInput, TermParts};

/// Columns and rows of a terminal; values outside are clamped.
pub const SIZE_RANGE: RangeInclusive<u16> = 1..=1000;
/// Input bytes that may wait for the remote side (a slow link, a remote
/// program that stopped reading); more is refused with [`WriteError::Busy`].
pub const MAX_PENDING_INPUT: usize = 4 * 1024 * 1024;
/// TCP connect plus handshake timeout of an attempt.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
/// Grace time for closing a transport.
const CLOSE_TIMEOUT: Duration = Duration::from_secs(2);
/// Worker threads of the session runtime (network I/O and timers only:
/// parsing happens on each terminal's own thread).
const RUNTIME_WORKERS: usize = 2;

/// The terminal a session talks to (the terminal view's handle).
pub trait SessionTerminal: Send + Sync + 'static {
    /// Output for the terminal, in order, UTF-8. Waits while the terminal's
    /// queue is full (flow control); an error means the terminal is gone.
    fn feed<'a>(&'a self, bytes: &'a [u8]) -> BoxFuture<'a, Result<(), TerminalClosed>>;
    /// Called once when the session is created, before any output: input,
    /// query replies and size changes go to `session`
    /// ([`SessionHandle::write`], [`SessionHandle::resize`]).
    fn attach(&self, session: SessionHandle);
    /// Input the app sends on its own (a startup snippet): sent like typed
    /// input, and the line that holds it is never recorded in history
    /// (docs/security.md, section 10.1).
    fn inject(&self, bytes: Vec<u8>);
}

/// The terminal of a session is gone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TerminalClosed;

/// The "terminal" of a session without one (SFTP): nothing is shown.
struct NoTerminal;

impl SessionTerminal for NoTerminal {
    fn feed<'a>(&'a self, _bytes: &'a [u8]) -> BoxFuture<'a, Result<(), TerminalClosed>> {
        Box::pin(async { Ok(()) })
    }

    fn attach(&self, _session: SessionHandle) {}

    fn inject(&self, _bytes: Vec<u8>) {}
}

/// The `kind` of the "session opened" log line.
fn view_kind(purpose: ssh::Purpose) -> &'static str {
    match purpose {
        ssh::Purpose::Shell => "ssh",
        ssh::Purpose::Sftp => "sftp",
    }
}

/// Output kept per host of a snippet run on many hosts.
const SNIPPET_OUTPUT_LIMIT: usize = 256 * 1024;

/// How the session layer is built.
pub struct ManagerOptions {
    /// Key files and agent (lead decision 4).
    pub keys: KeySources,
    /// Run on this runtime instead of an own one (tests run on theirs).
    pub runtime: Option<tokio::runtime::Handle>,
    pub connector: Arc<dyn Connector>,
    pub local: Arc<dyn LocalBackend>,
    /// Serial ports (D88).
    pub serial: Arc<dyn SerialBackend>,
    /// Automatic reconnection of dropped SSH sessions by the session layer
    /// (the `terminal.autoreconnect` setting still applies). `false`: a
    /// dropped session stays `Dropped` until [`SessionHandle::reconnect`],
    /// for a caller that schedules the attempts itself.
    pub auto_reconnect: bool,
}

impl std::fmt::Debug for ManagerOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ManagerOptions")
            .field("own_runtime", &self.runtime.is_none())
            .field("auto_reconnect", &self.auto_reconnect)
            .finish_non_exhaustive()
    }
}

impl ManagerOptions {
    /// The app: `~/.ssh` key files, the system agent, its own runtime.
    pub fn production() -> Self {
        Self::with_keys(KeySources::system())
    }

    /// The real transports with these key sources and an own runtime.
    pub fn with_keys(keys: KeySources) -> Self {
        Self {
            keys,
            runtime: None,
            connector: Arc::new(SshConnector),
            local: Arc::new(PtyBackend::new()),
            serial: Arc::new(PortBackend),
            auto_reconnect: true,
        }
    }
}

/// A new session: its command handle and its events.
#[derive(Debug)]
pub struct Opened {
    pub handle: SessionHandle,
    /// Every change of the session, in order; the first is the initial view.
    pub events: futures::channel::mpsc::UnboundedReceiver<SessionEvent>,
}

/// Every session of the process.
pub struct SessionManager {
    shared: Arc<Shared>,
    /// The runtime the manager owns (none when it runs on the caller's).
    runtime: Option<tokio::runtime::Runtime>,
}

impl std::fmt::Debug for SessionManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionManager")
            .field("sessions", &self.shared.registry().len())
            .finish()
    }
}

pub(crate) struct Shared {
    pub(crate) core: Arc<dyn SessionCore>,
    pub(crate) connector: Arc<dyn Connector>,
    pub(crate) local: Arc<dyn LocalBackend>,
    pub(crate) serial: Arc<dyn SerialBackend>,
    pub(crate) keys: KeySources,
    pub(crate) passphrases: PassphraseCache,
    pub(crate) auto_reconnect: bool,
    pub(crate) rt: tokio::runtime::Handle,
    unlocked: watch::Sender<bool>,
    sessions: Mutex<BTreeMap<SessionId, Arc<Session>>>,
    next_id: AtomicU64,
    shut_down: AtomicBool,
    /// Forwarding rules without a tab (stage 5).
    pub(crate) forwarding: background::ForwardingService,
    /// The rules that start on launch were started in this app run.
    launch_autostart_done: AtomicBool,
}

impl Shared {
    /// Starts the rules marked "on launch", once per app run, when the app
    /// is unlocked (D9): at start when it already is ("remember on this
    /// device"), else at the first unlock. Later unlocks do not restart
    /// rules the user stopped.
    fn autostart_on_launch(self: &Arc<Self>) {
        if !self.is_unlocked() || self.launch_autostart_done.swap(true, Ordering::SeqCst) {
            return;
        }
        let shared = Arc::clone(self);
        drop(self.rt.spawn(async move {
            shared
                .forwarding
                .autostart(&shared, |rule| rule.autostart_on_launch)
                .await;
        }));
    }
}

impl Shared {
    fn registry(&self) -> MutexGuard<'_, BTreeMap<SessionId, Arc<Session>>> {
        self.sessions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    pub(crate) fn unlocked(&self) -> watch::Receiver<bool> {
        self.unlocked.subscribe()
    }

    pub(crate) fn is_unlocked(&self) -> bool {
        *self.unlocked.borrow()
    }

    /// Runs a core call on a blocking thread. `None` when it panicked.
    pub(crate) async fn blocking<T, F>(&self, f: F) -> Option<T>
    where
        T: Send + 'static,
        F: FnOnce(&dyn SessionCore) -> T + Send + 'static,
    {
        let core = self.core.clone();
        match self.rt.spawn_blocking(move || f(&*core)).await {
            Ok(value) => Some(value),
            Err(_) => {
                tracing::error!("session core call panicked");
                None
            }
        }
    }

    /// Fire-and-forget core call (connection log end).
    pub(crate) fn background<F>(&self, f: F)
    where
        F: FnOnce(&dyn SessionCore) + Send + 'static,
    {
        let core = self.core.clone();
        drop(self.rt.spawn_blocking(move || f(&*core)));
    }

    /// Marks a session closed (final event), forgets it and tells its task.
    /// Returns the connection log entry to end, if any.
    pub(crate) fn close_session(&self, session: &Arc<Session>) -> Option<Uuid> {
        let log_id = session.mark_closed();
        self.registry().remove(&session.id);
        let _ = session.ctrl.send(Control::Close);
        log_id
    }

    /// Ends a connection log entry on a blocking thread.
    pub(crate) fn end_log(
        &self,
        log_id: Option<Uuid>,
        reason: ConnectionEndReason,
        exit: Option<u32>,
    ) {
        if let Some(log_id) = log_id {
            self.background(move |core| core.connection_log_end(log_id, reason, exit));
        }
    }
}

/// Requests from commands to a session task.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Control {
    /// Cancel the attempt in progress or stop automatic reconnect attempts.
    Cancel,
    /// Reconnect / Retry now.
    Reconnect,
    /// The session was closed (already marked by the manager): tear down.
    Close,
}

/// Input for the writer task of a running terminal, in order.
#[derive(Debug)]
pub(crate) enum InputOp {
    Data(Vec<u8>),
    Resize(u16, u16),
}

/// The sending side of a running terminal's input queue, with the number
/// of data bytes not written yet.
#[derive(Clone, Debug)]
pub(crate) struct InputQueue {
    tx: mpsc::UnboundedSender<InputOp>,
    pending: Arc<AtomicUsize>,
}

impl InputQueue {
    /// Whether `bytes` more may be queued ([`MAX_PENDING_INPUT`]); an empty
    /// queue takes any write.
    fn has_room(&self, bytes: usize) -> bool {
        let pending = self.pending.load(Ordering::SeqCst);
        pending == 0 || pending.saturating_add(bytes) <= MAX_PENDING_INPUT
    }

    fn data(&self, bytes: Vec<u8>) {
        let len = bytes.len();
        self.pending.fetch_add(len, Ordering::SeqCst);
        if self.tx.send(InputOp::Data(bytes)).is_err() {
            self.pending.fetch_sub(len, Ordering::SeqCst);
        }
    }

    fn resize(&self, cols: u16, rows: u16) {
        let _ = self.tx.send(InputOp::Resize(cols, rows));
    }
}

/// The receiving side of [`InputQueue`] (the writer task).
#[derive(Debug)]
pub(crate) struct InputReceiver {
    rx: mpsc::UnboundedReceiver<InputOp>,
    pending: Arc<AtomicUsize>,
}

impl InputReceiver {
    /// A queue nobody sends to (the session was closed meanwhile).
    pub(crate) fn closed() -> Self {
        let (_, rx) = mpsc::unbounded_channel();
        Self {
            rx,
            pending: Arc::new(AtomicUsize::new(0)),
        }
    }
}

/// What a prompt accepts as a reply.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Expect {
    HostKeyUnknown,
    HostKeyChanged,
    Text,
    /// A forwarded signature: Allow or Cancel.
    Signature,
}

impl Expect {
    fn of(kind: &PromptKind) -> Self {
        match kind {
            PromptKind::HostKeyUnknown { .. } => Self::HostKeyUnknown,
            PromptKind::HostKeyChanged { .. } => Self::HostKeyChanged,
            PromptKind::AgentSignature { .. } => Self::Signature,
            _ => Self::Text,
        }
    }

    /// Whether `reply` answers a prompt of this kind. Cancel answers every
    /// prompt; "Replace" never answers a first connection and "Accept and
    /// save" or "Connect once" never answer a changed key (UI 10.2); only
    /// a signature question takes Allow.
    fn accepts(self, reply: &PromptReply) -> bool {
        match (self, reply) {
            (_, PromptReply::Cancel)
            | (Self::Text, PromptReply::Text { .. })
            | (Self::Signature, PromptReply::Allow) => true,
            (Self::HostKeyUnknown, PromptReply::HostKey(choice)) => matches!(
                choice,
                HostKeyChoice::AcceptAndSave | HostKeyChoice::ConnectOnce | HostKeyChoice::Cancel
            ),
            (Self::HostKeyChanged, PromptReply::HostKey(choice)) => {
                matches!(choice, HostKeyChoice::Replace | HostKeyChoice::Cancel)
            }
            _ => false,
        }
    }
}

struct PendingPrompt {
    id: u64,
    expect: Expect,
    reply: oneshot::Sender<PromptReply>,
}

/// Credentials typed at prompts, kept for reconnects of this session while
/// unlocked. They are bound to the server and user that accepted them: when
/// the host record later points elsewhere, they are dropped instead of being
/// sent to another server.
#[derive(Clone, Default)]
pub(crate) struct KeptCredentials {
    /// The address (lowercase), port and username the values were accepted for.
    pub(crate) bound: Option<(String, u16, String)>,
    pub(crate) username: Option<String>,
    pub(crate) password: Option<SecretString>,
    pub(crate) passphrase: Option<SecretString>,
    /// The key record the passphrase opened.
    pub(crate) passphrase_key: Option<RecordRef>,
    /// The key file (by name) and the passphrase that opened it.
    pub(crate) file_passphrase: Option<(String, SecretString)>,
}

impl KeptCredentials {
    /// Whether the kept values may be used to connect to `address`, `port`
    /// as `username` (`None`: the username the values were kept with).
    pub(crate) fn usable_for(&self, address: &str, port: u16, username: Option<&str>) -> bool {
        match &self.bound {
            None => true,
            Some((bound_address, bound_port, bound_user)) => {
                bound_address.eq_ignore_ascii_case(address.trim())
                    && *bound_port == port
                    && username.is_none_or(|username| username == bound_user)
            }
        }
    }

    fn wipe_secrets(&mut self) {
        self.password = None;
        self.passphrase = None;
        self.passphrase_key = None;
        self.file_passphrase = None;
    }
}

/// One session.
pub(crate) struct Session {
    pub(crate) id: SessionId,
    pub(crate) kind: SessionKind,
    /// Dropped when the session closes: the terminal holds this session's
    /// handle, so keeping it would keep both alive.
    terminal: Mutex<Option<Arc<dyn SessionTerminal>>>,
    ctrl: mpsc::UnboundedSender<Control>,
    events: futures::channel::mpsc::UnboundedSender<SessionEvent>,
    st: Mutex<SessionMut>,
}

pub(crate) struct SessionMut {
    pub(crate) view: SessionView,
    pub(crate) closed: bool,
    prompt: Option<PendingPrompt>,
    next_prompt_id: u64,
    pub(crate) input: Option<InputQueue>,
    pub(crate) size: (u16, u16),
    pub(crate) encoder: InputEncoder,
    pub(crate) kept: KeptCredentials,
    /// "Connect once" keys of this session: (key type, base64 blob).
    pub(crate) once_keys: Vec<(String, String)>,
    /// The open connection log entry.
    pub(crate) log_id: Option<Uuid>,
    /// An SFTP session's channel while connected.
    pub(crate) sftp: Option<Arc<shellpp_ssh::Sftp>>,
}

impl Session {
    pub(crate) fn lock(&self) -> MutexGuard<'_, SessionMut> {
        self.st
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    pub(crate) fn view(&self) -> SessionView {
        self.lock().view.clone()
    }

    /// The terminal while the session is open.
    pub(crate) fn terminal(&self) -> Option<Arc<dyn SessionTerminal>> {
        self.terminal
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    fn emit(&self, view: &SessionView) {
        let _ = self
            .events
            .unbounded_send(SessionEvent::Changed(view.clone()));
    }

    /// Applies a change, bumps `rev` and emits the view. Nothing happens
    /// once the session is closed.
    pub(crate) fn update(&self, change: impl FnOnce(&mut SessionMut)) {
        let mut st = self.lock();
        if st.closed {
            return;
        }
        change(&mut st);
        st.view.rev += 1;
        self.emit(&st.view);
    }

    /// The final `Closed` view. Returns the open connection log entry.
    fn mark_closed(&self) -> Option<Uuid> {
        let mut st = self.lock();
        if st.closed {
            return None;
        }
        st.closed = true;
        st.prompt = None;
        st.input = None;
        st.kept = KeptCredentials::default();
        // An SFTP channel is not handed out any more (its task closes it).
        st.sftp = None;
        let view = &mut st.view;
        view.state = SessionState::Closed;
        view.step = None;
        view.prompt = None;
        view.reconnect = None;
        view.rev += 1;
        self.emit(&st.view);
        self.events.close_channel();
        let log_id = st.log_id.take();
        drop(st);
        self.terminal
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        tracing::info!(session = self.id, "session closed");
        log_id
    }

    /// Shows a prompt; the reply arrives on `reply`. `None` when closed, or
    /// (`exclusive`) when another prompt is waiting.
    fn open_prompt(
        &self,
        kind: PromptKind,
        reply: oneshot::Sender<PromptReply>,
        exclusive: bool,
    ) -> Option<u64> {
        let mut st = self.lock();
        if st.closed || (exclusive && st.prompt.is_some()) {
            return None;
        }
        let id = st.next_prompt_id;
        st.next_prompt_id += 1;
        st.prompt = Some(PendingPrompt {
            id,
            expect: Expect::of(&kind),
            reply,
        });
        st.view.prompt = Some(SessionPrompt { id, kind });
        st.view.rev += 1;
        self.emit(&st.view);
        Some(id)
    }

    /// Removes prompt `id` if it is still shown.
    fn clear_prompt(&self, id: u64) {
        let mut st = self.lock();
        if st.prompt.as_ref().is_some_and(|prompt| prompt.id == id) {
            st.prompt = None;
        }
        let shown = st
            .view
            .prompt
            .as_ref()
            .is_some_and(|prompt| prompt.id == id);
        if shown && !st.closed {
            st.view.prompt = None;
            st.view.rev += 1;
            self.emit(&st.view);
        }
    }

    /// Lock: wipe kept credentials and cancel the pending prompt.
    fn on_lock(&self) {
        let mut st = self.lock();
        st.kept.wipe_secrets();
        // Dropping the sender cancels the prompt (the attempt fails).
        st.prompt = None;
        if st.view.prompt.is_some() && !st.closed {
            st.view.prompt = None;
            st.view.rev += 1;
            self.emit(&st.view);
        }
    }
}

/// Commands of one session; cheap to clone, usable from any thread.
#[derive(Clone)]
pub struct SessionHandle {
    session: Arc<Session>,
    shared: Arc<Shared>,
}

impl std::fmt::Debug for SessionHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionHandle")
            .field("id", &self.session.id)
            .finish_non_exhaustive()
    }
}

/// Why [`SessionHandle::reply`] was not taken.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ReplyError {
    /// The prompt is no longer shown (answered, cancelled, replaced).
    #[error("the prompt is no longer shown")]
    Stale,
    /// The reply does not answer this kind of prompt.
    #[error("the reply does not fit the prompt")]
    Invalid,
}

impl SessionHandle {
    pub fn id(&self) -> SessionId {
        self.session.id
    }

    pub fn kind(&self) -> SessionKind {
        self.session.kind
    }

    /// The current view.
    pub fn view(&self) -> SessionView {
        self.session.view()
    }

    /// The SFTP channel of an SFTP session while it is connected.
    pub fn sftp(&self) -> Option<Arc<shellpp_ssh::Sftp>> {
        self.session.lock().sftp.clone()
    }

    /// Runs `work` on the session layer's runtime (the requests of an SFTP
    /// channel); the result arrives through the returned handle.
    pub fn spawn<F>(&self, work: F) -> tokio::task::JoinHandle<F::Output>
    where
        F: std::future::Future + Send + 'static,
        F::Output: Send + 'static,
    {
        self.shared.rt.spawn(work)
    }

    /// Input for the remote side: keys, pastes, mouse reports and replies to
    /// terminal queries, as UTF-8 bytes (encoded here to the host charset).
    /// Dropped while not connected.
    pub fn write(&self, data: &[u8]) -> Result<(), WriteError> {
        if data.is_empty() {
            return Ok(());
        }
        let mut st = self.session.lock();
        let Some(queue) = st.input.clone() else {
            return Err(WriteError::NotConnected);
        };
        if st.view.state != SessionState::Connected {
            return Err(WriteError::NotConnected);
        }
        if !queue.has_room(data.len()) {
            return Err(WriteError::Busy);
        }
        let bytes = st.encoder.encode(data);
        drop(st);
        queue.data(bytes);
        Ok(())
    }

    /// The terminal's size in cells: remembered for the next shell and sent
    /// now when connected (SSH window-change, PTY resize). Clamped to
    /// [`SIZE_RANGE`].
    pub fn resize(&self, cols: u16, rows: u16) {
        let size = (clamp_size(cols), clamp_size(rows));
        let mut st = self.session.lock();
        if st.size == size {
            return;
        }
        st.size = size;
        if let Some(queue) = st.input.as_ref() {
            queue.resize(size.0, size.1);
        }
    }

    /// Answers the prompt `prompt_id` of the current view.
    pub fn reply(&self, prompt_id: u64, reply: PromptReply) -> Result<(), ReplyError> {
        let mut st = self.session.lock();
        let pending = match st.prompt.as_ref() {
            Some(prompt) if prompt.id == prompt_id => prompt,
            _ => return Err(ReplyError::Stale),
        };
        if !pending.expect.accepts(&reply) {
            return Err(ReplyError::Invalid);
        }
        let Some(pending) = st.prompt.take() else {
            return Err(ReplyError::Stale);
        };
        let _ = pending.reply.send(reply);
        if !st.closed && st.view.prompt.is_some() {
            st.view.prompt = None;
            st.view.rev += 1;
            self.session.emit(&st.view);
        }
        Ok(())
    }

    /// "Cancel" on the connection screen: stops the attempt in progress
    /// (the session becomes `Failed`, or stays `Dropped` for a reconnect
    /// attempt); "Cancel" of the reconnect countdown: stops automatic attempts.
    pub fn cancel(&self) {
        let _ = self.session.ctrl.send(Control::Cancel);
    }

    /// Reconnect / Retry in the same tab (`Ended`, `Dropped`, `Failed`);
    /// ignored in other states.
    pub fn reconnect(&self) {
        let state = self.session.lock().view.state;
        if matches!(
            state,
            SessionState::Ended | SessionState::Dropped | SessionState::Failed
        ) {
            let _ = self.session.ctrl.send(Control::Reconnect);
        }
    }

    /// Closes the session ("Close", the tab's close button, Ctrl+Shift+W):
    /// the final `Closed` view, the connection is ended.
    pub fn close(&self) {
        let log_id = self.shared.close_session(&self.session);
        self.shared
            .end_log(log_id, ConnectionEndReason::Closed, None);
    }
}

fn clamp_size(value: u16) -> u16 {
    value.clamp(*SIZE_RANGE.start(), *SIZE_RANGE.end())
}

/// A host with jump hosts is refused until they are supported (stage 5):
/// connecting directly would send its credentials to whatever answers at
/// that address from here.
/// A route the core could not build (a missing or repeated hop) fails
/// before anything connects (data-model 8.3).
pub(crate) fn check_route(
    config: &shellpp_core::service::host_api::SessionConfig,
) -> Result<(), SessionError> {
    match config.route_problem {
        Some(_) => Err(SessionError::RouteInvalid),
        None => Ok(()),
    }
}

/// The error of a core failure while opening or resolving a session.
pub(crate) fn core_error(error: &CoreError) -> SessionError {
    match error {
        CoreError::NotFound => SessionError::HostMissing,
        CoreError::Validation(_) => SessionError::InvalidTarget,
        CoreError::Locked => SessionError::Locked,
        _ => SessionError::Internal,
    }
}

impl SessionManager {
    /// The session layer for `core`. With `options.runtime` unset it starts
    /// its own runtime ([`RUNTIME_WORKERS`] workers).
    pub fn start(core: Arc<dyn SessionCore>, options: ManagerOptions) -> std::io::Result<Self> {
        let (runtime, handle) = match options.runtime {
            Some(handle) => (None, handle),
            None => {
                let runtime = tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(RUNTIME_WORKERS)
                    .max_blocking_threads(16)
                    .thread_name("shellpp-session")
                    .enable_all()
                    .build()?;
                let handle = runtime.handle().clone();
                (Some(runtime), handle)
            }
        };
        let unlocked = core.is_unlocked();
        let shared = Arc::new(Shared {
            core,
            connector: options.connector,
            local: options.local,
            serial: options.serial,
            keys: options.keys,
            passphrases: PassphraseCache::default(),
            auto_reconnect: options.auto_reconnect,
            rt: handle,
            unlocked: watch::Sender::new(unlocked),
            sessions: Mutex::new(BTreeMap::new()),
            next_id: AtomicU64::new(1),
            shut_down: AtomicBool::new(false),
            forwarding: background::ForwardingService::default(),
            launch_autostart_done: AtomicBool::new(false),
        });
        // Shell detection can be slow (WSL): warm the cache early.
        let local = shared.local.clone();
        drop(shared.rt.spawn_blocking(move || {
            let _ = local.shells(false);
        }));
        // Already unlocked at start ("remember on this device"): the rules
        // that start on launch start now.
        shared.autostart_on_launch();
        Ok(Self { shared, runtime })
    }

    /// Starts a forwarding rule of the personal vault on its host's shared
    /// connection (stage 5, the Port Forwarding screen). The connection uses
    /// only saved credentials and known host keys. The work starts now, on
    /// the session layer's runtime; the future only waits for its status.
    pub fn forward_start(
        &self,
        rule: &shellpp_core::service::snippets::ForwardingRuleView,
    ) -> impl std::future::Future<Output = shellpp_ssh::RuleStatus> + 'static {
        let shared = Arc::clone(&self.shared);
        let rule = rule.clone();
        let work = self
            .shared
            .rt
            .spawn(async move { shared.forwarding.start(&shared, &rule).await });
        async move {
            work.await.unwrap_or(shellpp_ssh::RuleStatus::ConnectFailed(
                shellpp_ssh::FailureKind::Cancelled,
            ))
        }
    }

    /// Export to host (UI 7.3; stage 4, item 8): appends `public_key` to
    /// `~/.ssh/authorized_keys` of each host over a connection without a tab
    /// (saved credentials, the agent and key files, known host keys only,
    /// D61), at most four hosts at once. The results keep the hosts' order.
    pub fn export_key(
        &self,
        hosts: Vec<RecordRef>,
        public_key: String,
    ) -> impl std::future::Future<
        Output = Vec<(
            RecordRef,
            Result<shellpp_ssh::ExportOutcome, shellpp_ssh::FailureKind>,
        )>,
    > + 'static {
        use futures::StreamExt as _;
        let shared = Arc::clone(&self.shared);
        let work = self.shared.rt.spawn(async move {
            let key = Arc::new(public_key);
            futures::stream::iter(hosts)
                .map(|host| {
                    let shared = Arc::clone(&shared);
                    let key = Arc::clone(&key);
                    async move {
                        let result = match background::connect(&shared, host).await {
                            Ok(connection) => {
                                let result = shellpp_ssh::export_public_key(&connection, &key)
                                    .await
                                    .map_err(|error| error.kind);
                                connection.disconnect().await;
                                result
                            }
                            Err(error) => Err(error.kind),
                        };
                        tracing::info!(ok = result.is_ok(), "key exported to a host");
                        (host, result)
                    }
                })
                .buffered(4)
                .collect::<Vec<_>>()
                .await
        });
        async move { work.await.unwrap_or_default() }
    }

    /// Stops a forwarding rule (the work starts now, as for
    /// [`SessionManager::forward_start`]).
    pub fn forward_stop(
        &self,
        rule: uuid::Uuid,
    ) -> impl std::future::Future<Output = ()> + 'static {
        let shared = Arc::clone(&self.shared);
        let work = self
            .shared
            .rt
            .spawn(async move { shared.forwarding.stop(rule).await });
        async move {
            let _ = work.await;
        }
    }

    /// The status of a started forwarding rule.
    pub fn forward_status(&self, rule: uuid::Uuid) -> Option<shellpp_ssh::RuleStatus> {
        self.shared.forwarding.status(rule)
    }

    /// Runs `script` (its variables filled in) on the saved hosts `hosts`
    /// through exec channels, at most 8 at a time, each with `timeout`
    /// (UI 9.4, more than 4 targets; the table and the confirmation are the
    /// interface's). Progress goes to `events`; the results are in host
    /// order. The connections use only saved credentials and known host
    /// keys, and close when their command ended. The work starts now; the
    /// future only waits for it.
    pub fn run_on_hosts(
        &self,
        script: String,
        hosts: Vec<RecordRef>,
        timeout: Duration,
        events: Option<mpsc::UnboundedSender<shellpp_ssh::batch::BatchEvent>>,
    ) -> impl std::future::Future<Output = Vec<shellpp_ssh::batch::TargetOutcome>> + 'static {
        // On the session layer's runtime: the caller may be any executor.
        let shared = Arc::clone(&self.shared);
        let run = async move {
            let targets = hosts
                .into_iter()
                .map(|host| {
                    let shared = Arc::clone(&shared);
                    shellpp_ssh::batch::target(move || async move {
                        background::connect(&shared, host).await
                    })
                })
                .collect();
            shellpp_ssh::batch::run(targets, script, timeout, SNIPPET_OUTPUT_LIMIT, events).await
        };
        let work = self.shared.rt.spawn(run);
        async move { work.await.unwrap_or_default() }
    }

    /// The lock state changed (call it from the app's lock events).
    pub fn set_unlocked(&self, unlocked: bool) {
        let changed = self.shared.unlocked.send_replace(unlocked) != unlocked;
        if !changed {
            return;
        }
        tracing::debug!(unlocked, "sessions follow the lock state");
        if unlocked {
            // The rules that start on launch waited for the unlock (D9).
            self.shared.autostart_on_launch();
        }
        if !unlocked {
            self.shared.passphrases.clear();
            let sessions: Vec<Arc<Session>> = self.shared.registry().values().cloned().collect();
            for session in sessions {
                session.on_lock();
            }
        }
    }

    /// Opens a session. `size` is the terminal's size in cells (the
    /// terminal's own resize after [`SessionTerminal::attach`] replaces it).
    /// The session connects in the background; failures to reach the host
    /// arrive as its `Failed` state. Errors here: the app is locked, the
    /// saved host is gone or the quick-connect target is invalid.
    pub fn open(
        &self,
        request: OpenRequest,
        size: (u16, u16),
        terminal: Arc<dyn SessionTerminal>,
    ) -> Result<Opened, SessionError> {
        if self.shared.shut_down.load(Ordering::SeqCst) || !self.shared.is_unlocked() {
            return Err(SessionError::Locked);
        }
        let size = (clamp_size(size.0), clamp_size(size.1));
        match request {
            OpenRequest::Ssh(target) => self.open_ssh(target, size, terminal, ssh::Purpose::Shell),
            OpenRequest::Telnet {
                vault_id,
                record_id,
            } => self.open_telnet(vault_id, record_id, size, terminal),
            OpenRequest::Serial(config) => Ok(self.open_serial(config, size, terminal)),
            OpenRequest::Local { shell_id } => {
                let settings = SessionSettings::read(&*self.shared.core);
                let view = SessionView {
                    id: self.next_id(),
                    kind: SessionKind::Local,
                    state: SessionState::Connecting,
                    step: None,
                    title: crate::i18n::sessions::LOCAL_TERMINAL.to_owned(),
                    host: None,
                    address: None,
                    port: None,
                    username: None,
                    os: None,
                    theme: settings.default_theme,
                    backspace: Backspace::Default,
                    prompt: None,
                    error: None,
                    host_key: None,
                    error_place: None,
                    exit_status: None,
                    exit_signal: None,
                    reconnect: None,
                    shell_id: None,
                    startup_ask: None,
                    rev: 1,
                };
                let (session, ctrl, opened) =
                    self.register(view, size, terminal, InputEncoder::new(encoding_rs::UTF_8));
                tracing::info!(session = session.id, kind = "local", "session opened");
                drop(self.shared.rt.spawn(local::run(
                    self.shared.clone(),
                    session,
                    ctrl,
                    shell_id,
                )));
                Ok(opened)
            }
        }
    }

    /// An SFTP session to `target` (the SFTP tab, stage 4, item 1): the
    /// route, credentials, prompts and host key dialogs of an SSH session,
    /// then the `sftp` subsystem instead of a shell
    /// ([`SessionHandle::sftp`] once connected). It has no terminal.
    pub fn open_sftp(&self, target: SessionTarget) -> Result<Opened, SessionError> {
        if self.shared.shut_down.load(Ordering::SeqCst) || !self.shared.is_unlocked() {
            return Err(SessionError::Locked);
        }
        self.open_ssh(target, (80, 24), Arc::new(NoTerminal), ssh::Purpose::Sftp)
    }

    /// The serial ports of this device (blocking): what the Serial dialog
    /// offers.
    pub fn serial_backend(&self) -> Arc<dyn SerialBackend> {
        self.shared.serial.clone()
    }

    /// A serial session on `config.port` (D88): the title is the port, the
    /// theme the default one.
    fn open_serial(
        &self,
        config: shellpp_term_local::serial::SerialConfig,
        size: (u16, u16),
        terminal: Arc<dyn SessionTerminal>,
    ) -> Opened {
        let settings = SessionSettings::read(&*self.shared.core);
        let view = SessionView {
            id: self.next_id(),
            kind: SessionKind::Serial,
            state: SessionState::Connecting,
            step: None,
            title: config.port.clone(),
            host: None,
            address: None,
            port: None,
            username: None,
            os: None,
            theme: settings.default_theme,
            backspace: Backspace::Default,
            prompt: None,
            error: None,
            host_key: None,
            error_place: None,
            exit_status: None,
            exit_signal: None,
            reconnect: None,
            shell_id: None,
            startup_ask: None,
            rev: 1,
        };
        let (session, ctrl, opened) =
            self.register(view, size, terminal, InputEncoder::new(encoding_rs::UTF_8));
        tracing::info!(session = session.id, kind = "serial", "session opened");
        drop(
            self.shared
                .rt
                .spawn(serial::run(self.shared.clone(), session, ctrl, config)),
        );
        opened
    }

    /// A Telnet session to the saved host's Telnet block (D87): its address
    /// and Telnet port; the title, OS icon, theme and Backspace of the host.
    fn open_telnet(
        &self,
        vault_id: Uuid,
        record_id: Uuid,
        size: (u16, u16),
        terminal: Arc<dyn SessionTerminal>,
    ) -> Result<Opened, SessionError> {
        let config = self
            .shared
            .core
            .session_config(&SessionTarget::Host {
                vault_id,
                record_id,
            })
            .map_err(|error| core_error(&error))?;
        let port = config.telnet_port.ok_or(SessionError::InvalidTarget)?;
        let settings = SessionSettings::read(&*self.shared.core);
        let encoding = charset::encoding_for(&config.charset);
        let view = SessionView {
            id: self.next_id(),
            kind: SessionKind::Telnet,
            state: SessionState::Connecting,
            step: Some(ConnectStep::Connecting),
            title: config.title.clone(),
            host: config.host,
            address: Some(config.address.clone()),
            port: Some(port),
            username: None,
            os: config.os.clone(),
            theme: backend::theme_slug(&config.terminal_theme)
                .unwrap_or_else(|| settings.default_theme.clone()),
            backspace: config.backspace,
            prompt: None,
            error: None,
            host_key: None,
            error_place: None,
            exit_status: None,
            exit_signal: None,
            reconnect: None,
            shell_id: None,
            startup_ask: None,
            rev: 1,
        };
        let (session, ctrl, opened) =
            self.register(view, size, terminal, InputEncoder::new(encoding));
        tracing::info!(session = session.id, kind = "telnet", "session opened");
        drop(self.shared.rt.spawn(telnet::run(
            self.shared.clone(),
            session,
            ctrl,
            telnet::TelnetTarget {
                address: config.address,
                port,
                term: settings.term,
                encoding,
            },
        )));
        Ok(opened)
    }

    /// An SSH session to `target`: a shell or SFTP.
    fn open_ssh(
        &self,
        target: SessionTarget,
        size: (u16, u16),
        terminal: Arc<dyn SessionTerminal>,
        purpose: ssh::Purpose,
    ) -> Result<Opened, SessionError> {
        let config = self
            .shared
            .core
            .session_config(&target)
            .map_err(|error| core_error(&error))?;
        check_route(&config)?;
        let settings = SessionSettings::read(&*self.shared.core);
        let view = SessionView {
            id: self.next_id(),
            kind: match purpose {
                ssh::Purpose::Shell => SessionKind::Ssh,
                ssh::Purpose::Sftp => SessionKind::Sftp,
            },
            state: SessionState::Connecting,
            step: Some(ConnectStep::Resolving),
            title: config.title.clone(),
            host: config.host,
            address: Some(config.address.clone()),
            port: Some(config.port),
            username: config.username.clone(),
            os: config.os.clone(),
            theme: backend::theme_slug(&config.terminal_theme)
                .unwrap_or_else(|| settings.default_theme.clone()),
            backspace: config.backspace,
            prompt: None,
            error: None,
            host_key: None,
            error_place: None,
            exit_status: None,
            exit_signal: None,
            reconnect: None,
            shell_id: None,
            startup_ask: None,
            rev: 1,
        };
        let encoding = charset::encoding_for(&config.charset);
        let (session, ctrl, opened) =
            self.register(view, size, terminal, InputEncoder::new(encoding));
        tracing::info!(
            session = session.id,
            kind = view_kind(purpose),
            "session opened"
        );
        let task = ssh::SshTask::new(
            self.shared.clone(),
            session,
            ctrl,
            target,
            config,
            settings,
            purpose,
        );
        drop(self.shared.rt.spawn(task.run()));
        Ok(opened)
    }

    fn next_id(&self) -> SessionId {
        self.shared.next_id.fetch_add(1, Ordering::SeqCst)
    }

    /// Creates and registers a session, emits its first view and attaches
    /// the terminal (before the task starts, so no output or query reply
    /// can arrive before the terminal knows the session).
    fn register(
        &self,
        view: SessionView,
        size: (u16, u16),
        terminal: Arc<dyn SessionTerminal>,
        encoder: InputEncoder,
    ) -> (Arc<Session>, mpsc::UnboundedReceiver<Control>, Opened) {
        let (ctrl, ctrl_rx) = mpsc::unbounded_channel();
        let (events, events_rx) = futures::channel::mpsc::unbounded();
        let session = Arc::new(Session {
            id: view.id,
            kind: view.kind,
            terminal: Mutex::new(Some(terminal.clone())),
            ctrl,
            events,
            st: Mutex::new(SessionMut {
                view: view.clone(),
                closed: false,
                prompt: None,
                next_prompt_id: 1,
                input: None,
                size,
                encoder,
                kept: KeptCredentials::default(),
                once_keys: Vec::new(),
                log_id: None,
                sftp: None,
            }),
        });
        session.emit(&view);
        self.shared.registry().insert(session.id, session.clone());
        let handle = SessionHandle {
            session: session.clone(),
            shared: self.shared.clone(),
        };
        terminal.attach(handle.clone());
        (
            session,
            ctrl_rx,
            Opened {
                handle,
                events: events_rx,
            },
        )
    }

    /// A handle of an open session.
    pub fn handle(&self, id: SessionId) -> Option<SessionHandle> {
        let session = self.shared.registry().get(&id).cloned()?;
        Some(SessionHandle {
            session,
            shared: self.shared.clone(),
        })
    }

    /// Views of the open sessions, oldest first.
    pub fn views(&self) -> Vec<SessionView> {
        let sessions: Vec<Arc<Session>> = self.shared.registry().values().cloned().collect();
        sessions.iter().map(|session| session.view()).collect()
    }

    /// Sessions that need the exit confirmation (UI 4.6).
    pub fn active_count(&self) -> usize {
        self.views().iter().filter(|view| view.is_active()).count()
    }

    /// Key file passphrases remembered for this app session.
    pub fn remembered_passphrases(&self) -> usize {
        self.shared.passphrases.len()
    }

    /// Closes every session and ends their connection log entries now
    /// (blocking core writes on the calling thread), then stops the own
    /// runtime. Call it before the core shuts down. Idempotent.
    pub fn shutdown(&mut self) {
        if self.shared.shut_down.swap(true, Ordering::SeqCst) {
            return;
        }
        let sessions: Vec<Arc<Session>> = self.shared.registry().values().cloned().collect();
        let logs: Vec<Uuid> = sessions
            .iter()
            .filter_map(|session| self.shared.close_session(session))
            .collect();
        for log_id in logs {
            self.shared
                .core
                .connection_log_end(log_id, ConnectionEndReason::Closed, None);
        }
        self.shared.passphrases.clear();
        // Forwarding rules: listeners and their connections close (bounded,
        // so a dead link does not hold the exit).
        let shared = Arc::clone(&self.shared);
        let stopping = self.shared.rt.spawn(async move {
            let _ =
                tokio::time::timeout(Duration::from_secs(2), shared.forwarding.stop_all()).await;
        });
        drop(stopping);
        if let Some(runtime) = self.runtime.take() {
            // Tasks are dropped: connections and shells end with them.
            runtime.shutdown_timeout(Duration::from_millis(500));
        }
        tracing::debug!("sessions shut down");
    }
}

impl Drop for SessionManager {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// How a running terminal stopped.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum PumpEnd {
    /// The terminal ended by itself.
    Ended(TermEnd),
    /// The session was closed by the user or the app.
    Closed,
}

/// Opens the input queue of a terminal that is about to run. Call it in the
/// same update that sets `Connected`, so input typed right after the event
/// is never dropped. `started_with` is the size the terminal was started
/// with: a resize that arrived while it started is sent first.
pub(crate) fn open_input(st: &mut SessionMut, started_with: (u16, u16)) -> InputReceiver {
    let (tx, rx) = mpsc::unbounded_channel();
    let pending = Arc::new(AtomicUsize::new(0));
    let queue = InputQueue {
        tx,
        pending: pending.clone(),
    };
    if st.size != started_with {
        queue.resize(st.size.0, st.size.1);
    }
    st.input = Some(queue);
    InputReceiver { rx, pending }
}

/// Terminal modes reset before the output of a new shell in a terminal
/// that showed an earlier one (reconnect, local restart): the old program
/// may have left the alternate screen, mouse tracking, application cursor
/// keys and keypad, bracketed paste, insert mode, a cursor shape, origin
/// mode, a scroll region, no autowrap, a hidden cursor, a character set or
/// colors behind.
///
/// The cursor stays where it is: leaving the alternate screen with 1049
/// (the only alternate screen mode alacritty_terminal knows; 47 and 1047
/// are ignored) only swaps back to the normal screen, which kept its own
/// cursor, and does nothing on the normal screen. The scroll region and
/// origin mode, which move the cursor home, are reset between a cursor save
/// and restore. The final CR LF starts the new output on a line of its own.
pub(crate) const MODE_RESET: &[u8] = b"\x1b[?1049l\x1b[?40l\
\x1b[?1000l\x1b[?1002l\x1b[?1003l\x1b[?1004l\x1b[?1005l\x1b[?1006l\x1b[?1015l\
\x1b[?2004l\x1b[?1l\x1b>\x1b[?7h\x1b[?25h\x1b[4l\x1b[0 q\
\x1b7\x1b[?6l\x1b[r\x1b8\
\x1b(B\x1b[0m\r\n";

/// Runs a terminal until it ends or the session is closed: output to the
/// terminal (decoded, with flow control) and to its recording, if any
/// (D75), input from the queue of [`open_input`]. Closes the input queue
/// at the end; the recording closes when it is dropped.
pub(crate) async fn pump(
    shared: &Arc<Shared>,
    session: &Arc<Session>,
    ctrl: &mut mpsc::UnboundedReceiver<Control>,
    parts: TermParts,
    mut decoder: charset::OutputDecoder,
    input_rx: InputReceiver,
    mut recording: Option<shellpp_core::recording::RecordingWriter>,
) -> PumpEnd {
    let TermParts {
        mut events, input, ..
    } = parts;
    let writer = shared.rt.spawn(write_loop(input.clone(), input_rx));
    let terminal = session.terminal();
    let mut terminal_open = terminal.is_some();
    let end = 'run: loop {
        let event = tokio::select! {
            event = events.next() => event,
            control = ctrl.recv() => match control {
                Some(Control::Close) | None => break 'run PumpEnd::Closed,
                Some(Control::Cancel | Control::Reconnect) => continue,
            },
        };
        match event {
            TermEvent::Data(data) => {
                let data = decoder.decode(data);
                // Output only (input is never recorded, security.md 10.4).
                if let Some(writer) = recording.as_mut()
                    && let Err(error) = writer.output(&data)
                {
                    tracing::warn!(%error, "session recording stopped");
                    recording = None;
                }
                let Some(terminal) = terminal.as_ref().filter(|_| terminal_open) else {
                    continue;
                };
                if data.is_empty() {
                    continue;
                }
                // Waits while the terminal is behind (flow control); a
                // close is still taken while waiting.
                let fed = {
                    let feed = terminal.feed(&data);
                    tokio::pin!(feed);
                    loop {
                        tokio::select! {
                            result = &mut feed => break result,
                            control = ctrl.recv() => match control {
                                Some(Control::Close) | None => break 'run PumpEnd::Closed,
                                Some(Control::Cancel | Control::Reconnect) => {}
                            },
                        }
                    }
                };
                if fed.is_err() {
                    // The terminal view is gone: keep reading (and
                    // dropping) until the session is closed.
                    terminal_open = false;
                }
            }
            TermEvent::End(end) => break PumpEnd::Ended(end),
        }
    };
    session.lock().input = None;
    writer.abort();
    if end == PumpEnd::Closed {
        let _ = tokio::time::timeout(CLOSE_TIMEOUT, input.close()).await;
    }
    end
}

async fn write_loop(input: Arc<dyn TermInput>, receiver: InputReceiver) {
    let InputReceiver { mut rx, pending } = receiver;
    while let Some(op) = rx.recv().await {
        let alive = match op {
            InputOp::Data(data) => {
                let len = data.len();
                let alive = input.write(data).await;
                pending.fetch_sub(len, Ordering::SeqCst);
                alive
            }
            InputOp::Resize(cols, rows) => input.resize(cols, rows).await,
        };
        if !alive {
            break;
        }
    }
}

/// Feeds [`MODE_RESET`] before a new shell's output.
pub(crate) async fn reset_terminal_modes(session: &Session) {
    if let Some(terminal) = session.terminal() {
        let _ = terminal.feed(MODE_RESET).await;
    }
}

/// Shows a prompt and waits for its reply; the prompt disappears when the
/// future completes or is dropped (attempt aborted). `None` = cancelled.
pub(crate) async fn ask(
    shared: &Arc<Shared>,
    session: &Arc<Session>,
    kind: PromptKind,
) -> Option<PromptReply> {
    ask_with(shared, session, kind, false).await
}

async fn ask_with(
    shared: &Arc<Shared>,
    session: &Arc<Session>,
    kind: PromptKind,
    exclusive: bool,
) -> Option<PromptReply> {
    struct Shown<'a> {
        session: &'a Arc<Session>,
        id: u64,
    }
    impl Drop for Shown<'_> {
        fn drop(&mut self) {
            self.session.clear_prompt(self.id);
        }
    }
    if !shared.is_unlocked() {
        return None;
    }
    let (tx, rx) = oneshot::channel();
    let id = session.open_prompt(kind, tx, exclusive)?;
    let _shown = Shown { session, id };
    rx.await.ok()
}

/// [`ask`] that never replaces a question already waiting: `None` at once
/// then (a forwarded signature while another is asked, D60).
pub(crate) async fn ask_exclusive(
    shared: &Arc<Shared>,
    session: &Arc<Session>,
    kind: PromptKind,
) -> Option<PromptReply> {
    ask_with(shared, session, kind, true).await
}

#[cfg(test)]
mod tests;
