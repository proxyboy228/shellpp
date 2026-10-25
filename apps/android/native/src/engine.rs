//! The process-wide state of the prototype: the core on a throwaway vault,
//! the session layer, and the open sessions with their terminals.
//!
//! The vault lives in `<files>/prototype`, is made again at every start with
//! a random password nobody knows, and holds only what a quick connect
//! writes (known hosts, the connection log): "no vault and no sync" for the
//! prototype (plan 3.7) on the real core, so host key checks and prompts are
//! the desktop's. The first version replaces it with the user's vault.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use alacritty_terminal::vte::ansi::CursorShape;
use futures::StreamExt as _;
use secrecy::SecretString;
use serde_json::{Value, json};
use shellpp_core::platform::memory::{MemoryClipboard, MemorySecretStore, RecordingLauncher};
use shellpp_core::platform::{EventSink, OsKind, Platform, PlatformClass, SystemClock};
use shellpp_core::service::dto::{CoreEvent, SessionTarget};
use shellpp_core::{Core, CoreConfig};
use shellpp_sessions::{
    AgentSource, BoxFuture, HostKeyChoice, KeySources, ManagerOptions, OpenRequest, PromptKind,
    PromptReply, SessionCore, SessionEvent, SessionHandle, SessionManager, SessionState,
    SessionTerminal, SessionView, TerminalClosed,
};
use shellpp_terminal::model::{ModelOptions, TerminalSize};
use shellpp_terminal::{TerminalHandle, TerminalSink};

use crate::cursor;
use crate::input::Cell;

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

/// The core's events go nowhere in the prototype (no screen shows them).
struct NoEvents;

impl EventSink for NoEvents {
    fn emit(&self, _event: &CoreEvent) {}
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
    manager: SessionManager,
    key_dir: PathBuf,
    sessions: Mutex<BTreeMap<u64, Live>>,
    next: AtomicU64,
    events_tx: mpsc::Sender<String>,
    events_rx: Mutex<mpsc::Receiver<String>>,
}

impl std::fmt::Debug for Engine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Engine").finish_non_exhaustive()
    }
}

impl Engine {
    /// Opens the core on a new throwaway vault under `files` and starts the
    /// session layer. Key files for quick connects are offered from
    /// `files/keys` (the app copies a picked key there).
    pub fn start(files: &Path) -> Result<Self, EngineError> {
        let data_dir = files.join("prototype");
        if data_dir.exists() {
            std::fs::remove_dir_all(&data_dir).map_err(|e| error(format!("old vault: {e}")))?;
        }
        std::fs::create_dir_all(&data_dir).map_err(|e| error(format!("data folder: {e}")))?;
        // A key file picked for a quick connect lasts one run, as the vault
        // does (review of run 13): never left on disk for later servers.
        let key_dir = files.join("keys");
        if key_dir.exists() {
            std::fs::remove_dir_all(&key_dir).map_err(|e| error(format!("old keys: {e}")))?;
        }
        std::fs::create_dir_all(&key_dir).map_err(|e| error(format!("key folder: {e}")))?;
        let platform = Platform {
            secret_store: Arc::new(MemorySecretStore::new()),
            clipboard: Arc::new(MemoryClipboard::new()),
            launcher: Arc::new(RecordingLauncher::new()),
            events: Arc::new(NoEvents),
            clock: Arc::new(SystemClock),
            class: PlatformClass::Mobile,
            os: OsKind::Android,
            device_name: "Android".to_owned(),
        };
        let core = Core::open(
            CoreConfig {
                data_dir,
                app_version: env!("CARGO_PKG_VERSION").to_owned(),
            },
            platform,
        )
        .map_err(|e| error(format!("core: {}", e.code())))?;
        let mut password = [0u8; 24];
        getrandom::fill(&mut password).map_err(|e| error(format!("random: {e}")))?;
        let password: String = password.iter().map(|byte| format!("{byte:02x}")).collect();
        core.first_run_setup(SecretString::from(password))
            .map_err(|e| error(format!("vault: {}", e.code())))?;
        let core = Arc::new(core);
        let options = ManagerOptions::with_keys(KeySources {
            key_dir: Some(key_dir.clone()),
            agent: AgentSource::None,
        });
        let manager = SessionManager::start(core.clone() as Arc<dyn SessionCore>, options)
            .map_err(|e| error(format!("sessions: {e}")))?;
        let (events_tx, events_rx) = mpsc::channel();
        Ok(Self {
            core,
            manager,
            key_dir,
            sessions: Mutex::new(BTreeMap::new()),
            next: AtomicU64::new(1),
            events_tx,
            events_rx: Mutex::new(events_rx),
        })
    }

    /// Where a key file for quick connects goes.
    pub fn key_dir(&self) -> &Path {
        &self.key_dir
    }

    /// The core (tests read its known hosts).
    pub fn core(&self) -> &Arc<Core> {
        &self.core
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

    /// Opens a quick-connect SSH session to `address` in a terminal of
    /// `cols` × `rows`, in the dark or light theme; returns its handle.
    /// Every change of the session comes from [`Self::next_event`].
    pub fn open(
        &self,
        address: &str,
        port: Option<u16>,
        username: Option<&str>,
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
            crate::theme::palette(dark),
            wake.clone(),
        );
        let target = SessionTarget::Quick {
            address: address.trim().to_owned(),
            port,
            username: username.map(str::to_owned).filter(|name| !name.is_empty()),
        };
        let opened = self
            .manager
            .open(
                OpenRequest::Ssh(target),
                (cols, rows),
                Arc::new(PhoneTerminal(terminal.clone())),
            )
            .map_err(|e| error(e.message()))?;
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let mut events = opened.events;
        let tx = self.events_tx.clone();
        // The session's events, as JSON, into the engine's queue: a thread of
        // its own waits on the session's stream.
        let forward = std::thread::Builder::new()
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
            });
        if forward.is_err() {
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

    /// The next session event as JSON, waiting at most `timeout`.
    pub fn next_event(&self, timeout: Duration) -> Option<String> {
        let events = self
            .events_rx
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        events.recv_timeout(timeout).ok()
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
        let session = self.with(id, |live| live.session.clone())?;
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

    /// Closes session `id` and its terminal.
    pub fn close(&self, id: u64) {
        if let Some(live) = self.sessions().remove(&id) {
            live.session.close();
            live.terminal.close();
        }
    }

    /// Ids of the sessions not closed yet.
    pub fn open_sessions(&self) -> Vec<u64> {
        self.sessions().keys().copied().collect()
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
