//! The calendar period the Top lists count plays in.

use crate::ui::when::{date_of, day_number, local, MONTHS, MONTH_NAMES};

type Date = (i32, u32, u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PeriodKind {
    Week,
    #[default]
    Month,
    Year,
    AllTime,
}

impl PeriodKind {
    pub const ALL: [PeriodKind; 4] = [Self::Week, Self::Month, Self::Year, Self::AllTime];

    pub fn name(self) -> &'static str {
        match self {
            Self::Week => "week",
            Self::Month => "month",
            Self::Year => "year",
            Self::AllTime => "all time",
        }
    }
}

/// A calendar period in local time, `back` periods before the current one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Period {
    pub kind: PeriodKind,
    pub back: u32,
}

impl Period {
    pub fn next_kind(self) -> Self {
        let kind = match self.kind {
            PeriodKind::Week => PeriodKind::Month,
            PeriodKind::Month => PeriodKind::Year,
            PeriodKind::Year => PeriodKind::AllTime,
            PeriodKind::AllTime => PeriodKind::Week,
        };
        Self { kind, back: 0 }
    }

    pub fn earlier(self) -> Self {
        match self.kind {
            PeriodKind::AllTime => self,
            _ => Self { back: self.back + 1, ..self },
        }
    }

    pub fn later(self) -> Option<Self> {
        match (self.kind, self.back) {
            (PeriodKind::AllTime, _) | (_, 0) => None,
            _ => Some(Self { back: self.back - 1, ..self }),
        }
    }

    /// Inclusive unix seconds from local midnight of the first day to the second before the next
    /// period; `None` for all time.
    pub fn span(self, now_unix: i64) -> Option<(i64, i64)> {
        let now = local(now_unix)?;
        let (start, end) = self.bounds((now.year, now.month, now.day))?;
        Some((midnight(start)?, midnight(end)? - 1))
    }

    pub fn label(self, now_unix: i64) -> String {
        match local(now_unix) {
            Some(now) => self.label_on((now.year, now.month, now.day)),
            None => String::new(),
        }
    }

    /// The first date and the date after the last one.
    fn bounds(self, today: Date) -> Option<(Date, Date)> {
        let back = i64::from(self.back);
        match self.kind {
            PeriodKind::AllTime => None,
            PeriodKind::Week => {
                let n = day_number(today.0, today.1, today.2);
                let since_monday = (n + 3).rem_euclid(7);
                let start = n - since_monday - 7 * back;
                Some((date_of(start), date_of(start + 7)))
            }
            PeriodKind::Month => {
                let index = i64::from(today.0) * 12 + i64::from(today.1) - 1 - back;
                Some((month_start(index), month_start(index + 1)))
            }
            PeriodKind::Year => {
                let year = today.0 - self.back as i32;
                Some(((year, 1, 1), (year + 1, 1, 1)))
            }
        }
    }

    fn label_on(self, today: Date) -> String {
        let Some((start, end)) = self.bounds(today) else { return "All time".into() };
        match self.kind {
            PeriodKind::Month => format!("{} {}", MONTH_NAMES[start.1 as usize - 1], start.0),
            PeriodKind::Year => start.0.to_string(),
            _ => {
                let last = date_of(day_number(end.0, end.1, end.2) - 1);
                let (from, to) = (MONTHS[start.1 as usize - 1], MONTHS[last.1 as usize - 1]);
                if start.0 != last.0 {
                    format!("{from} {}, {} – {to} {}, {}", start.2, start.0, last.2, last.0)
                } else if start.1 != last.1 {
                    format!("{from} {} – {to} {}, {}", start.2, last.2, last.0)
                } else {
                    format!("{from} {} – {}, {}", start.2, last.2, last.0)
                }
            }
        }
    }
}

fn month_start(index: i64) -> Date {
    (index.div_euclid(12) as i32, index.rem_euclid(12) as u32 + 1, 1)
}

fn midnight(date: Date) -> Option<i64> {
    // SAFETY: an all-zero `tm` is valid; `mktime` reads and normalizes it in place.
    let t = unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        tm.tm_year = date.0 - 1900;
        tm.tm_mon = date.1 as i32 - 1;
        tm.tm_mday = date.2 as i32;
        tm.tm_isdst = -1;
        libc::mktime(&mut tm)
    };
    (t != -1).then_some(t as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn of(kind: PeriodKind, back: u32) -> Period {
        Period { kind, back }
    }

    #[test]
    fn a_week_starts_on_the_monday_even_from_a_sunday() {
        let week = of(PeriodKind::Week, 0);
        assert_eq!(week.bounds((2026, 11, 22)), Some(((2026, 11, 16), (2026, 11, 23))));
        assert_eq!(week.bounds((2026, 11, 16)), Some(((2026, 11, 16), (2026, 11, 23))));
        assert_eq!(week.label_on((2026, 11, 22)), "Nov 16 – 22, 2026");
    }

    #[test]
    fn the_previous_week_is_seven_days_earlier() {
        let bounds = of(PeriodKind::Week, 1).bounds((2026, 11, 22));
        assert_eq!(bounds, Some(((2026, 11, 9), (2026, 11, 16))));
    }

    #[test]
    fn a_week_can_cross_a_month_or_a_year_end() {
        let week = of(PeriodKind::Week, 0);
        assert_eq!(week.bounds((2026, 11, 1)), Some(((2026, 10, 26), (2026, 11, 2))));
        assert_eq!(week.label_on((2026, 11, 1)), "Oct 26 – Nov 1, 2026");
        assert_eq!(week.bounds((2026, 1, 1)), Some(((2025, 12, 29), (2026, 1, 5))));
        assert_eq!(week.label_on((2026, 1, 1)), "Dec 29, 2025 – Jan 4, 2026");
    }

    #[test]
    fn a_month_back_from_january_is_the_december_before() {
        let december = of(PeriodKind::Month, 1);
        assert_eq!(december.bounds((2026, 1, 15)), Some(((2025, 12, 1), (2026, 1, 1))));
        assert_eq!(december.label_on((2026, 1, 15)), "December 2025");
        assert_eq!(of(PeriodKind::Month, 0).label_on((2026, 11, 22)), "November 2026");
    }

    #[test]
    fn a_year_spans_january_to_january() {
        let year = of(PeriodKind::Year, 1);
        assert_eq!(year.bounds((2026, 11, 22)), Some(((2025, 1, 1), (2026, 1, 1))));
        assert_eq!(year.label_on((2026, 11, 22)), "2025");
    }

    #[test]
    fn all_time_has_no_bounds() {
        let all = of(PeriodKind::AllTime, 0);
        assert_eq!(all.bounds((2026, 11, 22)), None);
        assert_eq!(all.span(1_795_000_000), None);
        assert_eq!(all.label_on((2026, 11, 22)), "All time");
    }

    #[test]
    fn later_stops_at_the_current_period_and_all_time_does_not_move() {
        assert_eq!(of(PeriodKind::Month, 2).later(), Some(of(PeriodKind::Month, 1)));
        assert_eq!(of(PeriodKind::Month, 0).later(), None);
        assert_eq!(of(PeriodKind::Year, 0).earlier(), of(PeriodKind::Year, 1));
        let all = of(PeriodKind::AllTime, 0);
        assert_eq!((all.later(), all.earlier()), (None, all));
    }

    #[test]
    fn the_kind_cycles_and_lands_on_the_current_period() {
        let mut p = of(PeriodKind::Week, 3);
        let mut seen = Vec::new();
        for _ in 0..4 {
            p = p.next_kind();
            seen.push(p);
        }
        let kinds = [PeriodKind::Month, PeriodKind::Year, PeriodKind::AllTime, PeriodKind::Week];
        assert_eq!(seen, kinds.map(|kind| of(kind, 0)));
    }

    #[test]
    fn consecutive_periods_leave_no_gap_and_no_overlap() {
        let now = 1_795_000_000;
        for kind in [PeriodKind::Week, PeriodKind::Month, PeriodKind::Year] {
            let (_, before_end) = of(kind, 1).span(now).unwrap();
            let (start, end) = of(kind, 0).span(now).unwrap();
            assert_eq!(before_end + 1, start, "{kind:?}");
            assert!(end > start && start <= now && now <= end, "{kind:?}");
        }
    }
}
