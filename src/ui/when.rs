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

const MONTHS: [&str; 12] =
    ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

fn day_number(l: Local) -> i64 {
    let y = l.year as i64 - (l.month <= 2) as i64;
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let mp = (l.month as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + l.day as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

pub fn played_text(played: Local, now: Local) -> String {
    let ago = day_number(now) - day_number(played);
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

pub fn played_at(unix: i64) -> String {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
    match (local(unix), local(now)) {
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
