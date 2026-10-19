//! Minimal connection log (docs/data-model.md, section 13.3): one
//! `connection_log_entry` in the personal vault per established SSH session,
//! written at start and updated at the end; entries left open by a crash get
//! `end_reason = unknown` at the next unlock. Stage 2 uses it only for Recent
//! connections (UI 6); the Logs screen, retention and team copies are stage 6
//! and later.
//!
//! While locked nothing can be written (docs/storage-format.md, section 4.6):
//! end times are held in memory and written after the next unlock.
//!
//! The class is lazy (D10): its rows are decrypted on first use after each
//! unlock. The crash rule therefore runs when the class is first loaded in an
//! unlocked session, which happens at once after an unlock that has ends to
//! write, and otherwise at the first connection or Recent connections list.
//!
//! Owner: S4. The facade methods (`Core::connection_log_start`,
//! `Core::connection_log_end`, `Core::recent_connections`) are the contract.

use std::collections::HashSet;

use uuid::Uuid;

use crate::service::host_api::ConnectionEndReason;

/// Most rows of Recent connections (one per host link, or per address and
/// port for quick connects).
pub const RECENT_LIMIT: usize = 10;

/// `end_reason` of entries left open by a previous run of the app.
pub const UNKNOWN_END: &str = "unknown";

/// An end that waits for the next unlock.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingEnd {
    pub log_id: Uuid,
    pub ended_at: String,
    pub reason: ConnectionEndReason,
    pub exit_status: Option<u32>,
}

/// Entries of this process: started and not ended yet (`active`), and ends
/// that could not be written while locked (`pending`). Survives locks.
#[derive(Debug, Default)]
pub struct Tracker {
    active: HashSet<Uuid>,
    pending: Vec<PendingEnd>,
}

impl Tracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// A session of this process started with this entry.
    pub fn started(&mut self, log_id: Uuid) {
        self.active.insert(log_id);
    }

    /// `true` while the entry belongs to a running session of this process
    /// (or its end is still pending).
    pub fn is_active(&self, log_id: &Uuid) -> bool {
        self.active.contains(log_id) || self.pending.iter().any(|end| end.log_id == *log_id)
    }

    /// The end was written.
    pub fn ended(&mut self, log_id: &Uuid) {
        self.active.remove(log_id);
        self.pending.retain(|end| end.log_id != *log_id);
    }

    /// Keeps an end for the next unlock (the latest end of an entry wins).
    pub fn defer(&mut self, end: PendingEnd) {
        self.active.remove(&end.log_id);
        self.pending.retain(|pending| pending.log_id != end.log_id);
        self.pending.push(end);
    }

    pub fn has_pending(&self) -> bool {
        !self.pending.is_empty()
    }

    /// The pending ends, oldest first.
    pub fn pending(&self) -> &[PendingEnd] {
        &self.pending
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tracker_lifecycle() {
        let mut tracker = Tracker::new();
        let a = Uuid::from_u128(1);
        let b = Uuid::from_u128(2);
        tracker.started(a);
        tracker.started(b);
        assert!(tracker.is_active(&a));
        tracker.defer(PendingEnd {
            log_id: a,
            ended_at: "2026-10-18T22:31:05.123Z".into(),
            reason: ConnectionEndReason::Dropped,
            exit_status: None,
        });
        assert!(tracker.is_active(&a));
        assert!(tracker.has_pending());
        tracker.defer(PendingEnd {
            log_id: a,
            ended_at: "2026-10-18T22:32:00.000Z".into(),
            reason: ConnectionEndReason::Closed,
            exit_status: None,
        });
        assert_eq!(tracker.pending().len(), 1);
        assert_eq!(tracker.pending()[0].reason, ConnectionEndReason::Closed);
        tracker.ended(&a);
        assert!(!tracker.is_active(&a));
        assert!(!tracker.has_pending());
        tracker.ended(&b);
        assert!(!tracker.is_active(&b));
    }
}
