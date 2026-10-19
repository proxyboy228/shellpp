//! Terminal sessions (stage 2 plan, section 4.5; docs/architecture.md,
//! sections 3.5, 3.6 and 4.2): SSH sessions and local terminals owned by the
//! window that opened them.
//!
//! - One task per session drives it: connecting (steps, host key decision,
//!   prompts), the running terminal (output pump, input writer), the end and
//!   the reconnect loop ([`ssh`], [`local`]).
//! - Every change bumps the session's `rev` and emits the whole
//!   [`SessionInfo`] as `sessions:changed` to the owning window only
//!   ([`SessionEvents`]); prompts are part of the info (plan decisions 1, 2).
//! - Output goes to a replay ring and to at most one subscriber (the Tauri
//!   channel of `session_attach`), see [`output`].
//! - Commands from a window that does not own the session get `not_found`
//!   (D23: sessions never move between windows).
//! - Lock (plan decision 27): pending prompts are cancelled, credentials
//!   kept for reconnects are wiped, connection attempts in progress are
//!   aborted and reconnect attempts wait for unlock; running terminals keep
//!   running (D9), so `session_attach`, `session_resize` and `session_ack`
//!   are accepted while locked (they reveal nothing).
//! - Flow control: output stops being read while the pane has not
//!   acknowledged enough of it (`session_ack`, [`output`]); input waiting
//!   for the terminal is bounded ([`MAX_PENDING_INPUT`]).
//! - A window that is gone never gets new sessions: its label is kept as
//!   closed (labels are never reused), so an open that finishes after the
//!   window was destroyed ends its session instead of leaving it running.
//!
//! Logs carry session ids, states, steps, error codes and durations only;
//! never addresses, usernames, prompts, server texts or terminal data.

pub mod backend;
pub mod charset;
pub mod lines;
#[cfg(desktop)]
pub mod local;
pub mod output;
mod ssh;
pub mod transport;

use std::collections::{BTreeMap, HashSet};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use secrecy::SecretString;
use shellpp_core::error::{CoreError, FieldError, ValidationReason};
use shellpp_core::service::dto::{
    HostKeyDecision, HostOs, PromptKind, PromptReply, RecordRef, SessionError, SessionInfo,
    SessionInjectInput, SessionKind, SessionOpenInput, SessionPrompt, SessionPromptReplyInput,
    SessionResizeInput, SessionState, SessionWriteInput,
};
use shellpp_core::service::host_api::{ConnectionEndReason, HistoryRecordInput};
use shellpp_ssh::{FailureKind, SshError};
use tokio::sync::{mpsc, oneshot, watch};
use uuid::Uuid;

pub use backend::{SessionCore, SessionSettings};
pub use output::{OutputHub, OutputSink};
pub use transport::{Connector, SshConnector};

use crate::error::CmdError;
use charset::{InputEncoder, OutputDecoder};
use lines::LineTracker;
use transport::{TermEnd, TermEvent, TermEvents, TermInput, TermParts};

/// Columns and rows accepted by `session_open`, `local_open` and `session_resize`.
pub const SIZE_RANGE: std::ops::RangeInclusive<u16> = 1..=1000;
/// Largest `data` of `session_write` and `session_inject` (bytes of UTF-8).
pub const MAX_WRITE_BYTES: usize = 1024 * 1024;
/// Input bytes that may wait for the terminal (the remote side stopped
/// reading, a slow link); more input is refused with `unavailable` until
/// the terminal took some, and the webview retries.
pub const MAX_PENDING_INPUT: usize = 4 * 1024 * 1024;
/// TCP connect plus handshake timeout of an attempt.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
/// How long agent discovery may take before an attempt goes on without it.
const AGENT_DISCOVERY_TIMEOUT: Duration = Duration::from_secs(3);
/// Grace time for closing a transport.
const CLOSE_TIMEOUT: Duration = Duration::from_secs(2);

/// Receives `sessions:changed`: the whole info, for the window `info.window`.
pub trait SessionEvents: Send + Sync + 'static {
    fn session_changed(&self, info: &SessionInfo);
}

/// What the manager is built from.
pub struct ManagerParts {
    pub events: Arc<dyn SessionEvents>,
    pub core: Arc<dyn SessionCore>,
    pub connector: Arc<dyn Connector>,
    #[cfg(desktop)]
    pub local: Arc<dyn local::LocalBackend>,
    /// The runtime that runs session tasks and blocking core calls.
    pub runtime: tokio::runtime::Handle,
    /// Initial lock state (later changes arrive through [`SessionManager::set_unlocked`]).
    pub unlocked: bool,
}

impl std::fmt::Debug for ManagerParts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ManagerParts")
            .field("unlocked", &self.unlocked)
            .finish_non_exhaustive()
    }
}

/// Every session of the process.
pub struct SessionManager {
    shared: Arc<Shared>,
}

impl std::fmt::Debug for SessionManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionManager")
            .field("sessions", &self.shared.sessions().len())
            .finish()
    }
}

pub(crate) struct Shared {
    pub(crate) events: Arc<dyn SessionEvents>,
    pub(crate) core: Arc<dyn SessionCore>,
    pub(crate) connector: Arc<dyn Connector>,
    #[cfg(desktop)]
    pub(crate) local: Arc<dyn local::LocalBackend>,
    pub(crate) rt: tokio::runtime::Handle,
    unlocked: watch::Sender<bool>,
    sessions: Mutex<Registry>,
}

/// The sessions, and the windows that are gone (under one lock, so a
/// session is never added for a window after it closed).
#[derive(Default)]
pub(crate) struct Registry {
    map: BTreeMap<Uuid, Arc<Session>>,
    closed_windows: HashSet<String>,
}

impl std::ops::Deref for Registry {
    type Target = BTreeMap<Uuid, Arc<Session>>;
    fn deref(&self) -> &Self::Target {
        &self.map
    }
}

impl std::ops::DerefMut for Registry {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.map
    }
}

impl Shared {
    fn sessions(&self) -> MutexGuard<'_, Registry> {
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

    /// Fire-and-forget core call (history, connection log end).
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
        let log_id = session.mark_closed(&*self.events);
        self.sessions().remove(&session.id);
        session.output.detach();
        let _ = session.ctrl.send(Control::Close);
        log_id
    }

    /// Adds a new session; `not_found` when its window is gone already.
    fn insert(&self, session: &Arc<Session>) -> Result<(), CmdError> {
        let mut registry = self.sessions();
        if registry.closed_windows.contains(&session.window) {
            tracing::info!(session = %session.id, "window gone before the session started");
            return Err(CmdError::code("not_found"));
        }
        registry.insert(session.id, session.clone());
        Ok(())
    }

    /// Records a history candidate. Candidates of one session are recorded
    /// one after the other, in the order they were typed (the core's
    /// per-session duplicate rule compares with the previous command).
    fn record_history(&self, session: &Arc<Session>, input: HistoryRecordInput) {
        let mut st = session.lock();
        if st.closed {
            return;
        }
        let sender = st.history.get_or_insert_with(|| {
            let (tx, rx) = mpsc::unbounded_channel();
            drop(
                self.rt
                    .spawn(history_loop(self.core.clone(), self.rt.clone(), rx)),
            );
            tx
        });
        let _ = sender.send(input);
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
/// of data bytes not written to the terminal yet.
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
pub(crate) enum Expect {
    HostKeyUnknown,
    HostKeyChanged,
    Text,
}

impl Expect {
    pub(crate) fn of(kind: &PromptKind) -> Self {
        match kind {
            PromptKind::HostKeyUnknown { .. } => Self::HostKeyUnknown,
            PromptKind::HostKeyChanged { .. } => Self::HostKeyChanged,
            _ => Self::Text,
        }
    }

    /// Whether `reply` answers a prompt of this kind (`validation_failed`
    /// with field `reply` otherwise). Cancel answers every prompt.
    pub(crate) fn accepts(self, reply: &PromptReply) -> bool {
        match (self, reply) {
            (_, PromptReply::Cancel) | (Self::Text, PromptReply::Text { .. }) => true,
            (Self::HostKeyUnknown, PromptReply::HostKey { decision }) => matches!(
                decision,
                HostKeyDecision::AcceptSave
                    | HostKeyDecision::ConnectOnce
                    | HostKeyDecision::Cancel
            ),
            (Self::HostKeyChanged, PromptReply::HostKey { decision }) => {
                matches!(decision, HostKeyDecision::Replace | HostKeyDecision::Cancel)
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

/// Credentials typed at prompts, kept for reconnects while unlocked
/// (plan decision 23; docs/architecture.md, section 3.2). They are bound to
/// the server and user that accepted them: when the host record later
/// points elsewhere, they are dropped instead of being sent to another
/// server.
#[derive(Clone, Default)]
pub(crate) struct KeptCredentials {
    /// The address (lowercase), port and username the values were accepted for.
    pub(crate) bound: Option<(String, u16, String)>,
    pub(crate) username: Option<String>,
    pub(crate) password: Option<SecretString>,
    pub(crate) passphrase: Option<SecretString>,
    /// The key record the passphrase opened.
    pub(crate) passphrase_key: Option<RecordRef>,
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
}

/// One session.
pub(crate) struct Session {
    pub(crate) id: Uuid,
    pub(crate) window: String,
    pub(crate) output: OutputHub,
    ctrl: mpsc::UnboundedSender<Control>,
    st: Mutex<SessionMut>,
}

pub(crate) struct SessionMut {
    pub(crate) info: SessionInfo,
    pub(crate) closed: bool,
    prompt: Option<PendingPrompt>,
    next_prompt_id: u64,
    pub(crate) input: Option<InputQueue>,
    pub(crate) size: (u16, u16),
    pub(crate) encoder: InputEncoder,
    pub(crate) lines: LineTracker,
    pub(crate) kept: KeptCredentials,
    /// "Connect once" keys of this session: (key type, base64 blob) (plan decision 26).
    pub(crate) once_keys: Vec<(String, String)>,
    /// The saved host (history link); `None` for quick connects and local terminals.
    pub(crate) host: Option<RecordRef>,
    /// The open connection log entry.
    pub(crate) log_id: Option<Uuid>,
    /// History candidates waiting to be recorded, in order (created on the
    /// first candidate).
    history: Option<mpsc::UnboundedSender<HistoryRecordInput>>,
}

impl Session {
    fn new(
        info: SessionInfo,
        size: (u16, u16),
        host: Option<RecordRef>,
    ) -> (Arc<Self>, mpsc::UnboundedReceiver<Control>) {
        let (ctrl, ctrl_rx) = mpsc::unbounded_channel();
        let session = Arc::new(Self {
            id: info.session_id,
            window: info.window.clone(),
            output: OutputHub::new(),
            ctrl,
            st: Mutex::new(SessionMut {
                info,
                closed: false,
                prompt: None,
                next_prompt_id: 1,
                input: None,
                size,
                encoder: InputEncoder::new(encoding_rs::UTF_8),
                lines: LineTracker::new(),
                kept: KeptCredentials::default(),
                once_keys: Vec::new(),
                host,
                log_id: None,
                history: None,
            }),
        });
        (session, ctrl_rx)
    }

    pub(crate) fn lock(&self) -> MutexGuard<'_, SessionMut> {
        self.st
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    pub(crate) fn info(&self) -> SessionInfo {
        self.lock().info.clone()
    }

    /// Applies a change, bumps `rev` and emits it. Nothing happens once the
    /// session is closed.
    pub(crate) fn update(&self, events: &dyn SessionEvents, change: impl FnOnce(&mut SessionMut)) {
        let mut st = self.lock();
        if st.closed {
            return;
        }
        change(&mut st);
        st.info.rev += 1;
        events.session_changed(&st.info);
    }

    /// The final `closed` event. Returns the open connection log entry.
    fn mark_closed(&self, events: &dyn SessionEvents) -> Option<Uuid> {
        let mut st = self.lock();
        if st.closed {
            return None;
        }
        st.closed = true;
        st.prompt = None;
        st.input = None;
        st.history = None;
        st.kept = KeptCredentials::default();
        let info = &mut st.info;
        info.state = SessionState::Closed;
        info.step = None;
        info.prompt = None;
        info.reconnect = None;
        info.rev += 1;
        events.session_changed(&st.info);
        tracing::info!(session = %self.id, "session closed");
        st.log_id.take()
    }

    /// Counts for exit confirmation: connecting, connected, or dropped with
    /// automatic attempts scheduled.
    fn is_active(&self) -> bool {
        let st = self.lock();
        !st.closed
            && match st.info.state {
                SessionState::Connecting | SessionState::Connected => true,
                SessionState::Dropped => st.info.reconnect.is_some(),
                SessionState::Ended | SessionState::Failed | SessionState::Closed => false,
            }
    }

    /// Shows a prompt; the reply arrives on `reply`. `None` when the session
    /// is closed.
    pub(crate) fn open_prompt(
        &self,
        events: &dyn SessionEvents,
        kind: PromptKind,
        reply: oneshot::Sender<PromptReply>,
    ) -> Option<u64> {
        let mut st = self.lock();
        if st.closed {
            return None;
        }
        let id = st.next_prompt_id;
        st.next_prompt_id += 1;
        st.prompt = Some(PendingPrompt {
            id,
            expect: Expect::of(&kind),
            reply,
        });
        st.info.prompt = Some(SessionPrompt {
            prompt_id: id,
            kind,
        });
        st.info.rev += 1;
        events.session_changed(&st.info);
        Some(id)
    }

    /// Removes prompt `id` if it is still shown.
    pub(crate) fn clear_prompt(&self, events: &dyn SessionEvents, id: u64) {
        let mut st = self.lock();
        if st.prompt.as_ref().is_some_and(|prompt| prompt.id == id) {
            st.prompt = None;
        }
        let shown = st
            .info
            .prompt
            .as_ref()
            .is_some_and(|prompt| prompt.prompt_id == id);
        if shown && !st.closed {
            st.info.prompt = None;
            st.info.rev += 1;
            events.session_changed(&st.info);
        }
    }

    /// Lock: wipe kept credentials and cancel the pending prompt.
    fn on_lock(&self, events: &dyn SessionEvents) {
        let mut st = self.lock();
        st.kept.password = None;
        st.kept.passphrase = None;
        st.kept.passphrase_key = None;
        // Dropping the sender cancels the prompt (the attempt fails).
        st.prompt = None;
        if st.info.prompt.is_some() && !st.closed {
            st.info.prompt = None;
            st.info.rev += 1;
            events.session_changed(&st.info);
        }
    }
}

/// `SessionError` of a core error (`not_found`, `locked`, ...).
pub(crate) fn core_session_error(error: &CoreError) -> SessionError {
    SessionError {
        code: error.code().to_owned(),
        reason: None,
    }
}

/// `SessionError` of an SSH failure: `cancelled`, or `ssh_error` with the reason.
pub(crate) fn ssh_session_error(error: SshError) -> SessionError {
    match error.kind {
        FailureKind::Cancelled => SessionError {
            code: "cancelled".to_owned(),
            reason: None,
        },
        kind => SessionError {
            code: "ssh_error".to_owned(),
            reason: Some(kind.reason().to_owned()),
        },
    }
}

pub(crate) fn error_code(code: &str) -> SessionError {
    SessionError {
        code: code.to_owned(),
        reason: None,
    }
}

/// `cols`/`rows` within [`SIZE_RANGE`].
pub fn validate_size(cols: u16, rows: u16) -> Result<(), CmdError> {
    let mut errors = Vec::new();
    if !SIZE_RANGE.contains(&cols) {
        errors.push(FieldError::new("cols", ValidationReason::OutOfRange));
    }
    if !SIZE_RANGE.contains(&rows) {
        errors.push(FieldError::new("rows", ValidationReason::OutOfRange));
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(CmdError::from(CoreError::Validation(errors)))
    }
}

/// `data` of at most [`MAX_WRITE_BYTES`].
pub fn validate_data(data: &str) -> Result<(), CmdError> {
    if data.len() > MAX_WRITE_BYTES {
        return Err(CmdError::validation("data", ValidationReason::TooLong));
    }
    Ok(())
}

/// The OS of a detection result as `SessionInfo.os` shows it.
pub(crate) fn host_os(
    os: &shellpp_core::os_detect::DetectedOs,
    source: shellpp_core::service::dto::OsSource,
) -> HostOs {
    HostOs {
        family: os.family,
        distro: os.distro.clone(),
        version: os.version.clone(),
        source,
        detected_at: shellpp_core::model::time::format_timestamp(std::time::SystemTime::now()),
    }
}

impl SessionManager {
    pub fn new(parts: ManagerParts) -> Self {
        let (unlocked, _) = watch::channel(parts.unlocked);
        Self {
            shared: Arc::new(Shared {
                events: parts.events,
                core: parts.core,
                connector: parts.connector,
                #[cfg(desktop)]
                local: parts.local,
                rt: parts.runtime,
                unlocked,
                sessions: Mutex::new(Registry::default()),
            }),
        }
    }

    /// Lock state changes (`lock:changed`). Never calls the core: the core
    /// may emit while holding its own lock.
    pub fn set_unlocked(&self, unlocked: bool) {
        let changed = self.shared.unlocked.send_replace(unlocked) != unlocked;
        if !changed {
            return;
        }
        tracing::debug!(unlocked, "sessions follow the lock state");
        if !unlocked {
            let sessions: Vec<Arc<Session>> = self.shared.sessions().values().cloned().collect();
            for session in sessions {
                session.on_lock(&*self.shared.events);
            }
        }
    }

    /// The lock guard of the session commands (the state mirrored from
    /// `lock:changed`; the core enforces it again for its own calls).
    pub fn require_unlocked(&self) -> Result<(), CmdError> {
        if self.shared.is_unlocked() {
            Ok(())
        } else {
            Err(CmdError::locked())
        }
    }

    fn get(&self, window: &str, session_id: Uuid) -> Result<Arc<Session>, CmdError> {
        self.shared
            .sessions()
            .get(&session_id)
            .filter(|session| session.window == window)
            .cloned()
            .ok_or_else(|| CmdError::code("not_found"))
    }

    fn insert(&self, session: &Arc<Session>) -> Result<(), CmdError> {
        self.shared.insert(session)
    }

    /// The sessions of a window (state snapshot), oldest first.
    pub fn sessions_of(&self, window: &str) -> Vec<SessionInfo> {
        self.shared
            .sessions()
            .values()
            .filter(|session| session.window == window)
            .map(|session| session.info())
            .filter(|info| info.state != SessionState::Closed)
            .collect()
    }

    /// Active sessions of the app (Exit confirmation, UI 4.6).
    pub fn active_count(&self) -> u32 {
        let sessions: Vec<Arc<Session>> = self.shared.sessions().values().cloned().collect();
        count(sessions.iter().filter(|session| session.is_active()))
    }

    /// Active sessions of one window (window close confirmation).
    pub fn active_count_of(&self, window: &str) -> u32 {
        let sessions: Vec<Arc<Session>> = self.shared.sessions().values().cloned().collect();
        count(
            sessions
                .iter()
                .filter(|session| session.window == window && session.is_active()),
        )
    }

    /// `session_open`: resolves the effective settings (errors are command
    /// errors), creates the session and starts its task.
    pub async fn open_ssh(
        &self,
        window: &str,
        input: SessionOpenInput,
    ) -> Result<SessionInfo, CmdError> {
        self.require_unlocked()?;
        validate_size(input.cols, input.rows)?;
        let target = input.target.clone();
        let resolved = self
            .shared
            .blocking(move |core| {
                if !core.is_unlocked() {
                    return Err(CoreError::Locked);
                }
                let config = core.session_config(&target)?;
                Ok((config, SessionSettings::read(core)))
            })
            .await
            .ok_or_else(CmdError::internal)?;
        let (config, settings) = resolved.map_err(CmdError::from)?;
        let info = SessionInfo {
            session_id: Uuid::now_v7(),
            rev: 1,
            kind: SessionKind::Ssh,
            window: window.to_owned(),
            state: SessionState::Connecting,
            step: Some(shellpp_core::service::dto::ConnectStep::Resolving),
            title: config.title.clone(),
            target: Some(input.target),
            shell_id: None,
            address: Some(config.address.clone()),
            port: Some(config.port),
            username: config.username.clone(),
            os: config.os.clone(),
            terminal_theme: config.terminal_theme.clone(),
            backspace: config.backspace,
            record_history: config.record_history,
            prompt: None,
            error: None,
            host_key: None,
            exit_status: None,
            reconnect: None,
        };
        let (session, ctrl) = Session::new(info.clone(), (input.cols, input.rows), config.host);
        // The window may have closed while the settings were read: then the
        // session never starts.
        self.insert(&session)?;
        self.shared.events.session_changed(&info);
        tracing::info!(session = %session.id, kind = "ssh", "session opened");
        let task = ssh::SshTask::new(self.shared.clone(), session, ctrl, config, settings);
        drop(self.shared.rt.spawn(task.run()));
        Ok(info)
    }

    /// `local_open` (desktop): resolves the shell and starts it.
    #[cfg(desktop)]
    pub async fn open_local(
        &self,
        window: &str,
        input: shellpp_core::service::dto::LocalOpenInput,
    ) -> Result<SessionInfo, CmdError> {
        self.require_unlocked()?;
        validate_size(input.cols, input.rows)?;
        local::open(self, window, input).await
    }

    /// `local_shells` (desktop).
    #[cfg(desktop)]
    pub async fn local_shells(
        &self,
    ) -> Result<shellpp_core::service::dto::LocalShellsResult, CmdError> {
        self.require_unlocked()?;
        let backend = self.shared.local.clone();
        self.shared
            .rt
            .spawn_blocking(move || {
                let shells = backend.shells(true);
                local::shells_result(&*backend, &shells)
            })
            .await
            .map_err(|_| CmdError::internal())
    }

    /// `session_attach`: replay, then live output to `sink`. Accepted while
    /// locked: running terminals keep their output (D9).
    pub fn attach(
        &self,
        window: &str,
        session_id: Uuid,
        sink: Box<dyn OutputSink>,
    ) -> Result<SessionInfo, CmdError> {
        let session = self.get(window, session_id)?;
        session.output.attach(sink);
        tracing::debug!(session = %session.id, "output attached");
        Ok(session.info())
    }

    /// `session_ack`: the pane's terminal processed `bytes` more bytes of
    /// output (flow control). Accepted while locked.
    pub fn ack(&self, window: &str, session_id: Uuid, bytes: u64) -> Result<(), CmdError> {
        let session = self.get(window, session_id)?;
        session.output.ack(bytes);
        Ok(())
    }

    /// Hosts or settings changed (`records:changed` with hosts,
    /// `settings:changed`): every session takes its current `record_history`
    /// (docs/security.md, section 10.1, rule 4). Runs on a blocking thread
    /// later; never calls the core from the caller's thread (the core may
    /// emit while holding its lock).
    pub fn refresh_record_history(&self) {
        let sessions: Vec<Arc<Session>> = self.shared.sessions().values().cloned().collect();
        if sessions.is_empty() {
            return;
        }
        let shared = self.shared.clone();
        drop(self.shared.rt.spawn_blocking(move || {
            if !shared.core.is_unlocked() {
                return;
            }
            for session in sessions {
                let host = session.lock().host;
                let Ok(record) = shared.core.record_history_now(host) else {
                    continue;
                };
                let current = session.lock().info.record_history;
                if current != record {
                    session.update(&*shared.events, |st| st.info.record_history = record);
                    tracing::debug!(session = %session.id, record, "history recording changed");
                }
            }
        }));
    }

    /// A window (re)loads its page: its output channels are gone, so output
    /// goes to the replay rings until its panes attach again.
    pub fn detach_window(&self, window: &str) {
        let sessions: Vec<Arc<Session>> = self
            .shared
            .sessions()
            .values()
            .filter(|session| session.window == window)
            .cloned()
            .collect();
        for session in sessions {
            session.output.detach();
        }
    }

    /// `session_write`: typed input (and pastes) with an optional history
    /// candidate. `unavailable` while too much input waits for the terminal
    /// ([`MAX_PENDING_INPUT`]): nothing was taken, the webview sends it again.
    pub fn write(&self, window: &str, input: SessionWriteInput) -> Result<(), CmdError> {
        self.require_unlocked()?;
        let session = self.get(window, input.session_id)?;
        validate_data(&input.data)?;
        let mut st = session.lock();
        let Some(queue) = st.input.clone() else {
            // Not connected: dropped silently.
            return Ok(());
        };
        if st.info.state != SessionState::Connected {
            return Ok(());
        }
        let bytes = if input.binary {
            charset::binary_bytes(&input.data)
                .ok_or_else(|| CmdError::validation("data", ValidationReason::Invalid))?
        } else {
            if !queue.has_room(input.data.len()) {
                return Err(CmdError::unavailable());
            }
            st.encoder.encode(&input.data)
        };
        if !queue.has_room(bytes.len()) {
            return Err(CmdError::unavailable());
        }
        let ended = st.lines.typed(&bytes);
        let injected = ended.unwrap_or_else(|| st.lines.current_injected());
        let record = st.info.record_history;
        let host = st.host;
        drop(st);
        if !bytes.is_empty() {
            queue.data(bytes);
        }
        if let Some(candidate) = input.history
            && record
        {
            self.shared.record_history(
                &session,
                HistoryRecordInput {
                    host,
                    session_id: session.id,
                    candidate,
                    injected,
                    record_history: record,
                },
            );
        }
        Ok(())
    }

    /// `session_inject`: input the app sends on its own; never recorded.
    pub fn inject(&self, window: &str, input: SessionInjectInput) -> Result<(), CmdError> {
        self.require_unlocked()?;
        let session = self.get(window, input.session_id)?;
        validate_data(&input.data)?;
        let mut st = session.lock();
        let Some(queue) = st.input.clone() else {
            return Ok(());
        };
        if st.info.state != SessionState::Connected {
            return Ok(());
        }
        if !queue.has_room(input.data.len()) {
            return Err(CmdError::unavailable());
        }
        let bytes = st.encoder.encode(&input.data);
        st.lines.injected(&bytes);
        drop(st);
        if !bytes.is_empty() {
            queue.data(bytes);
        }
        Ok(())
    }

    /// `session_resize`: remembered for the next shell and sent now when
    /// connected. Accepted while locked: a window resized under the lock
    /// screen keeps its terminals' size in step.
    pub fn resize(&self, window: &str, input: SessionResizeInput) -> Result<(), CmdError> {
        let session = self.get(window, input.session_id)?;
        validate_size(input.cols, input.rows)?;
        let mut st = session.lock();
        if st.size == (input.cols, input.rows) {
            return Ok(());
        }
        st.size = (input.cols, input.rows);
        if let Some(queue) = st.input.as_ref() {
            queue.resize(input.cols, input.rows);
        }
        Ok(())
    }

    /// `session_prompt_reply`.
    pub fn prompt_reply(
        &self,
        window: &str,
        input: SessionPromptReplyInput,
    ) -> Result<(), CmdError> {
        self.require_unlocked()?;
        let session = self.get(window, input.session_id)?;
        let mut st = session.lock();
        let pending = match st.prompt.as_ref() {
            Some(prompt) if prompt.id == input.prompt_id => prompt,
            _ => return Err(CmdError::code("not_found")),
        };
        if !pending.expect.accepts(&input.reply) {
            return Err(CmdError::validation("reply", ValidationReason::Invalid));
        }
        let Some(pending) = st.prompt.take() else {
            return Err(CmdError::code("not_found"));
        };
        let _ = pending.reply.send(input.reply);
        if !st.closed && st.info.prompt.is_some() {
            st.info.prompt = None;
            st.info.rev += 1;
            self.shared.events.session_changed(&st.info);
        }
        Ok(())
    }

    /// `session_cancel`: cancels an attempt or stops automatic attempts.
    pub fn cancel(&self, window: &str, session_id: Uuid) -> Result<(), CmdError> {
        self.require_unlocked()?;
        let session = self.get(window, session_id)?;
        let _ = session.ctrl.send(Control::Cancel);
        Ok(())
    }

    /// `session_close`: final `closed`, the session is forgotten.
    pub fn close(&self, window: &str, session_id: Uuid) -> Result<(), CmdError> {
        self.require_unlocked()?;
        let session = self.get(window, session_id)?;
        let log_id = self.shared.close_session(&session);
        self.shared
            .end_log(log_id, ConnectionEndReason::Closed, None);
        Ok(())
    }

    /// `session_reconnect`: Reconnect / Retry in the same tab.
    pub fn reconnect(&self, window: &str, session_id: Uuid) -> Result<SessionInfo, CmdError> {
        self.require_unlocked()?;
        let session = self.get(window, session_id)?;
        let info = session.info();
        match info.state {
            SessionState::Ended | SessionState::Dropped | SessionState::Failed => {
                let _ = session.ctrl.send(Control::Reconnect);
                Ok(info)
            }
            SessionState::Connecting | SessionState::Connected | SessionState::Closed => {
                Err(CmdError::wrong_state())
            }
        }
    }

    /// Closes every session of a window (window closed or destroyed); the
    /// window never gets new sessions afterwards (an open still in flight
    /// ends instead). Returns the connection log entries to end
    /// ([`Self::end_logs_now`] or [`Self::end_logs_later`]).
    pub fn close_window(&self, window: &str) -> Vec<Uuid> {
        let sessions: Vec<Arc<Session>> = {
            let mut registry = self.shared.sessions();
            registry.closed_windows.insert(window.to_owned());
            registry
                .values()
                .filter(|session| session.window == window)
                .cloned()
                .collect()
        };
        if !sessions.is_empty() {
            tracing::info!(closed = sessions.len(), "window sessions closed");
        }
        sessions
            .iter()
            .filter_map(|session| self.shared.close_session(session))
            .collect()
    }

    /// Closes every session (Exit). Returns the connection log entries to
    /// end; the caller ends them before the core shuts down.
    pub fn close_all(&self) -> Vec<Uuid> {
        let sessions: Vec<Arc<Session>> = self.shared.sessions().values().cloned().collect();
        sessions
            .iter()
            .filter_map(|session| self.shared.close_session(session))
            .collect()
    }

    /// Ends connection log entries now (call on a blocking thread).
    pub fn end_logs_now(&self, logs: &[Uuid]) {
        for log_id in logs {
            self.shared
                .core
                .connection_log_end(*log_id, ConnectionEndReason::Closed, None);
        }
    }

    /// Ends connection log entries on a blocking thread.
    pub fn end_logs_later(&self, logs: Vec<Uuid>) {
        for log_id in logs {
            self.shared
                .end_log(Some(log_id), ConnectionEndReason::Closed, None);
        }
    }

    #[cfg(test)]
    pub(crate) fn shared(&self) -> &Arc<Shared> {
        &self.shared
    }
}

fn count<'a>(sessions: impl Iterator<Item = &'a Arc<Session>>) -> u32 {
    u32::try_from(sessions.count()).unwrap_or(u32::MAX)
}

/// How a running terminal stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PumpEnd {
    /// The terminal ended by itself.
    Ended(TermEnd),
    /// The session was closed by the user or its window.
    Closed,
}

/// Opens the input queue of a terminal that is about to run. Call it in the
/// same update that sets `connected`, so input typed right after the event
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
    st.lines.reset();
    InputReceiver { rx, pending }
}

/// Terminal modes reset before the output of a new shell in a pane that
/// showed an earlier one (reconnect, local restart): the old program may
/// have left the alternate screen, mouse tracking, application cursor keys
/// and keypad, bracketed paste, origin mode, a scroll region, no autowrap,
/// a hidden cursor, a character set or colors behind.
///
/// The cursor stays where it is: the normal screen comes back with 1047
/// (1049 would restore a saved cursor position), and the scroll region and
/// origin mode, which move the cursor home, are reset between a cursor save
/// and restore. The final CR LF starts the new output on a line of its own.
pub(crate) const MODE_RESET: &[u8] = b"\x1b[?1047l\x1b[?47l\
\x1b[?1000l\x1b[?1002l\x1b[?1003l\x1b[?1004l\x1b[?1005l\x1b[?1006l\x1b[?1015l\
\x1b[?2004l\x1b[?1l\x1b>\x1b[?7h\x1b[?25h\
\x1b7\x1b[?6l\x1b[r\x1b8\
\x1b(B\x1b[0m\r\n";

/// Puts [`MODE_RESET`] into the output (ring and pane) before a new shell's output.
pub(crate) fn reset_terminal_modes(session: &Session) {
    session.output.push(MODE_RESET);
    session.output.flush();
}

/// Runs a terminal until it ends or the session is closed: output to the
/// hub (decoded, coalesced), input from the queue of [`open_input`] to the
/// terminal. Closes the input queue at the end. Output is not read while
/// the pane is behind with acknowledgements ([`OutputHub::paused`]).
pub(crate) async fn pump(
    shared: &Arc<Shared>,
    session: &Arc<Session>,
    ctrl: &mut mpsc::UnboundedReceiver<Control>,
    parts: TermParts,
    mut decoder: OutputDecoder,
    input_rx: InputReceiver,
) -> PumpEnd {
    let TermParts { events, input } = parts;
    let writer = shared.rt.spawn(write_loop(input.clone(), input_rx));
    let (event_tx, mut event_rx) = mpsc::channel(64);
    let reader = shared.rt.spawn(read_loop(events, event_tx));
    let mut flush_at: Option<tokio::time::Instant> = None;
    let end = loop {
        let flush = async {
            match flush_at {
                Some(at) => tokio::time::sleep_until(at).await,
                None => std::future::pending::<()>().await,
            }
        };
        let paused = session.output.paused();
        tokio::select! {
            event = event_rx.recv(), if !paused => match event {
                Some(TermEvent::Data(data)) => {
                    let data = decoder.decode(data);
                    if let Some(wait) = session.output.push(&data) {
                        flush_at.get_or_insert_with(|| tokio::time::Instant::now() + wait);
                    }
                }
                Some(TermEvent::End(end)) => break PumpEnd::Ended(end),
                None => break PumpEnd::Ended(TermEnd::Dropped(FailureKind::Disconnected)),
            },
            () = session.output.resumed(), if paused => {}
            () = flush => {
                session.output.flush();
                flush_at = None;
            }
            control = ctrl.recv() => match control {
                Some(Control::Close) | None => break PumpEnd::Closed,
                Some(Control::Cancel | Control::Reconnect) => {}
            },
        }
    };
    session.output.flush();
    session.lock().input = None;
    writer.abort();
    if end == PumpEnd::Closed {
        let _ = tokio::time::timeout(CLOSE_TIMEOUT, input.close()).await;
    }
    reader.abort();
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

/// Records the history candidates of one session in order.
async fn history_loop(
    core: Arc<dyn SessionCore>,
    rt: tokio::runtime::Handle,
    mut rx: mpsc::UnboundedReceiver<HistoryRecordInput>,
) {
    while let Some(input) = rx.recv().await {
        let session_id = input.session_id;
        let core = core.clone();
        match rt.spawn_blocking(move || core.history_record(input)).await {
            Ok(Ok(decision)) => tracing::debug!(session = %session_id, ?decision, "history"),
            Ok(Err(error)) => {
                tracing::debug!(session = %session_id, code = error.code(), "history not recorded");
            }
            Err(_) => tracing::error!("history call panicked"),
        }
    }
}

async fn read_loop(mut events: Box<dyn TermEvents>, tx: mpsc::Sender<TermEvent>) {
    loop {
        let event = events.next().await;
        let end = matches!(event, TermEvent::End(_));
        if tx.send(event).await.is_err() || end {
            break;
        }
    }
}

/// Shows a prompt and waits for its reply; the prompt disappears when the
/// future completes or is dropped (attempt aborted). `None` = cancelled.
pub(crate) async fn ask(
    shared: &Arc<Shared>,
    session: &Arc<Session>,
    kind: PromptKind,
) -> Option<PromptReply> {
    struct Shown<'a> {
        shared: &'a Arc<Shared>,
        session: &'a Arc<Session>,
        id: u64,
    }
    impl Drop for Shown<'_> {
        fn drop(&mut self) {
            self.session.clear_prompt(&*self.shared.events, self.id);
        }
    }
    if !shared.is_unlocked() {
        return None;
    }
    let (tx, rx) = oneshot::channel();
    let id = session.open_prompt(&*shared.events, kind, tx)?;
    let _shown = Shown {
        shared,
        session,
        id,
    };
    rx.await.ok()
}

#[cfg(test)]
mod tests;
