//! Working-time calendar and calendar arithmetic (spec 4.5 step 8, rules R4 to R9).
//!
//! Start-type results snap forward to the next working instant; finish-type results snap
//! back to the end of the previous working period. All arithmetic is on whole minutes.

use std::collections::BTreeMap;

use jiff::civil::Date;
use serde::{Deserialize, Serialize};

use crate::time::{day_number, weekday_index, Moment, MINUTES_PER_DAY};

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum CalendarError {
    #[error("a working time range must start before it ends and lie within one day ({start}..{end})")]
    InvalidRange { start: u16, end: u16 },
    #[error("working time ranges on the same day overlap")]
    OverlappingRanges,
    #[error("the working week has no working time, so no task could ever be scheduled")]
    NoWorkingTime,
}

/// Working time within one day, in minutes from midnight: `start..end`, `end <= 1440`.
///
/// Deserializing validates like the constructors do, so a damaged file cannot produce
/// an invalid calendar (which could make the arithmetic loop forever).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "RawRange")]
pub struct WorkRange {
    start: u16,
    end: u16,
}

#[derive(Deserialize)]
struct RawRange {
    start: u16,
    end: u16,
}

impl TryFrom<RawRange> for WorkRange {
    type Error = CalendarError;
    fn try_from(r: RawRange) -> Result<Self, CalendarError> {
        WorkRange::new(r.start, r.end)
    }
}

impl WorkRange {
    pub fn new(start: u16, end: u16) -> Result<Self, CalendarError> {
        if start < end && end as i64 <= MINUTES_PER_DAY {
            Ok(WorkRange { start, end })
        } else {
            Err(CalendarError::InvalidRange { start, end })
        }
    }

    /// Convenience for whole hours, e.g. `hours(8, 12)`.
    pub fn hours(start: u16, end: u16) -> Result<Self, CalendarError> {
        WorkRange::new(start * 60, end * 60)
    }

    pub fn start(self) -> u16 {
        self.start
    }

    pub fn end(self) -> u16 {
        self.end
    }
}

/// The working time of one day: sorted, non-overlapping ranges. Empty means non-working.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "Vec<WorkRange>")]
pub struct DayPattern(Vec<WorkRange>);

impl TryFrom<Vec<WorkRange>> for DayPattern {
    type Error = CalendarError;
    fn try_from(ranges: Vec<WorkRange>) -> Result<Self, CalendarError> {
        DayPattern::new(ranges)
    }
}

impl DayPattern {
    pub fn non_working() -> Self {
        DayPattern(Vec::new())
    }

    /// Sorts the ranges, merges ranges that touch, and rejects overlaps.
    pub fn new(mut ranges: Vec<WorkRange>) -> Result<Self, CalendarError> {
        ranges.sort();
        let mut merged: Vec<WorkRange> = Vec::with_capacity(ranges.len());
        for r in ranges {
            match merged.last_mut() {
                Some(last) if r.start < last.end => return Err(CalendarError::OverlappingRanges),
                Some(last) if r.start == last.end => last.end = r.end,
                _ => merged.push(r),
            }
        }
        Ok(DayPattern(merged))
    }

    pub fn ranges(&self) -> &[WorkRange] {
        &self.0
    }

    pub fn is_working(&self) -> bool {
        !self.0.is_empty()
    }

    pub fn working_minutes(&self) -> i64 {
        self.0.iter().map(|r| i64::from(r.end - r.start)).sum()
    }
}

/// A project calendar: a working week plus dated exceptions (R8).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "RawCalendar")]
pub struct Calendar {
    /// Monday first.
    week: [DayPattern; 7],
    /// Keyed by day number (see [`crate::time::day_number`]).
    exceptions: BTreeMap<i64, DayPattern>,
}

#[derive(Deserialize)]
struct RawCalendar {
    week: [DayPattern; 7],
    exceptions: BTreeMap<i64, DayPattern>,
}

impl TryFrom<RawCalendar> for Calendar {
    type Error = CalendarError;
    fn try_from(r: RawCalendar) -> Result<Self, CalendarError> {
        Ok(Calendar { exceptions: r.exceptions, ..Calendar::new(r.week)? })
    }
}

impl Calendar {
    /// Fails if the week has no working time at all (R9).
    pub fn new(week: [DayPattern; 7]) -> Result<Self, CalendarError> {
        if week.iter().all(|d| !d.is_working()) {
            return Err(CalendarError::NoWorkingTime);
        }
        Ok(Calendar { week, exceptions: BTreeMap::new() })
    }

    /// Monday to Friday, 08:00–12:00 and 13:00–17:00. Microsoft Project's "Standard" calendar.
    pub fn standard() -> Self {
        let day = DayPattern::new(vec![
            WorkRange::hours(8, 12).unwrap(),
            WorkRange::hours(13, 17).unwrap(),
        ])
        .unwrap();
        let off = DayPattern::non_working();
        Calendar::new([day.clone(), day.clone(), day.clone(), day.clone(), day, off.clone(), off]).unwrap()
    }

    pub fn week(&self) -> &[DayPattern; 7] {
        &self.week
    }

    /// Sets an exception for one date. An empty pattern makes the date a holiday.
    pub fn set_exception(&mut self, date: Date, pattern: DayPattern) {
        self.exceptions.insert(day_number(date), pattern);
    }

    pub fn remove_exception(&mut self, date: Date) -> Option<DayPattern> {
        self.exceptions.remove(&day_number(date))
    }

    pub fn exceptions(&self) -> impl Iterator<Item = (Date, &DayPattern)> {
        self.exceptions.iter().map(|(d, p)| (crate::time::date_from_day_number(*d), p))
    }

    /// The working pattern that applies on a given day number.
    pub fn pattern(&self, day: i64) -> &DayPattern {
        self.exceptions.get(&day).unwrap_or(&self.week[weekday_index(day)])
    }

    /// Working ranges of one day as absolute `(start, end)` moments.
    fn ranges_on(&self, day: i64) -> impl DoubleEndedIterator<Item = (i64, i64)> + '_ {
        let base = day * MINUTES_PER_DAY;
        self.pattern(day)
            .ranges()
            .iter()
            .map(move |r| (base + i64::from(r.start), base + i64::from(r.end)))
    }

    /// Working ranges from the day of `t` onwards. Never ends: the week has working time.
    fn ranges_from(&self, t: Moment) -> impl Iterator<Item = (i64, i64)> + '_ {
        (t.day()..).flat_map(|d| self.ranges_on(d))
    }

    /// Working ranges from the day of `t` backwards, latest first.
    fn ranges_back_from(&self, t: Moment) -> impl Iterator<Item = (i64, i64)> + '_ {
        (i64::MIN..=t.day()).rev().flat_map(|d| self.ranges_on(d).rev())
    }

    /// Start-type snap (R5): `t` itself if working time follows it, else the next range start.
    pub fn next_working(&self, t: Moment) -> Moment {
        let (s, _) = self.ranges_from(t).find(|&(_, e)| e > t.0).expect("week has working time");
        Moment(s.max(t.0))
    }

    /// Finish-type snap (R6): `t` itself if working time precedes it, else the previous range end.
    pub fn prev_working_end(&self, t: Moment) -> Moment {
        let (_, e) = self.ranges_back_from(t).find(|&(s, _)| s < t.0).expect("week has working time");
        Moment(e.min(t.0))
    }

    /// Adds `d` working minutes to `t` (R4). Negative `d` subtracts.
    ///
    /// For `d > 0` the result is finish-type (may be a range end). For `d < 0` it is
    /// start-type (may be a range start). `d == 0` returns `t` unchanged.
    pub fn add_working(&self, t: Moment, d: i64) -> Moment {
        let mut remaining = d.abs();
        if d > 0 {
            for (s, e) in self.ranges_from(t).filter(|&(_, e)| e > t.0) {
                let from = s.max(t.0);
                if remaining <= e - from {
                    return Moment(from + remaining);
                }
                remaining -= e - from;
            }
        } else if d < 0 {
            for (s, e) in self.ranges_back_from(t).filter(|&(s, _)| s < t.0) {
                let to = e.min(t.0);
                if remaining <= to - s {
                    return Moment(to - remaining);
                }
                remaining -= to - s;
            }
        }
        t
    }

    /// Signed working minutes in `[a, b)`; negative when `b < a` (R7).
    pub fn working_between(&self, a: Moment, b: Moment) -> i64 {
        if b < a {
            return -self.working_between(b, a);
        }
        let clipped = |day: i64| -> i64 { self.ranges_on(day).map(|(s, e)| (e.min(b.0) - s.max(a.0)).max(0)).sum() };
        let (first, last) = (a.day(), b.day());
        if first == last {
            return clipped(first);
        }
        // Days strictly between lie wholly inside [a, b). Skip whole weeks without exceptions.
        let week: i64 = self.week.iter().map(DayPattern::working_minutes).sum();
        let mut total = clipped(first) + clipped(last);
        let mut day = first + 1;
        while day < last {
            if last - day >= 7 && self.exceptions.range(day..day + 7).next().is_none() {
                total += week;
                day += 7;
            } else {
                total += self.pattern(day).working_minutes();
                day += 1;
            }
        }
        total
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::civil::date;
    use proptest::prelude::*;

    /// 2026-10-05 is a Monday.
    fn at(day: i8, h: i8, m: i8) -> Moment {
        Moment::from_datetime(date(2026, 10, day).at(h, m, 0, 0))
    }

    const HOUR: i64 = 60;
    const DAY: i64 = 8 * HOUR;

    #[test]
    fn ranges_are_validated() {
        assert!(WorkRange::new(600, 600).is_err());
        assert!(WorkRange::new(0, 1441).is_err());
        assert!(WorkRange::new(0, 1440).is_ok());
        let overlapping = vec![WorkRange::hours(8, 12).unwrap(), WorkRange::hours(11, 17).unwrap()];
        assert_eq!(DayPattern::new(overlapping), Err(CalendarError::OverlappingRanges));
    }

    #[test]
    fn deserializing_validates() {
        let c = Calendar::standard();
        let json = serde_json::to_string(&c).unwrap();
        assert_eq!(serde_json::from_str::<Calendar>(&json).unwrap(), c);
        let off = r#"{"week":[[],[],[],[],[],[],[]],"exceptions":{}}"#;
        assert!(serde_json::from_str::<Calendar>(off).is_err());
        assert!(serde_json::from_str::<WorkRange>(r#"{"start":600,"end":500}"#).is_err());
        assert!(serde_json::from_str::<DayPattern>(r#"[{"start":0,"end":600},{"start":500,"end":700}]"#).is_err());
    }

    #[test]
    fn touching_ranges_merge() {
        let p = DayPattern::new(vec![WorkRange::hours(13, 17).unwrap(), WorkRange::hours(8, 13).unwrap()]).unwrap();
        assert_eq!(p.ranges(), &[WorkRange::hours(8, 17).unwrap()]);
    }

    #[test]
    fn week_without_working_time_is_rejected() {
        let off = DayPattern::non_working();
        let week = std::array::from_fn(|_| off.clone());
        assert_eq!(Calendar::new(week), Err(CalendarError::NoWorkingTime));
    }

    #[test]
    fn next_working_snaps_forward() {
        let c = Calendar::standard();
        assert_eq!(c.next_working(at(5, 9, 0)), at(5, 9, 0));
        assert_eq!(c.next_working(at(5, 7, 0)), at(5, 8, 0));
        assert_eq!(c.next_working(at(5, 12, 0)), at(5, 13, 0)); // lunch
        assert_eq!(c.next_working(at(5, 17, 0)), at(6, 8, 0)); // end of day
        assert_eq!(c.next_working(at(9, 17, 0)), at(12, 8, 0)); // Friday evening to Monday
        assert_eq!(c.next_working(at(10, 10, 0)), at(12, 8, 0)); // Saturday
    }

    #[test]
    fn prev_working_end_snaps_back() {
        let c = Calendar::standard();
        assert_eq!(c.prev_working_end(at(5, 9, 0)), at(5, 9, 0));
        assert_eq!(c.prev_working_end(at(5, 17, 0)), at(5, 17, 0));
        assert_eq!(c.prev_working_end(at(6, 8, 0)), at(5, 17, 0));
        assert_eq!(c.prev_working_end(at(5, 12, 30)), at(5, 12, 0));
        assert_eq!(c.prev_working_end(at(12, 8, 0)), at(9, 17, 0)); // Monday morning to Friday
    }

    #[test]
    fn zero_adds_nothing_and_does_not_snap() {
        let c = Calendar::standard();
        assert_eq!(c.add_working(at(5, 17, 0), 0), at(5, 17, 0));
        assert_eq!(c.add_working(at(10, 3, 0), 0), at(10, 3, 0));
    }

    #[test]
    fn add_one_day_from_morning_finishes_same_evening() {
        let c = Calendar::standard();
        assert_eq!(c.add_working(at(5, 8, 0), DAY), at(5, 17, 0));
    }

    #[test]
    fn add_skips_lunch_and_weekends() {
        let c = Calendar::standard();
        assert_eq!(c.add_working(at(5, 11, 0), 2 * HOUR), at(5, 14, 0));
        assert_eq!(c.add_working(at(9, 8, 0), 2 * DAY), at(12, 17, 0));
        assert_eq!(c.add_working(at(10, 9, 0), HOUR), at(12, 9, 0)); // starts on Saturday
    }

    #[test]
    fn subtract_mirrors_add() {
        let c = Calendar::standard();
        assert_eq!(c.add_working(at(5, 17, 0), -DAY), at(5, 8, 0));
        assert_eq!(c.add_working(at(12, 9, 0), -2 * HOUR), at(9, 16, 0));
        assert_eq!(c.add_working(at(5, 14, 0), -2 * HOUR), at(5, 11, 0));
        assert_eq!(c.add_working(at(6, 8, 0), -HOUR), at(5, 16, 0));
    }

    #[test]
    fn holiday_exception_is_skipped() {
        let mut c = Calendar::standard();
        c.set_exception(date(2026, 10, 6), DayPattern::non_working());
        assert_eq!(c.add_working(at(5, 8, 0), 2 * DAY), at(7, 17, 0));
        assert_eq!(c.next_working(at(5, 17, 0)), at(7, 8, 0));
    }

    #[test]
    fn working_exception_on_weekend_is_used() {
        let mut c = Calendar::standard();
        c.set_exception(date(2026, 10, 10), DayPattern::new(vec![WorkRange::hours(9, 13).unwrap()]).unwrap());
        assert_eq!(c.add_working(at(9, 13, 0), 8 * HOUR), at(10, 13, 0));
    }

    #[test]
    fn partial_day_exception() {
        let mut c = Calendar::standard();
        c.set_exception(date(2026, 10, 6), DayPattern::new(vec![WorkRange::hours(8, 10).unwrap()]).unwrap());
        assert_eq!(c.add_working(at(6, 8, 0), 3 * HOUR), at(7, 9, 0));
        assert_eq!(c.working_between(at(5, 8, 0), at(7, 8, 0)), DAY + 2 * HOUR);
    }

    #[test]
    fn working_between_is_signed() {
        let c = Calendar::standard();
        assert_eq!(c.working_between(at(5, 8, 0), at(5, 17, 0)), DAY);
        assert_eq!(c.working_between(at(5, 17, 0), at(5, 8, 0)), -DAY);
        assert_eq!(c.working_between(at(9, 8, 0), at(12, 17, 0)), 2 * DAY);
        assert_eq!(c.working_between(at(10, 0, 0), at(11, 23, 0)), 0);
    }

    #[test]
    fn night_shift_across_midnight() {
        // Monday to Friday 22:00–24:00, Tuesday to Saturday 00:00–06:00.
        let late = DayPattern::new(vec![WorkRange::hours(22, 24).unwrap()]).unwrap();
        let early_late = DayPattern::new(vec![WorkRange::hours(0, 6).unwrap(), WorkRange::hours(22, 24).unwrap()]).unwrap();
        let early = DayPattern::new(vec![WorkRange::hours(0, 6).unwrap()]).unwrap();
        let off = DayPattern::non_working();
        let c = Calendar::new([late, early_late.clone(), early_late.clone(), early_late.clone(), early_late, early, off]).unwrap();
        assert_eq!(c.add_working(at(5, 22, 0), DAY), at(6, 6, 0));
        // Exactly 24:00 on Monday is still within the shift for a finish.
        assert_eq!(c.prev_working_end(at(6, 0, 0)), at(6, 0, 0));
        assert_eq!(c.next_working(at(6, 0, 0)), at(6, 0, 0));
        assert_eq!(c.working_between(at(5, 23, 0), at(6, 1, 0)), 2 * HOUR);
    }

    fn calendars() -> impl Strategy<Value = Calendar> {
        let range = (0u16..1440, 1u16..600).prop_filter_map("in day", |(s, len)| WorkRange::new(s, s.checked_add(len)?).ok());
        let day = prop::collection::vec(range, 0..3).prop_filter_map("no overlap", |r| DayPattern::new(r).ok());
        let week = prop::array::uniform7(day);
        let exceptions = prop::collection::vec((0i64..60, prop::bool::ANY), 0..6);
        (week, exceptions).prop_filter_map("has working time", |(week, ex)| {
            let mut c = Calendar::new(week).ok()?;
            let base = day_number(date(2026, 10, 5));
            for (offset, holiday) in ex {
                let pattern = if holiday {
                    DayPattern::non_working()
                } else {
                    DayPattern::new(vec![WorkRange::hours(9, 11).unwrap()]).unwrap()
                };
                c.exceptions.insert(base + offset, pattern);
            }
            Some(c)
        })
    }

    fn moments() -> impl Strategy<Value = Moment> {
        let base = at(5, 0, 0).0;
        (0i64..60 * MINUTES_PER_DAY).prop_map(move |m| Moment(base + m))
    }

    /// Day-by-day reference for [`Calendar::working_between`].
    fn working_between_naive(c: &Calendar, a: Moment, b: Moment) -> i64 {
        if b < a {
            return -working_between_naive(c, b, a);
        }
        (a.day()..=b.day()).flat_map(|d| c.ranges_on(d)).map(|(s, e)| (e.min(b.0) - s.max(a.0)).max(0)).sum()
    }

    proptest! {
        #[test]
        fn fast_between_matches_naive(c in calendars(), a in moments(), span in -400 * MINUTES_PER_DAY..400 * MINUTES_PER_DAY) {
            let b = Moment(a.0 + span);
            prop_assert_eq!(c.working_between(a, b), working_between_naive(&c, a, b));
        }

        #[test]
        fn between_measures_what_add_added(c in calendars(), t in moments(), d in -20_000i64..20_000) {
            let r = c.add_working(t, d);
            prop_assert_eq!(c.working_between(t, r), d);
        }

        #[test]
        fn add_is_monotonic(c in calendars(), t in moments(), a in -20_000i64..20_000, b in -20_000i64..20_000) {
            let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
            prop_assert!(c.add_working(t, lo) <= c.add_working(t, hi));
        }

        #[test]
        fn add_is_additive(c in calendars(), t in moments(), a in 1i64..10_000, b in 1i64..10_000) {
            prop_assert_eq!(c.add_working(c.add_working(t, a), b), c.add_working(t, a + b));
            prop_assert_eq!(c.add_working(c.add_working(t, -a), -b), c.add_working(t, -a - b));
        }

        #[test]
        fn snaps_land_on_working_boundaries(c in calendars(), t in moments()) {
            let n = c.next_working(t);
            prop_assert!(n >= t);
            prop_assert_eq!(c.working_between(t, n), 0);
            prop_assert_eq!(c.add_working(n, 1).0, n.0 + 1, "a working minute follows a start snap");
            let p = c.prev_working_end(t);
            prop_assert!(p <= t);
            prop_assert_eq!(c.working_between(p, t), 0);
            prop_assert_eq!(c.add_working(p, -1).0, p.0 - 1, "a working minute precedes a finish snap");
        }
    }
}
