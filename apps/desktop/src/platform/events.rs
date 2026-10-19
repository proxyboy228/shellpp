//! `EventSink` for the core: broadcast events go into a channel that the UI
//! thread drains ([`crate::state::AppState`] listens to it).
//!
//! The core emits while holding its own mutex, so the sink must never call
//! back into the core: it only queues the event.

use std::sync::{Arc, Mutex};

use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use shellpp_core::platform::EventSink;
use shellpp_core::service::dto::CoreEvent;

/// The receiving end, owned by the app state.
pub type EventReceiver = UnboundedReceiver<CoreEvent>;

/// Queues every core event for the UI thread.
pub struct ChannelEventSink {
    sender: Mutex<UnboundedSender<CoreEvent>>,
}

impl std::fmt::Debug for ChannelEventSink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ChannelEventSink")
    }
}

/// A new sink and the receiver of its events.
pub fn event_channel() -> (Arc<ChannelEventSink>, EventReceiver) {
    let (sender, receiver) = unbounded();
    (
        Arc::new(ChannelEventSink {
            sender: Mutex::new(sender),
        }),
        receiver,
    )
}

impl EventSink for ChannelEventSink {
    fn emit(&self, event: &CoreEvent) {
        let sender = self
            .sender
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        // A closed receiver means the UI is gone (shutdown); nothing to do.
        let _ = sender.unbounded_send(event.clone());
    }
}
