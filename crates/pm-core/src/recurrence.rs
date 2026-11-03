//! Recurring tasks (iteration 4, automation): the dates a repeating task falls on, as in
//! Microsoft Project's Recurring Task Information. The task itself becomes a summary with one
//! occurrence per date; this module only works out the dates.

use jiff::civil::Date;
use serde::{Deserialize, Serialize};

use crate::time::{date_from_day_number, day_number, weekday_index};
use crate::Calendar;

/// At most this many occurrences, as in Microsoft Project.
pub const MAX_OCCURRENCES: usize = 999;

/// How often the task repeats. Weekdays are Monday = 0 … Sunday = 6.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Pattern {
    /// Every `every` days.
    Daily { every: u32 },
    /// Every `every` weeks, on the days marked.
    Weekly { every: u32, weekdays: [bool; 7] },
    /// Day `day` of every `every` months; the month's last day when it is shorter.
    MonthlyOnDay { every: u32, day: u8 },
    /// The `week`th (1–4, or 5 for the last) `weekday` of every `every` months.
    MonthlyOnWeekday { every: u32, week: u8, weekday: u8 },
}

/// When the repeating stops.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "camelCase")]
pub enum End {
    /// After this many occurrences.
    After(u32),
    /// On or before this day (day number).
    By(i64),
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum RecurrenceError {
    #[error("repeat at least every 1")]
    ZeroInterval,
    #[error("choose at least one day of the week")]
    NoWeekdays,
    #[error("choose a day of the month from 1 to 31")]
    BadDay,
    #[error("choose first, second, third, fourth or last, and a day of the week")]
    BadWeek,
    #[error("repeat at least once")]
    ZeroCount,
    #[error("the end date is before the start date")]
    EndsBeforeStart,
    #[error("this would make more than {MAX_OCCURRENCES} occurrences; choose a later start or an earlier end")]
    TooMany,
    #[error("no date between the start and the end matches")]
    Empty,
}

fn ymd(day: i64) -> (i32, i32, i32) {
    let d = date_from_day_number(day);
    (i32::from(d.year()), i32::from(d.month()), i32::from(d.day()))
}

fn days_in_month(year: i32, month: i32) -> i32 {
    i32::from(Date::new(year as i16, month as i8, 1).expect("valid month").days_in_month())
}

fn day_of(year: i32, month: i32, day: i32) -> i64 {
    day_number(Date::new(year as i16, month as i8, day as i8).expect("valid date"))
}

/// The `week`th `weekday` of a month (5: the last one).
fn nth_weekday(year: i32, month: i32, week: u8, weekday: u8) -> i64 {
    if week == 5 {
        let last = day_of(year, month, days_in_month(year, month));
        last - ((weekday_index(last) as i64 - i64::from(weekday)).rem_euclid(7))
    } else {
        let first = day_of(year, month, 1);
        first + (i64::from(weekday) - weekday_index(first) as i64).rem_euclid(7) + 7 * (i64::from(week) - 1)
    }
}

/// The days (day numbers) the task falls on, from `start` on, in order.
pub fn dates(pattern: Pattern, start: i64, end: End) -> Result<Vec<i64>, RecurrenceError> {
    let every = match pattern {
        Pattern::Daily { every } | Pattern::Weekly { every, .. } | Pattern::MonthlyOnDay { every, .. } | Pattern::MonthlyOnWeekday { every, .. } => every,
    };
    if every == 0 {
        return Err(RecurrenceError::ZeroInterval);
    }
    match pattern {
        Pattern::Weekly { weekdays, .. } if !weekdays.contains(&true) => return Err(RecurrenceError::NoWeekdays),
        Pattern::MonthlyOnDay { day, .. } if !(1..=31).contains(&day) => return Err(RecurrenceError::BadDay),
        Pattern::MonthlyOnWeekday { week, weekday, .. } if !(1..=5).contains(&week) || weekday > 6 => return Err(RecurrenceError::BadWeek),
        _ => {}
    }
    let (limit, until) = match end {
        End::After(0) => return Err(RecurrenceError::ZeroCount),
        End::After(n) if n as usize > MAX_OCCURRENCES => return Err(RecurrenceError::TooMany),
        End::After(n) => (n as usize, i64::MAX),
        End::By(d) if d < start => return Err(RecurrenceError::EndsBeforeStart),
        End::By(d) => (MAX_OCCURRENCES + 1, d),
    };
    let every = i64::from(every);
    let mut out = Vec::new();
    // Each step is one period (day, week or month) from the start's; candidates before the
    // start are skipped. Periods are bounded, so a pattern that never matches still ends.
    let (y0, m0, _) = ymd(start);
    let week0 = start - weekday_index(start) as i64;
    let mut step = 0i64;
    while out.len() < limit {
        let candidates: Vec<i64> = match pattern {
            Pattern::Daily { .. } => vec![start + step * every],
            Pattern::Weekly { weekdays, .. } => {
                let monday = week0 + step * every * 7;
                (0..7).filter(|&i| weekdays[i]).map(|i| monday + i as i64).collect()
            }
            Pattern::MonthlyOnDay { day, .. } | Pattern::MonthlyOnWeekday { week: day, .. } => {
                let months = i64::from(m0 - 1) + step * every;
                let (y, m) = (y0 + months.div_euclid(12) as i32, (months.rem_euclid(12) + 1) as i32);
                match pattern {
                    Pattern::MonthlyOnWeekday { week, weekday, .. } => vec![nth_weekday(y, m, week, weekday)],
                    _ => vec![day_of(y, m, i32::from(day).min(days_in_month(y, m)))],
                }
            }
        };
        if candidates.first().is_some_and(|&d| d > until) {
            break;
        }
        for d in candidates.into_iter().filter(|&d| d >= start && d <= until) {
            if out.len() < limit {
                out.push(d);
            }
        }
        if out.len() > MAX_OCCURRENCES {
            return Err(RecurrenceError::TooMany);
        }
        step += 1;
    }
    if out.is_empty() {
        return Err(RecurrenceError::Empty);
    }
    Ok(out)
}

/// Occurrences on non-working days move to the next working day, as Microsoft Project offers;
/// one that would then share a day with another occurrence is dropped (the occurrence that
/// was already on that day keeps it).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OnWorkingDays {
    pub days: Vec<i64>,
    pub moved: usize,
    pub dropped: usize,
}

pub fn on_working_days(days: &[i64], calendar: &Calendar) -> OnWorkingDays {
    let working = |d: i64| calendar.pattern(d).is_working();
    let mut kept: std::collections::BTreeSet<i64> = days.iter().copied().filter(|&d| working(d)).collect();
    let (mut moved, mut dropped) = (0, 0);
    for &d in days.iter().filter(|&&d| !working(d)) {
        let next = calendar.next_working(crate::Moment::from_day_minute(d, 0)).day();
        if kept.insert(next) {
            moved += 1;
        } else {
            dropped += 1;
        }
    }
    OnWorkingDays { days: kept.into_iter().collect(), moved, dropped }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::civil::date;

    fn d(y: i16, m: i8, day: i8) -> i64 {
        day_number(date(y, m, day))
    }
    fn shown(days: &[i64]) -> Vec<String> {
        days.iter().map(|&x| date_from_day_number(x).to_string()).collect()
    }
    const MON_WED: [bool; 7] = [true, false, true, false, false, false, false];

    #[test]
    fn daily_every_two_days_by_count() {
        let got = dates(Pattern::Daily { every: 2 }, d(2026, 11, 2), End::After(3)).unwrap();
        assert_eq!(shown(&got), ["2026-11-02", "2026-11-04", "2026-11-06"]);
    }

    #[test]
    fn weekly_on_chosen_days_until_a_date() {
        // Starts on a Tuesday: that week's Monday is skipped, its Wednesday kept.
        let got = dates(Pattern::Weekly { every: 1, weekdays: MON_WED }, d(2026, 11, 3), End::By(d(2026, 11, 16))).unwrap();
        assert_eq!(shown(&got), ["2026-11-04", "2026-11-09", "2026-11-11", "2026-11-16"]);
    }

    #[test]
    fn every_other_week() {
        let got = dates(Pattern::Weekly { every: 2, weekdays: MON_WED }, d(2026, 11, 2), End::After(4)).unwrap();
        assert_eq!(shown(&got), ["2026-11-02", "2026-11-04", "2026-11-16", "2026-11-18"]);
    }

    #[test]
    fn monthly_day_31_uses_the_last_day_of_short_months() {
        let got = dates(Pattern::MonthlyOnDay { every: 1, day: 31 }, d(2027, 1, 1), End::After(3)).unwrap();
        assert_eq!(shown(&got), ["2027-01-31", "2027-02-28", "2027-03-31"]);
    }

    #[test]
    fn monthly_day_before_the_start_begins_next_month() {
        let got = dates(Pattern::MonthlyOnDay { every: 3, day: 1 }, d(2026, 11, 15), End::After(2)).unwrap();
        assert_eq!(shown(&got), ["2027-02-01", "2027-05-01"]);
    }

    #[test]
    fn first_monday_and_last_friday() {
        let first = dates(Pattern::MonthlyOnWeekday { every: 1, week: 1, weekday: 0 }, d(2026, 11, 1), End::After(2)).unwrap();
        assert_eq!(shown(&first), ["2026-11-02", "2026-12-07"]);
        let last = dates(Pattern::MonthlyOnWeekday { every: 1, week: 5, weekday: 4 }, d(2026, 11, 1), End::After(2)).unwrap();
        assert_eq!(shown(&last), ["2026-11-27", "2026-12-25"]);
    }

    #[test]
    fn crosses_the_year() {
        let got = dates(Pattern::MonthlyOnDay { every: 1, day: 15 }, d(2026, 11, 1), End::By(d(2027, 2, 1))).unwrap();
        assert_eq!(shown(&got), ["2026-11-15", "2026-12-15", "2027-01-15"]);
    }

    #[test]
    fn mistakes_are_explained() {
        let s = d(2026, 11, 2);
        assert_eq!(dates(Pattern::Daily { every: 0 }, s, End::After(2)), Err(RecurrenceError::ZeroInterval));
        assert_eq!(dates(Pattern::Weekly { every: 1, weekdays: [false; 7] }, s, End::After(2)), Err(RecurrenceError::NoWeekdays));
        assert_eq!(dates(Pattern::MonthlyOnDay { every: 1, day: 0 }, s, End::After(2)), Err(RecurrenceError::BadDay));
        assert_eq!(dates(Pattern::MonthlyOnWeekday { every: 1, week: 6, weekday: 0 }, s, End::After(2)), Err(RecurrenceError::BadWeek));
        assert_eq!(dates(Pattern::Daily { every: 1 }, s, End::After(0)), Err(RecurrenceError::ZeroCount));
        assert_eq!(dates(Pattern::Daily { every: 1 }, s, End::By(s - 1)), Err(RecurrenceError::EndsBeforeStart));
        assert_eq!(dates(Pattern::Daily { every: 1 }, s, End::By(s + 5000)), Err(RecurrenceError::TooMany));
        assert_eq!(dates(Pattern::Daily { every: 1 }, s, End::After(1000)), Err(RecurrenceError::TooMany));
        // The 1st of the month never falls in 2–20 November.
        assert_eq!(dates(Pattern::MonthlyOnDay { every: 1, day: 1 }, s, End::By(d(2026, 11, 20))), Err(RecurrenceError::Empty));
        assert_eq!(dates(Pattern::Daily { every: 1 }, s, End::After(999)).unwrap().len(), 999);
    }

    #[test]
    fn weekends_move_to_monday_and_clashes_are_dropped() {
        let cal = Calendar::standard();
        // Fri 6, Sat 7, Sun 8, Mon 9 November.
        let got = on_working_days(&[d(2026, 11, 6), d(2026, 11, 7), d(2026, 11, 8), d(2026, 11, 9)], &cal);
        assert_eq!(shown(&got.days), ["2026-11-06", "2026-11-09"]);
        assert_eq!((got.moved, got.dropped), (0, 2));
        let got = on_working_days(&[d(2026, 11, 7), d(2026, 11, 14)], &cal);
        assert_eq!(shown(&got.days), ["2026-11-09", "2026-11-16"]);
        assert_eq!((got.moved, got.dropped), (2, 0));
    }
}
