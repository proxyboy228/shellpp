//! The session layer inside the GPUI app: one [`SessionManager`] as a
//! global that follows the lock state of [`AppState`], the link between a
//! session and the terminal view (`crate::terminal`), and a helper that
//! brings a session's events to the UI thread.
//!
//! ```ignore
//! // Startup, after the app state exists:
//! sessions::ui::install(&state, ManagerOptions::production(), cx);
//! // A tab: the terminal view first, then the session on its handle.
//! let terminal = view.read(cx).handle();
//! let opened = sessions::ui::manager(cx)?
//!     .open(OpenRequest::Ssh(target), (cols, rows), Arc::new(terminal))?;
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
use shellpp_core::service::host_api::HistoryRecordInput;
use uuid::Uuid;

use super::{
    BoxFuture, ManagerOptions, SessionEvent, SessionHandle, SessionKind, SessionManager,
    SessionTerminal, SessionView, TerminalClosed,
};
use crate::state::{AppState, AppStateEvent};
use crate::terminal::{HistorySink, TerminalHandle, TerminalSink, TerminalSize};

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
        }
        AppStateEvent::RecordsChanged => {
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

/// The terminal view's handle is a session's terminal: output goes to its
/// parser queue (which waits while full: flow control), and the session
/// becomes its sink for input and size changes.
impl SessionTerminal for TerminalHandle {
    fn inject(&self, bytes: Vec<u8>) {
        self.inject_input(bytes);
    }

    fn feed<'a>(&'a self, bytes: &'a [u8]) -> BoxFuture<'a, Result<(), TerminalClosed>> {
        Box::pin(async move {
            TerminalHandle::feed(self, bytes)
                .await
                .map_err(|_| TerminalClosed)
        })
    }

    fn attach(&self, session: SessionHandle) {
        // History is recorded for sessions to saved hosts only (stage 2b
        // lead decision 7): not for quick connects or local terminals.
        let view = session.view();
        let history = match (view.kind, view.host) {
            (SessionKind::Ssh, Some(host)) => {
                Some(HistoryRecorder::start(session.clone(), host) as Arc<dyn HistorySink>)
            }
            _ => None,
        };
        self.set_history_sink(history);
        self.set_sink(Some(Arc::new(session)));
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
}

impl HistoryRecorder {
    /// The recorder of a new session, registered for refreshes; its flag
    /// is read at once (the core checks it again for every line meanwhile).
    fn start(session: SessionHandle, host: RecordRef) -> Arc<Self> {
        let recorder = Arc::new(Self {
            session,
            host,
            id: Uuid::now_v7(),
            record: AtomicBool::new(true),
            refreshes: AtomicU64::new(0),
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
        self.session.shared.background(move |core| {
            let on = core.record_history_now(host);
            if let Some(this) = this.upgrade()
                && this.refreshes.load(Ordering::Acquire) == generation
            {
                this.record.store(on, Ordering::Release);
            }
        });
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
        self.session
            .shared
            .background(move |core| match core.history_record(input) {
                Ok(decision) => tracing::debug!(session, ?decision, "history candidate"),
                Err(error) => tracing::debug!(session, code = error.code(), "history not written"),
            });
    }
}

/// Input, query replies and size changes of the terminal view go to the
/// session (never blocks: a queue push under a short lock).
impl TerminalSink for SessionHandle {
    fn send(&self, bytes: Vec<u8>) {
        if let Err(error) = self.write(&bytes) {
            tracing::debug!(session = self.id(), %error, "terminal input dropped");
        }
    }

    fn resize(&self, size: TerminalSize) {
        SessionHandle::resize(self, size.cols, size.rows);
    }
}
