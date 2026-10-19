//! Timestamps and record ids taken from the injected clock
//! (docs/data-model.md, sections 1.1 and 3.2).
//!
//! Timestamps are RFC 3339 UTC with milliseconds and a `Z` suffix, for example
//! `2026-10-18T22:31:05.123Z`. Record ids are UUIDv7 built from the injected
//! clock so that tests with a manual clock get predictable creation times.

use std::fmt;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use time::OffsetDateTime;
use time::format_description::BorrowedFormatItem;
use time::macros::format_description;
use uuid::{ContextV7, NoContext, Timestamp, Uuid};

const FORMAT: &[BorrowedFormatItem<'static>] =
    format_description!("[year]-[month]-[day]T[hour]:[minute]:[second].[subsecond digits:3]Z");

const EPOCH_TEXT: &str = "1970-01-01T00:00:00.000Z";

/// Milliseconds since the Unix epoch (0 for times before it).
pub fn unix_ms(time: SystemTime) -> u64 {
    match time.duration_since(UNIX_EPOCH) {
        Ok(elapsed) => u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX),
        Err(_) => 0,
    }
}

/// Formats milliseconds since the Unix epoch as an RFC 3339 UTC timestamp.
pub fn format_unix_ms(ms: u64) -> String {
    let nanos = i128::from(ms) * 1_000_000;
    match OffsetDateTime::from_unix_timestamp_nanos(nanos) {
        Ok(moment) => moment
            .format(FORMAT)
            .unwrap_or_else(|_| EPOCH_TEXT.to_owned()),
        Err(_) => EPOCH_TEXT.to_owned(),
    }
}

/// RFC 3339 UTC with milliseconds, for example `2026-10-18T22:31:05.123Z`.
pub fn format_timestamp(time: SystemTime) -> String {
    format_unix_ms(unix_ms(time))
}

/// A UUIDv7 whose timestamp is `now` (random sub-millisecond bits).
pub fn new_record_id(now: SystemTime) -> Uuid {
    let (secs, nanos) = split(now);
    Uuid::new_v7(Timestamp::from_unix(NoContext, secs, nanos))
}

/// Milliseconds of a UUIDv7 timestamp; 0 for other UUID versions.
pub fn uuid_v7_ms(id: &Uuid) -> u64 {
    match id.get_timestamp() {
        Some(ts) => {
            let (secs, nanos) = ts.to_unix();
            secs.saturating_mul(1000)
                .saturating_add(u64::from(nanos) / 1_000_000)
        }
        None => 0,
    }
}

/// The creation time of a record: the timestamp of its UUIDv7 `record_id`.
pub fn uuid_v7_timestamp(id: &Uuid) -> String {
    format_unix_ms(uuid_v7_ms(id))
}

fn split(time: SystemTime) -> (u64, u32) {
    let elapsed = time.duration_since(UNIX_EPOCH).unwrap_or(Duration::ZERO);
    (elapsed.as_secs(), elapsed.subsec_nanos())
}

/// Generates UUIDv7 values that are strictly increasing within the process
/// even when several ids are created in the same millisecond.
pub struct IdGenerator {
    context: ContextV7,
}

impl IdGenerator {
    pub fn new() -> Self {
        Self {
            context: ContextV7::new(),
        }
    }

    /// A new UUIDv7 with the timestamp `now`.
    pub fn next(&self, now: SystemTime) -> Uuid {
        let (secs, nanos) = split(now);
        Uuid::new_v7(Timestamp::from_unix(&self.context, secs, nanos))
    }
}

impl Default for IdGenerator {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for IdGenerator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("IdGenerator")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_rfc3339_with_milliseconds() {
        let time = UNIX_EPOCH + Duration::from_millis(1_792_362_665_123);
        assert_eq!(format_timestamp(time), "2026-10-18T22:31:05.123Z");
        assert_eq!(format_unix_ms(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(
            format_timestamp(UNIX_EPOCH - Duration::from_secs(5)),
            EPOCH_TEXT
        );
    }

    #[test]
    fn record_ids_carry_the_clock_time() {
        let time = UNIX_EPOCH + Duration::from_millis(1_792_362_665_123);
        let id = new_record_id(time);
        assert_eq!(id.get_version_num(), 7);
        assert_eq!(uuid_v7_ms(&id), 1_792_362_665_123);
        assert_eq!(uuid_v7_timestamp(&id), "2026-10-18T22:31:05.123Z");
    }

    #[test]
    fn generator_is_monotonic_within_a_millisecond() {
        let generator = IdGenerator::new();
        let time = UNIX_EPOCH + Duration::from_millis(1_792_362_665_123);
        let mut previous = generator.next(time);
        for _ in 0..100 {
            let next = generator.next(time);
            assert!(next > previous);
            previous = next;
        }
    }

    #[test]
    fn non_v7_ids_have_epoch_time() {
        assert_eq!(uuid_v7_ms(&Uuid::nil()), 0);
    }
}
