//! Reconnect policy (UI 4.6, D18; docs/architecture.md, section 4.2, step 8):
//! attempts after 1, 2, 4, 8, 16 and 32 seconds, then every 60 seconds. The
//! loop itself (lock waiting, countdown events, cancel) runs in the desktop
//! session manager.

use std::time::Duration;

/// Delays of the first attempts, in seconds.
pub const RECONNECT_DELAYS_S: [u64; 6] = [1, 2, 4, 8, 16, 32];
/// Delay of every later attempt, in seconds.
pub const RECONNECT_STEADY_S: u64 = 60;

/// Backoff state of one dropped session.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Backoff {
    attempt: u32,
}

impl Backoff {
    pub fn new() -> Self {
        Self::default()
    }

    /// The delay before the next attempt; advances the attempt counter.
    /// 1, 2, 4, 8, 16, 32, 60, 60, ... seconds.
    pub fn next_delay(&mut self) -> Duration {
        let index = self.attempt as usize;
        self.attempt = self.attempt.saturating_add(1);
        let seconds = RECONNECT_DELAYS_S
            .get(index)
            .copied()
            .unwrap_or(RECONNECT_STEADY_S);
        Duration::from_secs(seconds)
    }

    /// Number of delays handed out so far (= the number of the next attempt minus one).
    pub fn attempt(&self) -> u32 {
        self.attempt
    }

    /// Back to the first delay (after a successful reconnect).
    pub fn reset(&mut self) {
        self.attempt = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delays_follow_the_policy() {
        let mut backoff = Backoff::new();
        let delays: Vec<u64> = (0..9).map(|_| backoff.next_delay().as_secs()).collect();
        assert_eq!(delays, vec![1, 2, 4, 8, 16, 32, 60, 60, 60]);
        assert_eq!(backoff.attempt(), 9);
        backoff.reset();
        assert_eq!(backoff.next_delay(), Duration::from_secs(1));
    }
}
