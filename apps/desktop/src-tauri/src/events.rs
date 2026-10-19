//! Broadcast events (docs/architecture.md, section 3.5): the core emits every
//! event through [`EventSink`] after its change is committed, with a state
//! version `v` that increases with every event; this sink forwards it to all
//! windows under [`CoreEvent::name`] with [`CoreEvent::payload`].
//!
//! The sink also follows `lock:changed` to release `shellpp://` links that
//! waited for unlock. It never calls back into the core: the core may emit
//! while holding its own lock.

use std::sync::Arc;

use shellpp_core::platform::EventSink;
use shellpp_core::service::dto::{CoreEvent, CoreEventKind, LockState};
use tauri::{AppHandle, Emitter, Runtime};

use crate::links::{LINK_OPEN_EVENT, Link, LinkQueue};
use crate::windows::WindowRegistry;

/// Forwards core events to every window.
pub struct TauriEventSink<R: Runtime> {
    app: AppHandle<R>,
    links: Arc<LinkQueue>,
    windows: Arc<WindowRegistry>,
}

impl<R: Runtime> std::fmt::Debug for TauriEventSink<R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TauriEventSink")
    }
}

impl<R: Runtime> TauriEventSink<R> {
    pub fn new(app: AppHandle<R>, links: Arc<LinkQueue>, windows: Arc<WindowRegistry>) -> Self {
        Self {
            app,
            links,
            windows,
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
        if let CoreEventKind::LockChanged { state, .. } = &event.kind {
            let ready = self.links.set_unlocked(*state == LockState::Unlocked);
            deliver_links(&self.app, &self.links, &self.windows, ready);
        }
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
