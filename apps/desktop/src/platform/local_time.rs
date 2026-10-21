//! The user's time zone and date format on Windows (D68, D71): a UTC time
//! converted to the zone of the system, with the daylight saving rules of
//! its date (`SystemTimeToTzSpecificLocalTimeEx` over the dynamic zone
//! information), and dates and times written in the format of the user's
//! locale (`GetDateFormatEx`, `GetTimeFormatEx`). [`crate::dates`] uses it;
//! other platforms show UTC there.
//!
//! A platform module (CONTRIBUTING.md, "Unsafe code"): the calls below are
//! its only `unsafe` code.
#![cfg_attr(windows, allow(unsafe_code))]

#[cfg(windows)]
pub use system::{date_text, time_text, to_local};

#[cfg(windows)]
mod system {
    use std::time::{SystemTime, UNIX_EPOCH};

    use windows_sys::Win32::Foundation::{FILETIME, SYSTEMTIME};
    use windows_sys::Win32::Globalization::{
        DATE_SHORTDATE, GetDateFormatEx, GetTimeFormatEx, TIME_NOSECONDS,
    };
    use windows_sys::Win32::System::Time::{
        DYNAMIC_TIME_ZONE_INFORMATION, FileTimeToSystemTime, GetDynamicTimeZoneInformation,
        SystemTimeToTzSpecificLocalTime, SystemTimeToTzSpecificLocalTimeEx, TIME_ZONE_ID_INVALID,
        TIME_ZONE_INFORMATION,
    };

    use crate::dates::Civil;

    /// 100 ns intervals from 1601-01-01 (FILETIME) to 1970-01-01.
    const EPOCH_DIFFERENCE: u64 = 116_444_736_000_000_000;

    /// `time` as a UTC `SYSTEMTIME` (`None` before 1970).
    fn utc_system_time(time: SystemTime) -> Option<SYSTEMTIME> {
        let ticks = time.duration_since(UNIX_EPOCH).ok()?.as_nanos() / 100;
        let ticks = u64::try_from(ticks).ok()?.checked_add(EPOCH_DIFFERENCE)?;
        let file_time = FILETIME {
            dwLowDateTime: (ticks & 0xffff_ffff) as u32,
            dwHighDateTime: (ticks >> 32) as u32,
        };
        let mut out = SYSTEMTIME::default();
        // SAFETY: both pointers point to locals that outlive the call; the
        // function reads `file_time` and writes `out`, nothing else.
        let converted = unsafe { FileTimeToSystemTime(&file_time, &mut out) };
        (converted != 0).then_some(out)
    }

    fn civil(time: &SYSTEMTIME) -> Civil {
        Civil {
            year: i32::from(time.wYear),
            month: time.wMonth as u8,
            day: time.wDay as u8,
            hour: time.wHour as u8,
            minute: time.wMinute as u8,
            second: time.wSecond as u8,
        }
    }

    fn system_time(civil: &Civil) -> SYSTEMTIME {
        SYSTEMTIME {
            wYear: u16::try_from(civil.year).unwrap_or(1970),
            wMonth: u16::from(civil.month),
            wDayOfWeek: 0,
            wDay: u16::from(civil.day),
            wHour: u16::from(civil.hour),
            wMinute: u16::from(civil.minute),
            wSecond: u16::from(civil.second),
            wMilliseconds: 0,
        }
    }

    /// `time` in the user's time zone, with the daylight saving rules of
    /// its date; `None` when the system cannot say.
    pub fn to_local(time: SystemTime) -> Option<Civil> {
        let utc = utc_system_time(time)?;
        let mut zone = DYNAMIC_TIME_ZONE_INFORMATION::default();
        // SAFETY: writes the zone information into `zone`, a local that
        // outlives the call.
        if unsafe { GetDynamicTimeZoneInformation(&mut zone) } == TIME_ZONE_ID_INVALID {
            return None;
        }
        let mut local = SYSTEMTIME::default();
        // SAFETY: reads `zone` and `utc` and writes `local`, all locals that
        // outlive the call.
        let converted = unsafe { SystemTimeToTzSpecificLocalTimeEx(&zone, &utc, &mut local) };
        (converted != 0).then(|| civil(&local))
    }

    /// `time` in `zone` (fixed daylight saving rules; the tests' zone).
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn to_local_in(zone: &TIME_ZONE_INFORMATION, time: SystemTime) -> Option<Civil> {
        let utc = utc_system_time(time)?;
        let mut local = SYSTEMTIME::default();
        // SAFETY: reads `zone` (borrowed for the call) and `utc`, writes
        // `local`; the locals outlive the call.
        let converted = unsafe { SystemTimeToTzSpecificLocalTime(zone, &utc, &mut local) };
        (converted != 0).then(|| civil(&local))
    }

    /// A locale name as a NUL-terminated UTF-16 string (`None`: the user's
    /// locale, a null pointer).
    fn locale_name(locale: Option<&str>) -> Option<Vec<u16>> {
        locale.map(|name| name.encode_utf16().chain(std::iter::once(0)).collect())
    }

    /// The text a format call wrote: its count includes the NUL.
    fn written(buffer: &[u16], count: i32) -> Option<String> {
        let length = usize::try_from(count).ok()?.checked_sub(1)?;
        Some(String::from_utf16_lossy(buffer.get(..length)?))
    }

    /// The short date of `civil` in `locale` (`None`: the user's).
    pub fn date_text(civil: &Civil, locale: Option<&str>) -> Option<String> {
        let name = locale_name(locale);
        let name_ptr = name.as_ref().map_or(std::ptr::null(), |name| name.as_ptr());
        let date = system_time(civil);
        let mut buffer = [0u16; 96];
        // SAFETY: `name_ptr` is null (the user's locale) or the NUL-terminated
        // `name`, alive until the end of this function; `date` is read;
        // `buffer` is written up to its length, passed in UTF-16 units. No
        // format or calendar string is passed.
        let count = unsafe {
            GetDateFormatEx(
                name_ptr,
                DATE_SHORTDATE,
                &date,
                std::ptr::null(),
                buffer.as_mut_ptr(),
                buffer.len() as i32,
                std::ptr::null(),
            )
        };
        written(&buffer, count)
    }

    /// The time of day of `civil` without seconds in `locale` (`None`: the
    /// user's).
    pub fn time_text(civil: &Civil, locale: Option<&str>) -> Option<String> {
        let name = locale_name(locale);
        let name_ptr = name.as_ref().map_or(std::ptr::null(), |name| name.as_ptr());
        let time = system_time(civil);
        let mut buffer = [0u16; 64];
        // SAFETY: as in `date_text`: `name_ptr` is null or the NUL-terminated
        // `name`, alive until the end of this function; `time` is read;
        // `buffer` is written up to its length in UTF-16 units.
        let count = unsafe {
            GetTimeFormatEx(
                name_ptr,
                TIME_NOSECONDS,
                &time,
                std::ptr::null(),
                buffer.as_mut_ptr(),
                buffer.len() as i32,
            )
        };
        written(&buffer, count)
    }

    #[cfg(test)]
    mod tests {
        use std::time::Duration;

        use super::*;

        fn at(timestamp: &str) -> SystemTime {
            crate::dates::parse(timestamp).unwrap()
        }

        /// Central European time: UTC+1, UTC+2 from the last Sunday of March
        /// (02:00) to the last Sunday of October (03:00).
        fn central_europe() -> TIME_ZONE_INFORMATION {
            let rule = |month: u16, hour: u16| SYSTEMTIME {
                wYear: 0,
                wMonth: month,
                wDayOfWeek: 0,
                wDay: 5,
                wHour: hour,
                wMinute: 0,
                wSecond: 0,
                wMilliseconds: 0,
            };
            TIME_ZONE_INFORMATION {
                Bias: -60,
                StandardDate: rule(10, 3),
                StandardBias: 0,
                DaylightDate: rule(3, 2),
                DaylightBias: -60,
                ..TIME_ZONE_INFORMATION::default()
            }
        }

        /// The conversion calls: daylight saving time is applied by the date
        /// (summer +2, winter +1, around the October change), and the
        /// user's own zone gives an answer within a day of UTC.
        #[test]
        fn a_time_takes_the_daylight_saving_rule_of_its_date() {
            let zone = central_europe();
            let summer = to_local_in(&zone, at("2026-07-01T12:00:00.000Z")).unwrap();
            assert_eq!((summer.hour, summer.minute), (14, 0));
            let winter = to_local_in(&zone, at("2026-01-15T23:30:00.000Z")).unwrap();
            assert_eq!((winter.day, winter.hour, winter.minute), (16, 0, 30));
            // 2026-10-25 is the last Sunday of October: 01:00 UTC is 03:00
            // CEST becoming 02:00 CET.
            let before = to_local_in(&zone, at("2026-10-25T00:30:00.000Z")).unwrap();
            assert_eq!(before.hour, 2);
            let after = to_local_in(&zone, at("2026-10-25T01:30:00.000Z")).unwrap();
            assert_eq!(after.hour, 2);
            let system = to_local(at("2026-10-21T12:00:00.000Z")).unwrap();
            assert!((20..=22).contains(&system.day), "{system:?}");
            assert!(to_local(SystemTime::UNIX_EPOCH - Duration::from_secs(1)).is_none());
        }

        /// The format calls: a named locale's short date and time, and the
        /// user's locale writes something.
        #[test]
        fn dates_and_times_take_the_locale_format() {
            let civil = Civil {
                year: 2026,
                month: 10,
                day: 21,
                hour: 18,
                minute: 4,
                second: 9,
            };
            assert_eq!(date_text(&civil, Some("de-DE")).unwrap(), "21.10.2026");
            assert_eq!(date_text(&civil, Some("en-US")).unwrap(), "10/21/2026");
            assert_eq!(time_text(&civil, Some("de-DE")).unwrap(), "18:04");
            let us = time_text(&civil, Some("en-US")).unwrap();
            assert!(us.starts_with("6:04") && us.ends_with("PM"), "{us}");
            assert!(!date_text(&civil, None).unwrap().is_empty());
            assert!(!time_text(&civil, None).unwrap().is_empty());
            assert!(date_text(&civil, Some("no-such-locale-at-all")).is_none());
        }
    }
}
