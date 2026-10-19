//! The unlock attempt limit (decision D29; UI 13.2; docs/security.md,
//! section 8): the first four wrong master passwords in a row cost nothing
//! but the Argon2id run; the fifth ([`FAILURES_BEFORE_PAUSE`]) starts a pause
//! of [`FIRST_PAUSE`], and every further wrong password doubles it, up to
//! [`MAX_PAUSE`]. During a pause the core refuses `unlock` before any key
//! derivation (`too_many_attempts`). A successful unlock resets the counter.
//!
//! The counter is not secret: it is kept in the plaintext `meta` table under
//! [`META_UNLOCK_LOCKOUT`] as `{"failures": n, "retry_at_ms": t}` (Unix
//! milliseconds), so a restart does not reset it. It brakes guessing through
//! the app's own unlock screen only; an attacker with the database file
//! does not use the app (docs/security.md, section 8).

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

use crate::model::time::unix_ms;

/// The number of wrong passwords in a row that starts the first pause.
pub const FAILURES_BEFORE_PAUSE: u32 = 5;
/// The pause after the fifth wrong password in a row.
pub const FIRST_PAUSE: Duration = Duration::from_secs(5);
/// The longest pause.
pub const MAX_PAUSE: Duration = Duration::from_secs(5 * 60);
/// `meta` key of the counter (plaintext JSON).
pub const META_UNLOCK_LOCKOUT: &str = "unlock_lockout";

/// Consecutive wrong master passwords and the end of the current pause.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Lockout {
    failures: u32,
    retry_at: Option<SystemTime>,
}

impl Lockout {
    /// Wrong passwords since the last successful unlock.
    pub fn failures(&self) -> u32 {
        self.failures
    }

    /// The pause that follows the `failures`-th wrong password in a row:
    /// none for the first four, then 5 s, 10 s, 20 s, ... at most 5 minutes.
    pub fn pause_after(failures: u32) -> Option<Duration> {
        let extra = failures.checked_sub(FAILURES_BEFORE_PAUSE)?;
        // 2^6 * 5 s = 320 s is already above the cap.
        let factor = 1u32 << extra.min(6);
        Some((FIRST_PAUSE * factor).min(MAX_PAUSE))
    }

    /// The time left in the current pause, `None` when an attempt is allowed.
    ///
    /// A pause end further away than the counter allows (the clock was set
    /// back, or the stored value was edited) is moved to `now` plus the
    /// allowed pause, so the remaining time always counts down.
    pub fn remaining(&mut self, now: SystemTime) -> Option<Duration> {
        let retry_at = self.retry_at?;
        let left = match retry_at.duration_since(now) {
            Ok(left) if !left.is_zero() => left,
            _ => return None,
        };
        let allowed = Self::pause_after(self.failures).unwrap_or_default();
        if left > allowed {
            self.retry_at = now.checked_add(allowed);
            return (!allowed.is_zero()).then_some(allowed);
        }
        Some(left)
    }

    /// Counts a wrong password at `now` and starts the pause it earns.
    pub fn record_failure(&mut self, now: SystemTime) {
        self.failures = self.failures.saturating_add(1);
        self.retry_at = Self::pause_after(self.failures).and_then(|pause| now.checked_add(pause));
    }

    /// Forgets the counter after a successful unlock. `true` when it changed.
    pub fn reset(&mut self) -> bool {
        let changed = *self != Self::default();
        *self = Self::default();
        changed
    }

    /// The `meta` value.
    pub fn to_meta(&self) -> String {
        match self.retry_at {
            Some(retry_at) => {
                json!({ "failures": self.failures, "retry_at_ms": unix_ms(retry_at) }).to_string()
            }
            None => json!({ "failures": self.failures }).to_string(),
        }
    }

    /// Reads the `meta` value; anything unreadable is an empty counter.
    pub fn from_meta(text: Option<&str>) -> Self {
        let Some(value) = text.and_then(|text| serde_json::from_str::<Value>(text).ok()) else {
            return Self::default();
        };
        let failures = value
            .get("failures")
            .and_then(Value::as_u64)
            .and_then(|n| u32::try_from(n).ok())
            .unwrap_or(0);
        let retry_at = value
            .get("retry_at_ms")
            .and_then(Value::as_u64)
            .and_then(|ms| UNIX_EPOCH.checked_add(Duration::from_millis(ms)));
        Self { failures, retry_at }
    }
}

/// Whole seconds left, rounded up (what the unlock screen counts down).
pub fn whole_seconds(left: Duration) -> u64 {
    let secs = left.as_secs();
    if left.subsec_nanos() > 0 {
        secs + 1
    } else {
        secs
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(secs: u64) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(secs)
    }

    #[test]
    fn pauses_start_at_the_fifth_wrong_password_and_double_up_to_five_minutes() {
        let pauses: Vec<Option<u64>> = (0..=13)
            .map(|n| Lockout::pause_after(n).map(|pause| pause.as_secs()))
            .collect();
        assert_eq!(
            pauses,
            vec![
                None,
                None,
                None,
                None,
                None,
                Some(5),
                Some(10),
                Some(20),
                Some(40),
                Some(80),
                Some(160),
                Some(300),
                Some(300),
                Some(300)
            ]
        );
        assert_eq!(Lockout::pause_after(u32::MAX), Some(MAX_PAUSE));
    }

    #[test]
    fn a_pause_counts_down_and_a_success_resets() {
        let mut lockout = Lockout::default();
        for second in 0..4 {
            lockout.record_failure(at(100 + second));
            assert_eq!(lockout.remaining(at(100 + second)), None);
        }
        lockout.record_failure(at(110));
        assert_eq!(lockout.failures(), 5);
        assert_eq!(lockout.remaining(at(110)), Some(Duration::from_secs(5)));
        assert_eq!(lockout.remaining(at(113)), Some(Duration::from_secs(2)));
        assert_eq!(lockout.remaining(at(115)), None);
        lockout.record_failure(at(120));
        assert_eq!(lockout.remaining(at(120)), Some(Duration::from_secs(10)));
        assert!(lockout.reset());
        assert!(!lockout.reset());
        assert_eq!(lockout.remaining(at(121)), None);
        assert_eq!(lockout.failures(), 0);
    }

    #[test]
    fn a_clock_set_back_never_extends_the_pause() {
        let mut lockout = Lockout::default();
        for _ in 0..5 {
            lockout.record_failure(at(10_000));
        }
        // The clock went back an hour: the pause is the allowed 5 s again,
        // and it counts down from there.
        assert_eq!(lockout.remaining(at(6_400)), Some(Duration::from_secs(5)));
        assert_eq!(lockout.remaining(at(6_403)), Some(Duration::from_secs(2)));
        assert_eq!(lockout.remaining(at(6_405)), None);
    }

    #[test]
    fn meta_round_trip_and_unreadable_values() {
        let mut lockout = Lockout::default();
        assert_eq!(lockout.to_meta(), r#"{"failures":0}"#);
        for _ in 0..6 {
            lockout.record_failure(at(1_000));
        }
        let text = lockout.to_meta();
        assert_eq!(text, r#"{"failures":6,"retry_at_ms":1010000}"#);
        let mut read = Lockout::from_meta(Some(&text));
        assert_eq!(read, lockout);
        assert_eq!(read.remaining(at(1_004)), Some(Duration::from_secs(6)));
        assert_eq!(Lockout::from_meta(None), Lockout::default());
        assert_eq!(Lockout::from_meta(Some("not json")), Lockout::default());
        assert_eq!(
            Lockout::from_meta(Some(r#"{"failures":-1}"#)),
            Lockout::default()
        );
    }

    #[test]
    fn whole_seconds_round_up() {
        assert_eq!(whole_seconds(Duration::from_millis(4_001)), 5);
        assert_eq!(whole_seconds(Duration::from_secs(5)), 5);
        assert_eq!(whole_seconds(Duration::ZERO), 0);
    }
}
