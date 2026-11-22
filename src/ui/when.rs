//! A play time as the Played column shows it.

use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Local {
    pub year: i32,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
}

pub(crate) const MONTHS: [&str; 12] =
    ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

pub(crate) const MONTH_NAMES: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

/// Days since 1970-01-01 of a proleptic Gregorian date.
pub(crate) fn day_number(year: i32, month: u32, day: u32) -> i64 {
    let y = year as i64 - (month <= 2) as i64;
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let mp = (month as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The inverse of [`day_number`]: (year, month, day).
pub(crate) fn date_of(days: i64) -> (i32, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = (yoe + era * 400 + (month <= 2) as i64) as i32;
    (year, month, day)
}

pub fn played_text(played: Local, now: Local) -> String {
    let ago = day_number(now.year, now.month, now.day)
        - day_number(played.year, played.month, played.day);
    if ago == 0 {
        format!("{:02}:{:02}", played.hour, played.minute)
    } else if ago == 1 {
        "yesterday".into()
    } else if played.year == now.year {
        format!("{} {}", MONTHS[played.month as usize - 1], played.day)
    } else {
        format!("{}-{:02}-{:02}", played.year, played.month, played.day)
    }
}

pub fn local(unix: i64) -> Option<Local> {
    let t = libc::time_t::try_from(unix).ok()?;
    // SAFETY: both pointers are valid for the call; localtime_r is reentrant.
    let tm = unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        if libc::localtime_r(&t, &mut tm).is_null() {
            return None;
        }
        tm
    };
    Some(Local {
        year: tm.tm_year + 1900,
        month: tm.tm_mon as u32 + 1,
        day: tm.tm_mday as u32,
        hour: tm.tm_hour as u32,
        minute: tm.tm_min as u32,
    })
}

pub fn now_unix() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

pub fn played_at(unix: i64) -> String {
    match (local(unix), local(now_unix())) {
        (Some(played), Some(now)) => played_text(played, now),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(year: i32, month: u32, day: u32, hour: u32, minute: u32) -> Local {
        Local { year, month, day, hour, minute }
    }

    #[test]
    fn today_shows_the_clock_time() {
        let now = at(2026, 11, 22, 18, 0);
        assert_eq!(played_text(at(2026, 11, 22, 10, 25), now), "10:25");
        assert_eq!(played_text(at(2026, 11, 22, 0, 5), now), "00:05");
    }

    #[test]
    fn midnight_separates_today_from_yesterday() {
        let now = at(2026, 11, 22, 0, 0);
        assert_eq!(played_text(at(2026, 11, 21, 23, 59), now), "yesterday");
        assert_eq!(played_text(at(2026, 11, 22, 0, 0), now), "00:00");
        assert_eq!(played_text(at(2026, 11, 20, 23, 59), now), "Nov 20");
    }

    #[test]
    fn yesterday_crosses_month_and_year_ends() {
        assert_eq!(played_text(at(2026, 10, 31, 12, 0), at(2026, 11, 1, 9, 0)), "yesterday");
        assert_eq!(played_text(at(2025, 12, 31, 12, 0), at(2026, 1, 1, 9, 0)), "yesterday");
        assert_eq!(played_text(at(2024, 2, 29, 12, 0), at(2024, 3, 1, 9, 0)), "yesterday");
    }

    #[test]
    fn earlier_this_year_names_the_month_and_other_years_give_the_date() {
        let now = at(2026, 11, 22, 18, 0);
        assert_eq!(played_text(at(2026, 1, 5, 8, 0), now), "Jan 5");
        assert_eq!(played_text(at(2025, 11, 20, 8, 0), now), "2025-11-20");
        assert_eq!(played_text(at(2025, 12, 31, 23, 59), at(2026, 1, 2, 0, 1)), "2025-12-31");
    }
}
