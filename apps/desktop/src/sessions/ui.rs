//! The session layer inside the GPUI app: one [`SessionManager`] as a
//! global that follows the lock state of [`AppState`], the link between a
//! session and the terminal view (`crate::terminal`), and a helper that
//! brings a session's events to the UI thread.
//!
//! ```ignore
//! // Startup, after the app state exists:
//! sessions::ui::install(&state, sessions::ui::options(KeySources::system()), cx);
//! // A tab: the terminal view first, then the session on its handle.
//! let terminal = view.read(cx).handle();
//! let opened = sessions::ui::manager(cx)?
//!     .open(OpenRequest::Ssh(target), (cols, rows), Arc::new(ViewTerminal(terminal)))?;
//! let task = sessions::ui::forward_events(opened.events, cx, |tab, view, cx| { .. });
//! // Quit, before `AppState::shutdown` (connection log ends need the core):
//! sessions::ui::shutdown(cx);
//! ```

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError, Weak};

use futures::StreamExt as _;
use futures::channel::mpsc::UnboundedReceiver;
use gpui::{App, Context, Entity, Global, Subscription, Task};
use shellpp_core::service::dto::{HistoryCandidate, LockState, RecordRef};
use shellpp_core::service::host_api::{HistoryDecision, HistoryRecordInput};
use uuid::Uuid;

use super::{
    BoxFuture, KeySources, ManagerOptions, SessionCore, SessionEvent, SessionHandle, SessionKind,
    SessionManager, SessionTerminal, SessionView, TerminalClosed,
};
use crate::state::{AppState, AppStateEvent};
use crate::terminal::{HistorySink, HistorySource, TerminalHandle, TerminalSink, TerminalSize};

/// The app's session layer.
pub struct Sessions {
    manager: SessionManager,
    _lock: Subscription,
    /// "Record command history" changed in Settings > Terminal.
    _settings: Subscription,
}

impl std::fmt::Debug for Sessions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Sessions")
            .field("manager", &self.manager)
            .finish_non_exhaustive()
    }
}

impl Global for Sessions {}

impl Sessions {
    pub fn manager(&self) -> &SessionManager {
        &self.manager
    }
}

/// The session layer's options in this app (review of run 13): `keys`,
/// the local terminal's title in the app's words, and the system's
/// security keys (D143). Every manager the app starts takes its options
/// from here, so none goes without the WebAuthn authenticator.
pub fn options(keys: KeySources) -> ManagerOptions {
    ManagerOptions {
        local_title: crate::i18n::sessions::LOCAL_TERMINAL.to_owned(),
        // Security keys sign through the system (D143).
        authenticator: crate::platform::security_key::authenticator,
        ..ManagerOptions::with_keys(keys)
    }
}

/// Starts the session layer for the app's core and makes it follow the
/// lock state (UI 13.3). `false` when there is no core (it did not open)
/// or the runtime did not start.
pub fn install(state: &Entity<AppState>, options: ManagerOptions, cx: &mut App) -> bool {
    let Some(core) = state.read(cx).core().cloned() else {
        return false;
    };
    let manager = match SessionManager::start(core, options) {
        Ok(manager) => manager,
        Err(_) => {
            tracing::error!("the session runtime did not start");
            return false;
        }
    };
    // Hosts (their "Record history") and the lock change the history flag
    // of running sessions; history and connection log writes do not.
    let mut records = state.read(cx).records_generation();
    let lock = cx.subscribe(state, move |state, event: &AppStateEvent, cx| match event {
        AppStateEvent::LockChanged(lock) => {
            if cx.has_global::<Sessions>() {
                cx.global::<Sessions>()
                    .manager
                    .set_unlocked(*lock == LockState::Unlocked);
            }
            refresh_history_flags();
            forget_suggestions();
        }
        AppStateEvent::RecordsChanged => {
            // History may have been deleted or cleared (autocomplete, D142).
            forget_suggestions();
            let generation = state.read(cx).records_generation();
            if generation != records {
                records = generation;
                refresh_history_flags();
            }
        }
        AppStateEvent::SelectionChanged
        | AppStateEvent::LocationChanged(_)
        | AppStateEvent::AutoLockDue => {}
    });
    let settings =
        cx.observe_global::<crate::settings::terminal::TerminalPrefs>(|_| refresh_history_flags());
    // The Serial dialog lists the ports of the same backend (D88).
    cx.set_global(crate::shell::serial_dialog::SerialPorts(
        manager.serial_backend(),
    ));
    cx.set_global(Sessions {
        manager,
        _lock: lock,
        _settings: settings,
    });
    true
}

/// The session manager, once [`install`] ran.
pub fn manager(cx: &App) -> Option<&SessionManager> {
    cx.has_global::<Sessions>()
        .then(|| &cx.global::<Sessions>().manager)
}

/// Closes every session and ends their connection log entries. Call it
/// when the app quits, before the core shuts down.
pub fn shutdown(cx: &mut App) {
    if cx.has_global::<Sessions>() {
        cx.global_mut::<Sessions>().manager.shutdown();
    }
}

/// Applies the events of a session to the entity of `cx` on the UI thread,
/// in order, until the session closes or the entity is gone. Views queued
/// meanwhile are coalesced: each one is complete, so only the newest is
/// applied.
pub fn forward_events<T: 'static>(
    mut events: UnboundedReceiver<SessionEvent>,
    cx: &mut Context<T>,
    mut apply: impl FnMut(&mut T, SessionView, &mut Context<T>) + 'static,
) -> Task<()> {
    cx.spawn(async move |this, cx| {
        while let Some(SessionEvent::Changed(mut view)) = events.next().await {
            while let Ok(SessionEvent::Changed(newer)) = events.try_recv() {
                view = newer;
            }
            if this.update(cx, |this, cx| apply(this, view, cx)).is_err() {
                break;
            }
        }
    })
}

/// The terminal view's handle as a session's terminal: output goes to its
/// parser queue (which waits while full: flow control), and the session
/// becomes its sink for input and size changes. A wrapper: the handle and
/// the trait live in two crates (run 13).
#[derive(Clone, Debug)]
pub struct ViewTerminal(pub TerminalHandle);

impl SessionTerminal for ViewTerminal {
    fn inject(&self, bytes: Vec<u8>) {
        self.0.inject_input(bytes);
    }

    fn feed<'a>(&'a self, bytes: &'a [u8]) -> BoxFuture<'a, Result<(), TerminalClosed>> {
        Box::pin(async move { self.0.feed(bytes).await.map_err(|_| TerminalClosed) })
    }

    fn attach(&self, session: SessionHandle) {
        // History is recorded for sessions to saved hosts only (stage 2b
        // lead decision 7): not for quick connects or local terminals.
        let view = session.view();
        let (history, source) = match (view.kind, view.host) {
            (SessionKind::Ssh, Some(host)) => (
                Some(
                    HistoryRecorder::start(session.clone(), host, self.0.autocomplete())
                        as Arc<dyn HistorySink>,
                ),
                // Autocomplete (D142) suggests from the same host's history.
                Some(Arc::new(HostHistory {
                    core: session.core(),
                    host,
                }) as Arc<dyn HistorySource>),
            ),
            _ => (None, None),
        };
        if source.is_some() {
            register_suggestions(&self.0.autocomplete());
        }
        self.0.set_history_sink(history);
        self.0.set_history_source(source);
        self.0.set_sink(Some(Arc::new(SessionSink(session))));
    }
}

/// The local history of a session's saved host, for autocomplete (D142).
struct HostHistory {
    core: Arc<dyn SessionCore>,
    host: RecordRef,
}

impl HistorySource for HostHistory {
    fn commands(&self) -> Vec<String> {
        self.core.history_commands(self.host)
    }
}

/// The autocomplete lists of running sessions' terminals (D142).
static SUGGESTIONS: Mutex<Vec<Weak<crate::terminal::autocomplete::Slot>>> = Mutex::new(Vec::new());

fn register_suggestions(slot: &Arc<crate::terminal::autocomplete::Slot>) {
    let mut list = SUGGESTIONS.lock().unwrap_or_else(PoisonError::into_inner);
    list.retain(|slot| slot.strong_count() > 0);
    list.push(Arc::downgrade(slot));
}

/// Drops the autocomplete lists of every terminal (the lock changed, or
/// records changed: history may have been deleted); each is read again
/// when next needed (docs/security.md, section 8).
pub fn forget_suggestions() {
    let slots: Vec<Arc<crate::terminal::autocomplete::Slot>> = {
        let mut list = SUGGESTIONS.lock().unwrap_or_else(PoisonError::into_inner);
        list.retain(|slot| slot.strong_count() > 0);
        list.iter().filter_map(Weak::upgrade).collect()
    };
    for slot in slots {
        slot.invalidate();
    }
}

/// The recorders of running sessions, so that a change of hosts, settings
/// or the lock refreshes their "Record history" flag.
static RECORDERS: Mutex<Vec<Weak<HistoryRecorder>>> = Mutex::new(Vec::new());

/// Reads "Record history" again for every running session to a saved host
/// (docs/security.md, section 10.1, rule 4), on blocking threads of the
/// session runtime. Called when hosts, the settings or the lock change.
pub fn refresh_history_flags() {
    let recorders: Vec<Arc<HistoryRecorder>> = {
        let mut list = RECORDERS.lock().unwrap_or_else(PoisonError::into_inner);
        list.retain(|recorder| recorder.strong_count() > 0);
        list.iter().filter_map(Weak::upgrade).collect()
    };
    for recorder in &recorders {
        recorder.refresh();
    }
}

/// Records the command lines of a session's terminal through the core, on
/// a blocking thread of the session runtime (the core checks every rule of
/// docs/security.md, section 10.1 again). Logs the decision only.
struct HistoryRecorder {
    session: SessionHandle,
    host: RecordRef,
    /// The core's "same as the previous entry of this session" rule.
    id: Uuid,
    /// "Record history" for the host now (the host's override, else the
    /// setting): while it is off, captured lines do not leave the terminal
    /// (rule 4). Read when the session starts and on every change.
    record: AtomicBool,
    /// The newest refresh; an older one that ends later is ignored.
    refreshes: AtomicU64,
    /// The terminal's autocomplete list: a recorded command joins it.
    suggestions: Arc<crate::terminal::autocomplete::Slot>,
}

impl HistoryRecorder {
    /// The recorder of a new session, registered for refreshes; its flag
    /// is read at once (the core checks it again for every line meanwhile).
    fn start(
        session: SessionHandle,
        host: RecordRef,
        suggestions: Arc<crate::terminal::autocomplete::Slot>,
    ) -> Arc<Self> {
        let recorder = Arc::new(Self {
            session,
            host,
            id: Uuid::now_v7(),
            record: AtomicBool::new(true),
            refreshes: AtomicU64::new(0),
            suggestions,
        });
        {
            let mut list = RECORDERS.lock().unwrap_or_else(PoisonError::into_inner);
            list.retain(|recorder| recorder.strong_count() > 0);
            list.push(Arc::downgrade(&recorder));
        }
        recorder.refresh();
        recorder
    }

    /// Reads the flag from the core on a blocking thread.
    fn refresh(self: &Arc<Self>) {
        let generation = self.refreshes.fetch_add(1, Ordering::AcqRel) + 1;
        let this = Arc::downgrade(self);
        let host = self.host;
        self.session.background(move |core| {
            let on = core.record_history_now(host);
            if let Some(this) = this.upgrade()
                && this.refreshes.load(Ordering::Acquire) == generation
            {
                this.record.store(on, Ordering::Release);
            }
        });
    }
}

/// The session ended (its terminal let go of the recorder): the core
/// forgets the session's previous command (section 12 review).
impl Drop for HistoryRecorder {
    fn drop(&mut self) {
        let id = self.id;
        self.session
            .background(move |core| core.history_session_end(id));
    }
}

impl HistorySink for HistoryRecorder {
    fn record(&self, candidate: HistoryCandidate) {
        if !self.record.load(Ordering::Acquire) {
            let mut candidate = candidate;
            zeroize::Zeroize::zeroize(&mut candidate.command);
            zeroize::Zeroize::zeroize(&mut candidate.prompt_text);
            return;
        }
        let input = HistoryRecordInput {
            host: Some(self.host),
            session_id: self.id,
            candidate,
            injected: false,
            record_history: true,
        };
        let session = self.session.id();
        let suggestions = self.suggestions.clone();
        self.session.background(move |core| {
            let command = zeroize::Zeroizing::new(input.candidate.command.clone());
            match core.history_record(input) {
                Ok(decision) => {
                    if decision == HistoryDecision::Recorded {
                        suggestions.remember(&command);
                    }
                    tracing::debug!(session, ?decision, "history candidate")
                }
                Err(error) => tracing::debug!(session, code = error.code(), "history not written"),
            }
        });
    }
}

/// Input, query replies and size changes of the terminal view go to the
/// session (never blocks: a queue push under a short lock). A wrapper, as
/// [`ViewTerminal`] is.
#[derive(Debug)]
pub struct SessionSink(pub SessionHandle);

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
