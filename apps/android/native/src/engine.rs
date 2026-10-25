//! The process-wide state of the app: the core on the user's vault, the
//! session layer, the open sessions with their terminals, and the queue of
//! events the app reads.
//!
//! The vault lives in `<files>/vault` and lasts across starts (M1); the
//! app's first run makes it, the master password or the fingerprint opens
//! it (UI spec 4, 5). Sessions need it unlocked, as on the desktop.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use alacritty_terminal::vte::ansi::CursorShape;
use futures::StreamExt as _;
use secrecy::SecretString;
use serde_json::{Value, json};
use shellpp_core::platform::{
    Clipboard, ExternalLauncher, OsKind, Platform, PlatformClass, SecretStore, SystemClock,
};
use shellpp_core::service::dto::SessionTarget;
use shellpp_core::{Core, CoreConfig};
use shellpp_sessions::{
    AgentSource, BoxFuture, HostKeyChoice, KeySources, ManagerOptions, OpenRequest, PromptKind,
    PromptReply, SessionCore, SessionEvent, SessionHandle, SessionManager, SessionState,
    SessionTerminal, SessionView, TerminalClosed,
};
use shellpp_terminal::model::{ModelOptions, TerminalSize};
use shellpp_terminal::{TerminalHandle, TerminalSink};

use crate::cursor;
use crate::events::EventQueue;
use crate::input::Cell;
use crate::sftp::{SftpFailure, SftpSessions};

/// Lines of scrollback of a prototype terminal.
const SCROLLBACK: usize = 2000;

/// Why the engine could not do something (shown to the user as it is).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EngineError(pub String);

impl std::fmt::Display for EngineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for EngineError {}

fn error(text: impl Into<String>) -> EngineError {
    EngineError(text.into())
}

/// The platform services the app gives the core (architecture 2): the
/// Keystore behind `SecretStore`, the clipboard, the browser.
pub struct Services {
    pub secret_store: Arc<dyn SecretStore>,
    pub clipboard: Arc<dyn Clipboard>,
    pub launcher: Arc<dyn ExternalLauncher>,
    /// The default device name (the phone's model).
    pub device_name: String,
}

impl std::fmt::Debug for Services {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Services").finish_non_exhaustive()
    }
}

/// The terminal model as a session's terminal: output goes to its parser
/// queue (which waits while full: flow control), and the session becomes
/// its sink for input and size changes. As the desktop's
/// `sessions::ui::ViewTerminal`, without the history (the prototype has no
/// saved hosts).
struct PhoneTerminal(TerminalHandle);

impl SessionTerminal for PhoneTerminal {
    fn feed<'a>(&'a self, bytes: &'a [u8]) -> BoxFuture<'a, Result<(), TerminalClosed>> {
        Box::pin(async move { self.0.feed(bytes).await.map_err(|_| TerminalClosed) })
    }

    fn attach(&self, session: SessionHandle) {
        self.0.set_sink(Some(Arc::new(SessionSink(session))));
    }

    fn inject(&self, bytes: Vec<u8>) {
        self.0.inject_input(bytes);
    }
}

/// Input, query replies and size changes of the terminal go to the session.
struct SessionSink(SessionHandle);

impl TerminalSink for SessionSink {
    fn send(&self, bytes: Vec<u8>) {
        if let Err(error) = self.0.write(&bytes) {
            tracing::debug!(session = self.0.id(), %error, "terminal input dropped");
        }
    }

    fn resize(&self, size: TerminalSize) {
        self.0.resize(size.cols, size.rows);
    }
}

/// One open session and its terminal.
struct Live {
    session: SessionHandle,
    terminal: TerminalHandle,
    /// The terminal's own sender, for [`Engine::wake`].
    wake: async_channel::Sender<()>,
    woken: async_channel::Receiver<()>,
}

/// The core, the session layer and the open sessions.
pub struct Engine {
    core: Arc<Core>,
    manager: Arc<SessionManager>,
    key_dir: PathBuf,
    sessions: Mutex<BTreeMap<u64, Live>>,
    /// SFTP sessions (no terminal) and their transfers (UI spec 10).
    sftp: SftpSessions,
    next: AtomicU64,
    events: Arc<EventQueue>,
    /// Held while the session layer's lock state is set from the core's.
    lock_sync: Arc<Mutex<()>>,
    /// The terminal theme of Settings → Font & colours (a built-in slug),
    /// `None` to follow the system's dark or light mode (run 15).
    theme: Mutex<Option<String>>,
}

impl std::fmt::Debug for Engine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Engine").finish_non_exhaustive()
    }
}

impl Engine {
    /// Opens the core on the vault under `files` (made at the app's first
    /// run) and starts the session layer. Key files for quick connects are
    /// offered from `files/keys` (the app copies a picked key there).
    pub fn start(files: &Path, services: Services) -> Result<Self, EngineError> {
        let data_dir = files.join("vault");
        std::fs::create_dir_all(&data_dir).map_err(|e| error(format!("data folder: {e}")))?;
        // A key file picked for a quick connect lasts one run, as the vault
        // does (review of run 13): never left on disk for later servers.
        let key_dir = files.join("keys");
        if key_dir.exists() {
            std::fs::remove_dir_all(&key_dir).map_err(|e| error(format!("old keys: {e}")))?;
        }
        std::fs::create_dir_all(&key_dir).map_err(|e| error(format!("key folder: {e}")))?;
        let events = Arc::new(EventQueue::default());
        let lock_changes = events.follow_lock();
        let platform = Platform {
            secret_store: services.secret_store,
            clipboard: services.clipboard,
            launcher: services.launcher,
            events: events.clone(),
            clock: Arc::new(SystemClock),
            class: PlatformClass::Mobile,
            os: OsKind::Android,
            device_name: services.device_name,
        };
        let core = Core::open(
            CoreConfig {
                data_dir,
                app_version: env!("CARGO_PKG_VERSION").to_owned(),
            },
            platform,
        )
        .map_err(|e| error(format!("core: {}", e.code())))?;
        let core = Arc::new(core);
        let options = ManagerOptions::with_keys(KeySources {
            key_dir: Some(key_dir.clone()),
            agent: AgentSource::None,
        });
        let manager = SessionManager::start(core.clone() as Arc<dyn SessionCore>, options)
            .map_err(|e| error(format!("sessions: {e}")))?;
        let manager = Arc::new(manager);
        // The session layer follows the core's lock (prompts cancelled while
        // locked, reconnects waiting for unlock), as the desktop's lock
        // events tell it; it ends with the core.
        // Every change is applied from the core's state as it is then, under
        // the same lock as the opens, so a lock between a read and a write
        // never leaves the session layer unlocked (review of run 14).
        let lock_sync = Arc::new(Mutex::new(()));
        let follower = Arc::downgrade(&manager);
        let follower_core = Arc::downgrade(&core);
        let follower_sync = lock_sync.clone();
        std::thread::Builder::new()
            .name("lock-follower".to_owned())
            .spawn(move || {
                while lock_changes.recv().is_ok() {
                    let (Some(manager), Some(core)) = (follower.upgrade(), follower_core.upgrade())
                    else {
                        break;
                    };
                    let _held = follower_sync.lock().unwrap_or_else(PoisonError::into_inner);
                    manager.set_unlocked(core.is_unlocked());
                }
            })
            .map_err(|e| error(format!("lock thread: {e}")))?;
        Ok(Self {
            core,
            manager,
            key_dir,
            sessions: Mutex::new(BTreeMap::new()),
            sftp: SftpSessions::new(events.clone()),
            next: AtomicU64::new(1),
            events,
            lock_sync,
            theme: Mutex::new(None),
        })
    }

    /// Settings → Font & colours: the theme new terminals take (`None`:
    /// Graphite or Paper with the system's mode).
    pub fn set_theme(&self, slug: Option<String>) {
        *self.theme.lock().unwrap_or_else(PoisonError::into_inner) =
            slug.filter(|slug| !slug.is_empty());
    }

    /// The theme of a new terminal: the host's own (data-model 7.2
    /// `terminal_theme`, a built-in one), else Settings', else the system's
    /// mode.
    fn theme_for(&self, target: &SessionTarget, dark: bool) -> shellpp_terminal::colors::Palette {
        let host = match target {
            SessionTarget::Host {
                vault_id,
                record_id,
            } => self
                .core
                .record_get(shellpp_core::service::dto::RecordRef {
                    vault_id: *vault_id,
                    record_id: *record_id,
                })
                .ok()
                .and_then(|view| match view {
                    shellpp_core::service::dto::RecordView::Host(host) => host
                        .data
                        .get("terminal_theme")
                        .and_then(|theme| theme.get("builtin"))
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_owned),
                    shellpp_core::service::dto::RecordView::Group(_) => None,
                }),
            _ => None,
        };
        let chosen = host.or_else(|| {
            self.theme
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone()
        });
        crate::theme::palette_for(chosen.as_deref(), dark)
    }

    /// Where a key file for quick connects goes.
    pub fn key_dir(&self) -> &Path {
        &self.key_dir
    }

    /// The core (the dispatcher's commands; tests read its known hosts).
    pub fn core(&self) -> &Arc<Core> {
        &self.core
    }

    /// Runs one command of the core's service API ([`crate::dispatch`]).
    pub fn call(&self, command: &str, input: &[u8]) -> String {
        crate::dispatch::call(&self.core, command, input)
    }

    fn sessions(&self) -> std::sync::MutexGuard<'_, BTreeMap<u64, Live>> {
        self.sessions.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn with<R>(&self, id: u64, read: impl FnOnce(&Live) -> R) -> Result<R, EngineError> {
        self.sessions()
            .get(&id)
            .map(read)
            .ok_or_else(|| error("no such session"))
    }

    /// Opens a quick-connect SSH session to `address` (nothing saved).
    pub fn open(
        &self,
        address: &str,
        port: Option<u16>,
        username: Option<&str>,
        cols: u16,
        rows: u16,
        dark: bool,
    ) -> Result<u64, EngineError> {
        let target = SessionTarget::Quick {
            address: address.trim().to_owned(),
            port,
            username: username.map(str::to_owned).filter(|name| !name.is_empty()),
        };
        self.open_target(target, cols, rows, dark)
    }

    /// Opens an SSH session to `target` (a saved host or a quick connect)
    /// in a terminal of `cols` x `rows`, in the dark or light theme;
    /// returns its handle. Every change of the session comes from
    /// [`Self::next_event`].
    pub fn open_target(
        &self,
        target: SessionTarget,
        cols: u16,
        rows: u16,
        dark: bool,
    ) -> Result<u64, EngineError> {
        let (wake, woken) = async_channel::bounded(1);
        let terminal = TerminalHandle::spawn(
            ModelOptions {
                size: TerminalSize::cells(cols, rows),
                scrollback: SCROLLBACK,
                cursor_shape: CursorShape::Block,
            },
            self.theme_for(&target, dark),
            wake.clone(),
        );
        // An unlock just now may not have reached the follower thread yet.
        self.sync_lock_state();
        let opened = self
            .manager
            .open(
                OpenRequest::Ssh(target),
                (cols, rows),
                Arc::new(PhoneTerminal(terminal.clone())),
            )
            .map_err(|e| error(e.message()))?;
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        if self.forward(id, opened.events).is_err() {
            opened.handle.close();
            return Err(error("no thread for the session's events"));
        }
        self.sessions().insert(
            id,
            Live {
                session: opened.handle,
                terminal,
                wake,
                woken,
            },
        );
        Ok(id)
    }

    /// The session's events, as JSON, into the engine's queue: a thread of
    /// its own waits on the session's stream.
    fn forward(
        &self,
        id: u64,
        mut events: futures::channel::mpsc::UnboundedReceiver<SessionEvent>,
    ) -> std::io::Result<std::thread::JoinHandle<()>> {
        let tx = self.events.sender();
        std::thread::Builder::new()
            .name(format!("session-events-{id}"))
            .spawn(move || {
                let Ok(runtime) = tokio::runtime::Builder::new_current_thread().build() else {
                    return;
                };
                runtime.block_on(async move {
                    while let Some(event) = events.next().await {
                        let SessionEvent::Changed(view) = event;
                        let closed = view.state == SessionState::Closed;
                        if tx.send(view_json(id, &view).to_string()).is_err() || closed {
                            break;
                        }
                    }
                });
            })
    }

    /// The session layer's lock state from the core's, as the follower sets it.
    fn sync_lock_state(&self) {
        let _held = self
            .lock_sync
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        self.manager.set_unlocked(self.core.is_unlocked());
    }

    /// Opens an SFTP session to `target` (UI spec 10): the prompts of an SSH
    /// session come as its events; returns its handle.
    pub fn open_sftp(&self, target: SessionTarget) -> Result<u64, EngineError> {
        self.sync_lock_state();
        let opened = self
            .manager
            .open_sftp(target)
            .map_err(|e| error(e.message()))?;
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        if self.forward(id, opened.events).is_err() {
            opened.handle.close();
            return Err(error("no thread for the session's events"));
        }
        self.sftp.add(id, opened.handle);
        Ok(id)
    }

    /// One request of SFTP session `id` ([`SftpSessions::call`]).
    pub fn sftp_call(&self, id: u64, command: &str, input: &[u8]) -> String {
        self.sftp.call(id, command, input)
    }

    /// Queues a download of `remote` into the cache file `local`.
    pub fn sftp_download(
        &self,
        id: u64,
        remote: String,
        local: PathBuf,
    ) -> Result<u64, EngineError> {
        self.sftp
            .download(id, remote, local)
            .map_err(|SftpFailure { message, .. }| error(message))
    }

    /// Queues an upload of the cache file `local` to `remote`.
    pub fn sftp_upload(&self, id: u64, local: PathBuf, remote: String) -> Result<u64, EngineError> {
        self.sftp
            .upload(id, local, remote)
            .map_err(|SftpFailure { message, .. }| error(message))
    }

    /// Cancels a transfer.
    pub fn transfer_cancel(&self, transfer: u64) {
        self.sftp.cancel(transfer);
    }

    /// The next event (the core's or a session's) as JSON, waiting at most
    /// `timeout`.
    pub fn next_event(&self, timeout: Duration) -> Option<String> {
        self.events.next(timeout)
    }

    /// Answers the prompt `prompt` of session `id`: `kind` is "accept",
    /// "once", "replace", "cancel" (host keys and any prompt) or "text" with
    /// `text` (passwords, passphrases, usernames, codes).
    pub fn reply(
        &self,
        id: u64,
        prompt: u64,
        kind: &str,
        text: Option<SecretString>,
    ) -> Result<(), EngineError> {
        let reply = match (kind, text) {
            ("accept", _) => PromptReply::HostKey(HostKeyChoice::AcceptAndSave),
            ("once", _) => PromptReply::HostKey(HostKeyChoice::ConnectOnce),
            ("replace", _) => PromptReply::HostKey(HostKeyChoice::Replace),
            ("text", Some(value)) => PromptReply::Text { value, save: false },
            ("cancel", _) => PromptReply::Cancel,
            _ => return Err(error("unknown reply")),
        };
        let session = match self.with(id, |live| live.session.clone()) {
            Ok(session) => session,
            Err(missing) => self.sftp.handle(id).ok_or(missing)?,
        };
        session
            .reply(prompt, reply)
            .map_err(|e| error(e.to_string()))
    }

    /// Bytes for the session as typed input (keys, text, walks).
    pub fn send(&self, id: u64, bytes: Vec<u8>) -> Result<(), EngineError> {
        let terminal = self.with(id, |live| live.terminal.clone())?;
        terminal.send_typed(bytes);
        Ok(())
    }

    /// Bytes for the session that are not typing (mouse reports).
    pub fn send_raw(&self, id: u64, bytes: Vec<u8>) -> Result<(), EngineError> {
        let terminal = self.with(id, |live| live.terminal.clone())?;
        terminal.send_input(bytes);
        Ok(())
    }

    /// A new size of the terminal; the session tells the server.
    pub fn resize(&self, id: u64, cols: u16, rows: u16) -> Result<(), EngineError> {
        let terminal = self.with(id, |live| live.terminal.clone())?;
        terminal.resize(TerminalSize::cells(cols.max(1), rows.max(1)));
        Ok(())
    }

    /// Scrolls the view of session `id` by `lines` (up into the scrollback
    /// when positive); 0 goes back to the bottom.
    pub fn scroll(&self, id: u64, lines: i32) -> Result<(), EngineError> {
        let terminal = self.terminal(id)?;
        if lines == 0 {
            terminal.scroll_to_bottom();
        } else {
            terminal.scroll(alacritty_terminal::grid::Scroll::Delta(lines));
        }
        Ok(())
    }

    /// The terminal of session `id`.
    pub fn terminal(&self, id: u64) -> Result<TerminalHandle, EngineError> {
        self.with(id, |live| live.terminal.clone())
    }

    /// Waits until the terminal of session `id` has something new to show;
    /// `false` when it is gone (the pump thread ends).
    pub fn await_wake(&self, id: u64) -> bool {
        let Ok(woken) = self.with(id, |live| live.woken.clone()) else {
            return false;
        };
        woken.recv_blocking().is_ok()
    }

    /// Wakes one waiter of [`Engine::await_wake`] for session `id`. The wake
    /// channel has one receiver per session, shared by every waiter: a
    /// screen let go (Android recreates its activity) hands a wake it took
    /// on to the screen that replaced it.
    pub fn wake(&self, id: u64) {
        let _ = self.with(id, |live| live.wake.try_send(()));
    }

    /// Puts the cursor of session `id` on `target` ([`cursor::put`]).
    /// Blocks the calling thread for the walk.
    pub fn put_cursor(
        &self,
        id: u64,
        target: Cell,
        step_wait: Duration,
    ) -> Result<bool, EngineError> {
        Ok(cursor::put(&self.terminal(id)?, target, step_wait))
    }

    /// Closes session `id` and its terminal (or its transfers).
    pub fn close(&self, id: u64) {
        if self.sftp.contains(id) {
            self.sftp.close(id);
            return;
        }
        if let Some(live) = self.sessions().remove(&id) {
            live.session.close();
            live.terminal.close();
        }
    }

    /// The network came back (Android's default network, M2): a dropped
    /// terminal session tries again now instead of at the end of its
    /// countdown. Returns how many did.
    pub fn reconnect_dropped(&self) -> usize {
        let dropped: Vec<SessionHandle> = self
            .sessions()
            .values()
            .filter(|live| live.session.view().state == SessionState::Dropped)
            .map(|live| live.session.clone())
            .collect();
        for session in &dropped {
            session.reconnect();
        }
        dropped.len()
    }

    /// Ids of the sessions not closed yet (terminals and SFTP).
    pub fn open_sessions(&self) -> Vec<u64> {
        let mut ids: Vec<u64> = self.sessions().keys().copied().collect();
        ids.extend(self.sftp.ids());
        ids
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        for (_, live) in std::mem::take(&mut *self.sessions()) {
            live.session.close();
            live.terminal.close();
        }
    }
}

fn state_name(state: SessionState) -> &'static str {
    match state {
        SessionState::Connecting => "connecting",
        SessionState::Connected => "connected",
        SessionState::Ended => "ended",
        SessionState::Dropped => "dropped",
        SessionState::Failed => "failed",
        SessionState::Closed => "closed",
    }
}

/// The JSON the Kotlin side reads for a session's view: its state, the
/// question waiting (with the host key's fingerprint), the error and the
/// reconnect countdown. Server texts are passed as they are; the view
/// shows them as text, never as markup.
pub fn view_json(id: u64, view: &SessionView) -> Value {
    let prompt = view.prompt.as_ref().map(|prompt| {
        let mut out = json!({ "id": prompt.id, "kind": prompt.kind.name() });
        match &prompt.kind {
            PromptKind::HostKeyUnknown { key } => {
                out["keyType"] = json!(key.key_type);
                out["fingerprint"] = json!(key.fingerprint);
            }
            PromptKind::HostKeyChanged {
                key,
                old_key_type,
                old_fingerprint,
            } => {
                out["keyType"] = json!(key.key_type);
                out["fingerprint"] = json!(key.fingerprint);
                out["oldKeyType"] = json!(old_key_type);
                out["oldFingerprint"] = json!(old_fingerprint);
            }
            PromptKind::Password {
                username, error, ..
            } => {
                out["username"] = json!(username);
                out["error"] = json!(error);
            }
            PromptKind::Passphrase {
                key_label, error, ..
            } => {
                out["keyLabel"] = json!(key_label);
                out["error"] = json!(error);
            }
            PromptKind::KeyboardInteractive {
                name,
                instruction,
                prompt,
                echo,
                error,
                ..
            } => {
                out["name"] = json!(name);
                out["instruction"] = json!(instruction);
                out["prompt"] = json!(prompt);
                out["echo"] = json!(echo);
                out["error"] = json!(error);
            }
            PromptKind::Username | PromptKind::AgentSignature { .. } => {}
        }
        out
    });
    json!({
        "kind": "session",
        "id": id,
        "state": state_name(view.state),
        "title": view.title,
        "prompt": prompt,
        "error": view.error.map(|error| error.message()),
        "exitStatus": view.exit_status,
        "reconnectIn": view
            .reconnect
            .as_ref()
            .and_then(|reconnect| reconnect.seconds_left(Instant::now())),
    })
}
