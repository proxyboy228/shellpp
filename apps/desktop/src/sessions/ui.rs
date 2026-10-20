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

use std::sync::Arc;

use futures::StreamExt as _;
use futures::channel::mpsc::UnboundedReceiver;
use gpui::{App, Context, Entity, Global, Subscription, Task};
use shellpp_core::service::dto::LockState;

use super::{
    BoxFuture, ManagerOptions, SessionEvent, SessionHandle, SessionManager, SessionTerminal,
    SessionView, TerminalClosed,
};
use crate::state::{AppState, AppStateEvent};
use crate::terminal::{TerminalHandle, TerminalSink, TerminalSize};

/// The app's session layer.
pub struct Sessions {
    manager: SessionManager,
    _lock: Subscription,
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
    let lock = cx.subscribe(state, |_, event: &AppStateEvent, cx| {
        if let AppStateEvent::LockChanged(lock) = event
            && cx.has_global::<Sessions>()
        {
            cx.global::<Sessions>()
                .manager
                .set_unlocked(*lock == LockState::Unlocked);
        }
    });
    cx.set_global(Sessions {
        manager,
        _lock: lock,
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
    fn feed<'a>(&'a self, bytes: &'a [u8]) -> BoxFuture<'a, Result<(), TerminalClosed>> {
        Box::pin(async move {
            TerminalHandle::feed(self, bytes)
                .await
                .map_err(|_| TerminalClosed)
        })
    }

    fn attach(&self, session: SessionHandle) {
        self.set_sink(Some(Arc::new(session)));
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
