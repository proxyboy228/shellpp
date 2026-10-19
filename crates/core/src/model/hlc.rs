//! Hybrid logical clock (docs/sync-protocol.md, section 6.2).
//!
//! Value: `(unix_ms << 16) | counter`, 48 bits of milliseconds and a 16-bit
//! counter; text form: 16 lowercase hex digits, so string order equals numeric
//! order.

use std::fmt;

/// The drift guard: a received value never moves the local clock more than
/// one hour past `now`.
pub const MAX_DRIFT_MS: u64 = 3_600_000;

const MS_MASK: u64 = (1 << 48) - 1;

/// One HLC value.
#[derive(Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Hlc(pub u64);

impl Hlc {
    pub fn new(ms: u64, counter: u16) -> Self {
        Self(((ms & MS_MASK) << 16) | u64::from(counter))
    }

    /// The milliseconds part.
    pub fn ms(self) -> u64 {
        self.0 >> 16
    }

    pub fn counter(self) -> u16 {
        (self.0 & 0xFFFF) as u16
    }

    /// 16 lowercase hex digits.
    pub fn to_text(self) -> String {
        format!("{:016x}", self.0)
    }

    /// Parses exactly 16 lowercase hex digits.
    pub fn parse(text: &str) -> Option<Self> {
        if text.len() != 16
            || !text
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return None;
        }
        u64::from_str_radix(text, 16).ok().map(Self)
    }
}

impl fmt::Debug for Hlc {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Hlc({})", self.to_text())
    }
}

/// The local clock of one unlocked session.
#[derive(Clone, Debug)]
pub struct HlcClock {
    last: Hlc,
}

impl HlcClock {
    pub fn new(last: Hlc) -> Self {
        Self { last }
    }

    /// Start value at unlock: the greatest HLC of the decrypted records or
    /// `now`, whichever is greater.
    pub fn start(max_seen: Option<Hlc>, now_ms: u64) -> Self {
        let now = Hlc::new(now_ms, 0);
        Self {
            last: max_seen.map_or(now, |seen| seen.max(now)),
        }
    }

    pub fn last(&self) -> Hlc {
        self.last
    }

    /// A local event (any semantic edit or deletion).
    pub fn tick(&mut self, now_ms: u64) -> Hlc {
        let now_ms = now_ms & MS_MASK;
        let last_ms = self.last.ms();
        let ms = last_ms.max(now_ms);
        let next = if ms == last_ms {
            bump(ms, u32::from(self.last.counter()) + 1)
        } else {
            Hlc::new(ms, 0)
        };
        self.last = next;
        next
    }

    /// Merges a received value (every opened record of another device).
    pub fn observe(&mut self, remote: Hlc, now_ms: u64) -> Hlc {
        let now_ms = now_ms & MS_MASK;
        let ceiling = Hlc::new(now_ms.saturating_add(MAX_DRIFT_MS), 0);
        let remote = remote.min(ceiling);
        let local = self.last;
        let ms = local.ms().max(remote.ms()).max(now_ms);
        let next = if ms == local.ms() && ms == remote.ms() {
            bump(ms, u32::from(local.counter().max(remote.counter())) + 1)
        } else if ms == local.ms() {
            bump(ms, u32::from(local.counter()) + 1)
        } else if ms == remote.ms() {
            bump(ms, u32::from(remote.counter()) + 1)
        } else {
            Hlc::new(ms, 0)
        };
        self.last = next;
        next
    }
}

/// `counter` overflow moves to the next millisecond with counter 0.
fn bump(ms: u64, counter: u32) -> Hlc {
    match u16::try_from(counter) {
        Ok(counter) => Hlc::new(ms, counter),
        Err(_) => Hlc::new(ms + 1, 0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_round_trip_and_order() {
        let hlc = Hlc::new(0x0199_f6a1_b2c3, 1);
        assert_eq!(hlc.to_text(), "0199f6a1b2c30001");
        assert_eq!(Hlc::parse("0199f6a1b2c30001"), Some(hlc));
        assert_eq!(Hlc::parse("0199F6A1B2C30001"), None);
        assert_eq!(Hlc::parse("0199f6a1b2c3001"), None);
        assert_eq!(Hlc::parse("0199f6a1b2c3000g"), None);
        assert!(Hlc::new(5, 0) > Hlc::new(4, 0xFFFF));
        assert!(Hlc::new(5, 0).to_text() > Hlc::new(4, 0xFFFF).to_text());
    }

    #[test]
    fn tick_advances_with_time_and_counts_within_a_millisecond() {
        let mut clock = HlcClock::start(None, 1000);
        assert_eq!(clock.tick(1000), Hlc::new(1000, 1));
        assert_eq!(clock.tick(1000), Hlc::new(1000, 2));
        assert_eq!(clock.tick(1001), Hlc::new(1001, 0));
        // A clock that went backwards keeps the last milliseconds.
        assert_eq!(clock.tick(900), Hlc::new(1001, 1));
    }

    #[test]
    fn tick_counter_overflow_moves_to_next_millisecond() {
        let mut clock = HlcClock::new(Hlc::new(1000, 0xFFFF));
        assert_eq!(clock.tick(1000), Hlc::new(1001, 0));
    }

    #[test]
    fn start_uses_greatest_of_records_and_now() {
        assert_eq!(
            HlcClock::start(Some(Hlc::new(5000, 3)), 1000).last(),
            Hlc::new(5000, 3)
        );
        assert_eq!(
            HlcClock::start(Some(Hlc::new(500, 3)), 1000).last(),
            Hlc::new(1000, 0)
        );
    }

    #[test]
    fn observe_merges_and_guards_drift() {
        let mut clock = HlcClock::new(Hlc::new(1000, 4));
        assert_eq!(clock.observe(Hlc::new(1000, 7), 900), Hlc::new(1000, 8));
        assert_eq!(clock.observe(Hlc::new(1200, 2), 900), Hlc::new(1200, 3));
        assert_eq!(clock.observe(Hlc::new(100, 2), 900), Hlc::new(1200, 4));
        assert_eq!(clock.observe(Hlc::new(100, 2), 5000), Hlc::new(5000, 0));
        // A value far in the future moves the clock at most one hour past now.
        let far = Hlc::new(5000 + 10 * MAX_DRIFT_MS, 0);
        assert_eq!(clock.observe(far, 5000), Hlc::new(5000 + MAX_DRIFT_MS, 1));
    }
}
