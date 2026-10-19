//! Broadcast events (docs/architecture.md, section 3.5): the core emits every
//! event through [`EventSink`] after its change is committed, with a state
//! version `v` that increases with every event; this sink forwards it to all
//! windows under [`CoreEvent::name`] with [`CoreEvent::payload`].
//!
//! The sink also follows `lock:changed` to release `shellpp://` links that
//! waited for unlock and to tell the session manager (stage 2 plan,
//! decision 27). It never calls back into the core: the core may emit while
//! holding its own lock.
//!
//! Window-targeted session events (stage 2 plan, section 5.3):
//! `sessions:changed` (the whole `SessionInfo`, to the owning window only)
//! and `window:close-requested`.

use std::sync::{Arc, OnceLock};

use shellpp_core::model::types::RecordKind;
use shellpp_core::platform::EventSink;
use shellpp_core::service::dto::{CoreEvent, CoreEventKind, LockState, SessionInfo};
use tauri::{AppHandle, Emitter, EventTarget, Runtime};

use crate::links::{LINK_OPEN_EVENT, Link, LinkQueue};
use crate::sessions::{SessionEvents, SessionManager};
use crate::windows::WindowRegistry;

/// Window-targeted: every change of a session, to its window.
pub const SESSIONS_CHANGED_EVENT: &str = "sessions:changed";
/// Window-targeted: the OS asked to close a window that owns active sessions.
pub const WINDOW_CLOSE_REQUESTED_EVENT: &str = "window:close-requested";
/// The global "Record history" setting.
const RECORD_HISTORY_SETTING: &str = "terminal.record_history";

/// The session manager, set once after the core opened (the core needs this
/// sink before the manager can exist).
pub type SessionSlot = Arc<OnceLock<Arc<SessionManager>>>;

/// Forwards core events to every window.
pub struct TauriEventSink<R: Runtime> {
    app: AppHandle<R>,
    links: Arc<LinkQueue>,
    windows: Arc<WindowRegistry>,
    sessions: SessionSlot,
}

impl<R: Runtime> std::fmt::Debug for TauriEventSink<R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TauriEventSink")
    }
}

impl<R: Runtime> TauriEventSink<R> {
    pub fn new(
        app: AppHandle<R>,
        links: Arc<LinkQueue>,
        windows: Arc<WindowRegistry>,
        sessions: SessionSlot,
    ) -> Self {
        Self {
            app,
            links,
            windows,
            sessions,
        }
    }
}

impl<R: Runtime> EventSink for TauriEventSink<R> {
    fn emit(&self, event: &CoreEvent) {
        let name = event.name();
        if self.app.emit(name, event.payload()).is_err() {
            tracing::warn!(event = name, v = event.v, "event not delivered");
        } else {
            tracing::debug!(event = name, v = event.v, "event");
        }
        match &event.kind {
            CoreEventKind::LockChanged { state, .. } => {
                let unlocked = *state == LockState::Unlocked;
                if let Some(sessions) = self.sessions.get() {
                    sessions.set_unlocked(unlocked);
                }
                let ready = self.links.set_unlocked(unlocked);
                deliver_links(&self.app, &self.links, &self.windows, ready);
            }
            // A host's or the global "Record history" may have changed:
            // running sessions take the current value (security.md 10.1).
            CoreEventKind::RecordsChanged { classes, .. }
                if classes.contains(&RecordKind::Host.class()) =>
            {
                if let Some(sessions) = self.sessions.get() {
                    sessions.refresh_record_history();
                }
            }
            CoreEventKind::SettingsChanged { keys }
                if keys.iter().any(|key| key == RECORD_HISTORY_SETTING) =>
            {
                if let Some(sessions) = self.sessions.get() {
                    sessions.refresh_record_history();
                }
            }
            _ => {}
        }
    }
}

/// `sessions:changed` to the owning window.
pub struct TauriSessionEvents<R: Runtime> {
    app: AppHandle<R>,
}

impl<R: Runtime> std::fmt::Debug for TauriSessionEvents<R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TauriSessionEvents")
    }
}

impl<R: Runtime> TauriSessionEvents<R> {
    pub fn new(app: AppHandle<R>) -> Self {
        Self { app }
    }
}

impl<R: Runtime> SessionEvents for TauriSessionEvents<R> {
    fn session_changed(&self, info: &SessionInfo) {
        let target = EventTarget::webview_window(info.window.clone());
        if self
            .app
            .emit_to(target, SESSIONS_CHANGED_EVENT, info)
            .is_err()
        {
            tracing::warn!(session = %info.session_id, "session event not delivered");
        }
    }
}

/// `window:close-requested` to one window.
pub fn emit_close_requested<R: Runtime>(app: &AppHandle<R>, label: &str) {
    let target = EventTarget::webview_window(label.to_owned());
    if app
        .emit_to(target, WINDOW_CLOSE_REQUESTED_EVENT, serde_json::json!({}))
        .is_err()
    {
        tracing::warn!("close request not delivered");
    }
}

/// A link arrived (command line, second launch, e2e injection): delivered now
/// when unlocked, otherwise held until the next `lock:changed` to unlocked.
pub fn receive_link<R: Runtime>(
    app: &AppHandle<R>,
    links: &LinkQueue,
    windows: &WindowRegistry,
    link: Link,
) {
    match links.push(link) {
        Some(link) => deliver_links(app, links, windows, vec![link]),
        None => tracing::info!("link held until unlock"),
    }
}

/// Sends `link:open` to the focused or last focused ready window; links that
/// find no ready window wait for the next window snapshot.
pub fn deliver_links<R: Runtime>(
    app: &AppHandle<R>,
    links: &LinkQueue,
    windows: &WindowRegistry,
    ready: Vec<Link>,
) {
    let mut waiting = Vec::new();
    for link in ready {
        if windows.emit_to_target(app, LINK_OPEN_EVENT, link) {
            tracing::info!("link delivered");
        } else {
            waiting.push(link);
        }
    }
    if !waiting.is_empty() {
        tracing::info!(count = waiting.len(), "link waits for a window");
        links.requeue(waiting);
    }
}
