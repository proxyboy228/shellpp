//! Mock wall clock: system time plus an offset that `e2e_mock {kind:
//! "clock_advance"}` increases (auto-lock and clipboard clearing tests).

use std::sync::Mutex;
use std::time::{Duration, SystemTime};

use shellpp_core::platform::Clock;

/// The e2e clock.
#[derive(Debug, Default)]
pub struct ManualClock {
    offset: Mutex<Duration>,
}

impl ManualClock {
    /// Moves the clock forward.
    pub fn advance(&self, by: Duration) {
        let mut offset = self
            .offset
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *offset = offset.saturating_add(by);
    }

    /// The total offset applied so far.
    pub fn offset(&self) -> Duration {
        *self
            .offset
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

impl Clock for ManualClock {
    fn now(&self) -> SystemTime {
        SystemTime::now() + self.offset()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn advances() {
        let clock = ManualClock::default();
        let before = SystemTime::now();
        clock.advance(Duration::from_secs(61));
        let now = clock.now();
        assert!(now.duration_since(before).unwrap() >= Duration::from_secs(61));
        assert_eq!(clock.offset(), Duration::from_secs(61));
    }
}
