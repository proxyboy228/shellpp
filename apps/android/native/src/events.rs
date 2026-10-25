//! The events the app reads with `nextEvent` (architecture 3: Kotlin waits
//! on Rust instead of Rust calling back into the JVM): the core's broadcast
//! events, `{"kind": "core", "name", "payload"}`, and the session views,
//! `{"kind": "session", ...}` ([`crate::engine::view_json`]).

use std::sync::mpsc;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use serde_json::{Value, json};
use shellpp_core::platform::EventSink;
use shellpp_core::service::dto::{CoreEvent, CoreEventKind};

/// One queue of JSON events, filled from any thread.
pub struct EventQueue {
    tx: Mutex<mpsc::Sender<String>>,
    rx: Mutex<mpsc::Receiver<String>>,
    /// The lock state for the session layer (`true`: unlocked), handed to a
    /// thread of its own: the core emits under its own lock, which the
    /// session layer's `set_unlocked` takes again.
    lock_tx: Mutex<Option<mpsc::Sender<bool>>>,
}

impl std::fmt::Debug for EventQueue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EventQueue").finish_non_exhaustive()
    }
}

impl Default for EventQueue {
    fn default() -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            tx: Mutex::new(tx),
            rx: Mutex::new(rx),
            lock_tx: Mutex::new(None),
        }
    }
}

impl EventQueue {
    pub fn push(&self, event: &Value) {
        let tx = self.tx.lock().unwrap_or_else(PoisonError::into_inner);
        // The receiver lives as long as the queue.
        let _ = tx.send(event.to_string());
    }

    /// A sender for a thread of its own (a session's event forwarder).
    pub fn sender(&self) -> mpsc::Sender<String> {
        self.tx
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// From now on every lock change of the core goes to the receiver.
    pub fn follow_lock(&self) -> mpsc::Receiver<bool> {
        let (tx, rx) = mpsc::channel();
        *self.lock_tx.lock().unwrap_or_else(PoisonError::into_inner) = Some(tx);
        rx
    }

    /// The next event, waiting at most `timeout`.
    pub fn next(&self, timeout: Duration) -> Option<String> {
        self.rx
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .recv_timeout(timeout)
            .ok()
    }
}

impl EventSink for EventQueue {
    fn emit(&self, event: &CoreEvent) {
        if let CoreEventKind::LockChanged { locked, .. } = &event.kind
            && let Some(tx) = &*self.lock_tx.lock().unwrap_or_else(PoisonError::into_inner)
        {
            let _ = tx.send(!locked);
        }
        self.push(&json!({
            "kind": "core",
            "name": event.name(),
            "payload": event.payload(),
        }));
    }
}
