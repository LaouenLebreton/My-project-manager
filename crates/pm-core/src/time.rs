//! Naive local time as whole minutes since 1970-01-01T00:00 (R2).

use jiff::civil::{Date, DateTime, Time};
use serde::{Deserialize, Serialize};

pub const MINUTES_PER_DAY: i64 = 24 * 60;

/// An instant on the project's naive local timeline, in minutes since 1970-01-01T00:00.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Moment(pub i64);

impl Moment {
    pub fn from_day_minute(day: i64, minute: i64) -> Self {
        Moment(day * MINUTES_PER_DAY + minute)
    }

    /// Days since 1970-01-01 (negative before it).
    pub fn day(self) -> i64 {
        self.0.div_euclid(MINUTES_PER_DAY)
    }

    /// Minute of the day, 0..1440.
    pub fn minute_of_day(self) -> i64 {
        self.0.rem_euclid(MINUTES_PER_DAY)
    }

    pub fn from_datetime(dt: DateTime) -> Self {
        Moment::from_day_minute(
            day_number(dt.date()),
            i64::from(dt.hour()) * 60 + i64::from(dt.minute()),
        )
    }

    pub fn to_datetime(self) -> DateTime {
        let m = self.minute_of_day();
        date_from_day_number(self.day()).to_datetime(Time::constant((m / 60) as i8, (m % 60) as i8, 0, 0))
    }
}

/// Weekday index of a day number, Monday = 0 … Sunday = 6. 1970-01-01 was a Thursday.
pub fn weekday_index(day: i64) -> usize {
    (day + 3).rem_euclid(7) as usize
}

/// Days since 1970-01-01 for a civil date (Howard Hinnant's `days_from_civil`).
pub fn day_number(date: Date) -> i64 {
    let (y, m, d) = (i64::from(date.year()), i64::from(date.month()), i64::from(date.day()));
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Inverse of [`day_number`] (Howard Hinnant's `civil_from_days`).
pub fn date_from_day_number(z: i64) -> Date {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    Date::constant(y as i16, m as i8, d as i8)
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::civil::date;
    use proptest::prelude::*;

    #[test]
    fn epoch_is_day_zero_and_a_thursday() {
        assert_eq!(day_number(date(1970, 1, 1)), 0);
        assert_eq!(weekday_index(0), 3);
    }

    #[test]
    fn known_dates() {
        assert_eq!(day_number(date(2000, 3, 1)), 11_017);
        assert_eq!(day_number(date(1969, 12, 31)), -1);
        // 2026-10-08 is a Thursday.
        assert_eq!(weekday_index(day_number(date(2026, 10, 8))), 3);
    }

    #[test]
    fn moment_roundtrip_through_datetime() {
        let dt = date(2026, 10, 8).at(17, 30, 0, 0);
        let m = Moment::from_datetime(dt);
        assert_eq!(m.minute_of_day(), 17 * 60 + 30);
        assert_eq!(m.to_datetime(), dt);
    }

    proptest! {
        #[test]
        fn day_number_roundtrips_and_matches_weekday(n in -200_000i64..200_000) {
            let d = date_from_day_number(n);
            prop_assert_eq!(day_number(d), n);
            let jiff_weekday = d.weekday().to_monday_zero_offset() as usize;
            prop_assert_eq!(weekday_index(n), jiff_weekday);
        }
    }
}
