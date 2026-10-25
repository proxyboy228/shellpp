//! Dates shown to the user (D68, D71): in local time, in the format of the
//! system's locale, with the full time in UTC for tooltips. Records keep
//! their RFC 3339 UTC timestamps (docs/data-model.md, section 1.1); only the
//! display changes.
//!
//! On Windows the zone (with the daylight saving rules of each date) and
//! the format come from the system ([`crate::platform::local_time`]); on
//! Linux and macOS the zone comes from the C library and dates are written
//! as ISO dates ("2026-10-21 18:04"; D210). Where the zone cannot be read,
//! or is UTC, the time stays in UTC and says so ("18:04 UTC"). Tests fix the
//! zone and the format per thread ([`set_fixed_display`]).

use std::cell::Cell;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// A date and time of the calendar (proleptic Gregorian).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Civil {
    pub year: i32,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
}

impl Civil {
    /// The UTC calendar time of `time` (times before 1970 count as 1970).
    pub fn utc(time: SystemTime) -> Self {
        let seconds = time
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_secs());
        let seconds = i64::try_from(seconds).unwrap_or(i64::MAX / 2);
        let (year, month, day) = civil_from_days(seconds.div_euclid(86_400));
        let of_day = seconds.rem_euclid(86_400);
        Self {
            year,
            month,
            day,
            hour: (of_day / 3_600) as u8,
            minute: (of_day % 3_600 / 60) as u8,
            second: (of_day % 60) as u8,
        }
    }

    /// Days since 1970-01-01 of the date.
    pub fn days(&self) -> i64 {
        days_from_civil(self.year, self.month, self.day)
    }
}

/// Days since 1970-01-01 of a calendar date (Howard Hinnant's algorithm).
fn days_from_civil(year: i32, month: u8, day: u8) -> i64 {
    let (year, month, day) = (i64::from(year), i64::from(month), i64::from(day));
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let month_index = (month + 9) % 12;
    let day_of_year = (153 * month_index + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// The calendar date of a day number since 1970-01-01.
fn civil_from_days(days: i64) -> (i32, u8, u8) {
    let days = days + 719_468;
    let era = days.div_euclid(146_097);
    let day_of_era = days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * month_index + 2) / 5 + 1) as u8;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    } as u8;
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (i32::try_from(year).unwrap_or(i32::MAX), month, day)
}

/// Reads a record timestamp (`2026-10-20T12:34:56.789Z`, data-model 1.1;
/// the fraction is optional). Anything else is `None`.
pub fn parse(timestamp: &str) -> Option<SystemTime> {
    let bytes = timestamp.as_bytes();
    let shaped = bytes.len() >= 20
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes[10] == b'T'
        && bytes[13] == b':'
        && bytes[16] == b':'
        && timestamp.ends_with('Z');
    if !shaped {
        return None;
    }
    let number = |from: usize, to: usize| -> Option<u32> {
        let text = timestamp.get(from..to)?;
        text.bytes()
            .all(|byte| byte.is_ascii_digit())
            .then(|| text.parse().ok())
            .flatten()
    };
    let (year, month, day) = (number(0, 4)?, number(5, 7)?, number(8, 10)?);
    let (hour, minute, second) = (number(11, 13)?, number(14, 16)?, number(17, 19)?);
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) || hour > 23 || minute > 59 {
        return None;
    }
    if second > 60 {
        return None;
    }
    let millis = match bytes[19] {
        b'Z' if bytes.len() == 20 => 0,
        b'.' => {
            let fraction = timestamp.get(20..timestamp.len() - 1)?;
            if fraction.is_empty() || !fraction.bytes().all(|byte| byte.is_ascii_digit()) {
                return None;
            }
            let digits: String = fraction.chars().chain("00".chars()).take(3).collect();
            digits.parse::<u64>().ok()?
        }
        _ => return None,
    };
    let days = days_from_civil(i32::try_from(year).ok()?, month as u8, day as u8);
    let seconds =
        days * 86_400 + i64::from(hour) * 3_600 + i64::from(minute) * 60 + i64::from(second);
    let seconds = u64::try_from(seconds).ok()?;
    Some(UNIX_EPOCH + Duration::from_secs(seconds) + Duration::from_millis(millis))
}

/// A fixed zone and format for this thread (tests): dates `21.10.2026`,
/// times `18:04`, `offset_minutes` east of UTC.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FixedDisplay {
    pub offset_minutes: i32,
}

thread_local! {
    static FIXED: Cell<Option<FixedDisplay>> = const { Cell::new(None) };
}

/// Shows dates on this thread in a fixed zone and format (`None`: the
/// system's again). For tests, whose expectations cannot depend on the
/// machine's zone and locale.
pub fn set_fixed_display(display: Option<FixedDisplay>) {
    FIXED.with(|fixed| fixed.set(display));
}

fn fixed() -> Option<FixedDisplay> {
    FIXED.with(Cell::get)
}

/// The calendar time of `time` in the display's zone.
pub fn local(time: SystemTime) -> Civil {
    local_and_known(time).0
}

/// The calendar time of `time` in the display's zone, and whether that zone
/// is the local one (else UTC, said so).
fn local_and_known(time: SystemTime) -> (Civil, bool) {
    if let Some(fixed) = fixed() {
        let offset = Duration::from_secs(u64::from(fixed.offset_minutes.unsigned_abs()) * 60);
        let shifted = if fixed.offset_minutes >= 0 {
            time.checked_add(offset)
        } else {
            time.checked_sub(offset)
        };
        return (Civil::utc(shifted.unwrap_or(time)), true);
    }
    #[cfg(any(windows, target_os = "linux", target_os = "macos"))]
    if let Some(civil) = crate::platform::local_time::to_local(time) {
        return (civil, true);
    }
    (Civil::utc(time), cfg!(windows))
}

/// The short date in the user's locale ("21.10.2026", "10/21/2026").
pub fn date(time: SystemTime) -> String {
    let civil = local(time);
    if fixed().is_some() {
        return format!("{:02}.{:02}.{:04}", civil.day, civil.month, civil.year);
    }
    #[cfg(windows)]
    if let Some(text) = crate::platform::local_time::date_text(&civil, None) {
        return text;
    }
    format!("{:04}-{:02}-{:02}", civil.year, civil.month, civil.day)
}

/// The time of day without seconds in the user's locale ("18:04",
/// "6:04 PM"); "18:04 UTC" where the local zone is not known.
pub fn time(time: SystemTime) -> String {
    let (civil, known) = local_and_known(time);
    if fixed().is_none() {
        #[cfg(windows)]
        if let Some(text) = crate::platform::local_time::time_text(&civil, None) {
            return text;
        }
    }
    let text = format!("{:02}:{:02}", civil.hour, civil.minute);
    if known { text } else { format!("{text} UTC") }
}

/// The date and the time ("21.10.2026 18:04").
pub fn date_time(moment: SystemTime) -> String {
    format!("{} {}", date(moment), time(moment))
}

/// The full time in UTC, for tooltips ("2026-10-21 15:04:09 UTC").
pub fn utc_text(time: SystemTime) -> String {
    let civil = Civil::utc(time);
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02} UTC",
        civil.year, civil.month, civil.day, civil.hour, civil.minute, civil.second
    )
}

/// Calendar days from `start` to `end` in the display's zone (`(+1d)` of
/// the Logs screen).
pub fn days_apart(start: SystemTime, end: SystemTime) -> i64 {
    local(end).days() - local(start).days()
}

/// A record timestamp as the date and the time; anything else as it is.
pub fn timestamp_text(timestamp: &str) -> String {
    parse(timestamp).map_or_else(|| timestamp.to_owned(), date_time)
}

/// The tooltip of a record timestamp: its full time in UTC.
pub fn timestamp_tooltip(timestamp: &str) -> Option<String> {
    parse(timestamp).map(utc_text)
}

/// The tooltip of a date on screen: its full time in UTC (D68).
pub fn utc_tooltip(
    utc: String,
) -> impl Fn(&mut gpui::Window, &mut gpui::App) -> gpui::AnyView + 'static {
    let utc: gpui::SharedString = utc.into();
    move |window, cx| gpui_kit::component::tooltip::Tooltip::new(utc.clone()).build(window, cx)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(timestamp: &str) -> SystemTime {
        parse(timestamp).unwrap()
    }

    #[test]
    fn timestamps_are_read_and_written_back() {
        let moment = at("2026-10-21T15:04:09.250Z");
        assert_eq!(
            moment.duration_since(UNIX_EPOCH).unwrap().as_millis(),
            1_792_595_049_250
        );
        assert_eq!(utc_text(moment), "2026-10-21 15:04:09 UTC");
        assert_eq!(at("1970-01-01T00:00:00Z"), UNIX_EPOCH);
        assert_eq!(
            utc_text(at("2024-02-29T23:59:59.9Z")),
            "2024-02-29 23:59:59 UTC"
        );
        for bad in [
            "",
            "garbage",
            "2026-10-21 15:04:09Z",
            "2026-13-21T15:04:09Z",
            "2026-10-21T25:04:09Z",
            "2026-10-21T15:04:09.Z",
            "2026-10-21T15:04:09+03:00",
            "+026-10-21T15:04:09Z",
        ] {
            assert_eq!(parse(bad), None, "{bad}");
        }
        // Days and dates round trip across the calendar.
        for days in [-1, 0, 59, 365, 11_016, 20_747, 2_932_896] {
            let (year, month, day) = civil_from_days(days);
            assert_eq!(days_from_civil(year, month, day), days);
        }
    }

    /// A fixed zone east of UTC: the local date and time, the day boundary
    /// in local dates, and no "UTC" except in the tooltip.
    #[test]
    fn a_fixed_display_shows_local_dates() {
        set_fixed_display(Some(FixedDisplay {
            offset_minutes: 180,
        }));
        let moment = at("2026-10-21T22:30:00.000Z");
        assert_eq!(date(moment), "22.10.2026");
        assert_eq!(time(moment), "01:30");
        assert_eq!(date_time(moment), "22.10.2026 01:30");
        assert!(!date_time(moment).contains("UTC"));
        assert_eq!(utc_text(moment), "2026-10-21 22:30:00 UTC");
        // 20:50 and 21:20 UTC are on two local days (23:50 and 00:20).
        assert_eq!(
            days_apart(at("2026-10-21T20:50:00Z"), at("2026-10-21T21:20:00Z")),
            1
        );
        assert_eq!(
            days_apart(at("2026-10-21T15:00:00Z"), at("2026-10-21T20:00:00Z")),
            0
        );
        assert_eq!(timestamp_text("garbage"), "garbage");
        assert_eq!(
            timestamp_text("2026-10-21T22:30:00.000Z"),
            "22.10.2026 01:30"
        );
        assert_eq!(
            timestamp_tooltip("2026-10-21T22:30:00.000Z").as_deref(),
            Some("2026-10-21 22:30:00 UTC")
        );
        set_fixed_display(Some(FixedDisplay {
            offset_minutes: -300,
        }));
        assert_eq!(date_time(at("2026-10-21T02:00:00Z")), "20.10.2026 21:00");
        set_fixed_display(None);
    }

    /// The system's display writes something for a date (Windows: its
    /// zone and locale; Linux and macOS: the C library's zone, or UTC said
    /// so, as `platform::local_time` tests with set zones; elsewhere UTC,
    /// said so).
    #[test]
    fn the_system_display_writes_dates() {
        set_fixed_display(None);
        let moment = at("2026-10-21T12:00:00.000Z");
        assert!(!date(moment).is_empty());
        let says_utc = time(moment).contains("UTC");
        if cfg!(windows) {
            assert!(!says_utc);
        } else if !cfg!(any(target_os = "linux", target_os = "macos")) {
            assert!(says_utc);
        }
    }
}
