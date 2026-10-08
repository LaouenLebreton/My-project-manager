//! Typing durations and predecessors as text, as in Microsoft Project: `3d`, `4h`,
//! `2, 3SS+1d`. One implementation for the grid now and file import later.

use crate::model::LinkType;

/// Unit sizes in working minutes. Days and weeks follow the calendar's settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Units {
    pub minutes_per_day: i64,
    pub days_per_week: i64,
}

impl Default for Units {
    fn default() -> Self {
        Units { minutes_per_day: 480, days_per_week: 5 }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum TextError {
    #[error("type a duration like 3d, 4h or 2w")]
    BadDuration,
    #[error("a duration cannot be negative")]
    NegativeDuration,
    #[error("type predecessors as task numbers, like 2, 5SS or 7FF+2d")]
    BadPredecessor,
}

impl Units {
    /// Parses `3`, `3d`, `2.5 days`, `4h`, `30m`, `2w`, `+1d`, `-4h`. A bare number means days.
    /// Signed values are allowed (lags); results are rounded to whole minutes.
    pub fn parse_signed(&self, text: &str) -> Result<i64, TextError> {
        let s = text.trim().to_lowercase();
        let (sign, rest) = match s.strip_prefix('-') {
            Some(r) => (-1.0, r),
            None => (1.0, s.strip_prefix('+').unwrap_or(&s)),
        };
        let rest = rest.trim();
        let split = rest.find(|c: char| !(c.is_ascii_digit() || c == '.' || c == ',')).unwrap_or(rest.len());
        let (number, unit) = rest.split_at(split);
        let value: f64 = number.replace(',', ".").parse().map_err(|_| TextError::BadDuration)?;
        let per = match unit.trim() {
            "" | "d" | "day" | "days" | "dy" | "dys" => self.minutes_per_day,
            "h" | "hr" | "hrs" | "hour" | "hours" => 60,
            "m" | "min" | "mins" | "minute" | "minutes" => 1,
            "w" | "wk" | "wks" | "week" | "weeks" => self.minutes_per_day * self.days_per_week,
            _ => return Err(TextError::BadDuration),
        };
        let minutes = (sign * value * per as f64).round();
        if !minutes.is_finite() || minutes.abs() > 1e12 {
            return Err(TextError::BadDuration);
        }
        Ok(minutes as i64)
    }

    /// Like [`Units::parse_signed`] but rejects negative values.
    pub fn parse_duration(&self, text: &str) -> Result<i64, TextError> {
        match self.parse_signed(text)? {
            m if m < 0 => Err(TextError::NegativeDuration),
            m => Ok(m),
        }
    }

    /// Short, exact form for editing and the predecessor column: `3d`, `2.5d`, `9h`, `25m`.
    /// Decimals are used only when they are exact, so the text always reads back the same.
    pub fn format(&self, minutes: i64) -> String {
        let mpd = self.minutes_per_day;
        let decimal = |num: i64, den: i64, unit: &str| {
            let s = format!("{:.2}", num as f64 / den as f64);
            format!("{}{unit}", s.trim_end_matches('0').trim_end_matches('.'))
        };
        if minutes == 0 {
            "0d".into()
        } else if (minutes * 100) % mpd == 0 && (minutes % mpd == 0 || minutes.abs() >= mpd) {
            decimal(minutes, mpd, "d")
        } else if (minutes * 100) % 60 == 0 {
            decimal(minutes, 60, "h")
        } else {
            format!("{minutes}m")
        }
    }

    /// One predecessor entry: row number, link type and lag.
    pub fn parse_predecessors(&self, text: &str) -> Result<Vec<PredecessorEntry>, TextError> {
        text.split([',', ';'])
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|entry| {
                let digits = entry.find(|c: char| !c.is_ascii_digit()).unwrap_or(entry.len());
                let row: usize = entry[..digits].parse().map_err(|_| TextError::BadPredecessor)?;
                let rest = entry[digits..].trim_start();
                let upper = rest.get(..2).map(str::to_ascii_uppercase);
                let (kind, rest) = match upper.as_deref() {
                    Some("FS") => (LinkType::FinishToStart, &rest[2..]),
                    Some("SS") => (LinkType::StartToStart, &rest[2..]),
                    Some("FF") => (LinkType::FinishToFinish, &rest[2..]),
                    Some("SF") => (LinkType::StartToFinish, &rest[2..]),
                    _ => (LinkType::FinishToStart, rest),
                };
                let rest = rest.trim();
                let lag = if rest.is_empty() {
                    0
                } else if rest.starts_with(['+', '-']) {
                    self.parse_signed(rest).map_err(|_| TextError::BadPredecessor)?
                } else {
                    return Err(TextError::BadPredecessor);
                };
                Ok(PredecessorEntry { row, kind, lag })
            })
            .collect()
    }

    /// `3`, `3SS`, `3FF+2d`: the form Microsoft Project uses.
    pub fn format_predecessor(&self, e: &PredecessorEntry) -> String {
        let kind = match e.kind {
            LinkType::FinishToStart => "FS",
            LinkType::StartToStart => "SS",
            LinkType::FinishToFinish => "FF",
            LinkType::StartToFinish => "SF",
        };
        match (e.kind, e.lag) {
            (LinkType::FinishToStart, 0) => e.row.to_string(),
            (_, 0) => format!("{}{kind}", e.row),
            (_, lag) => format!("{}{kind}{}{}", e.row, if lag > 0 { "+" } else { "-" }, self.format(lag.abs())),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PredecessorEntry {
    /// 1-based row number in outline order.
    pub row: usize,
    pub kind: LinkType,
    pub lag: i64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use LinkType::*;

    #[test]
    fn durations() {
        let u = Units::default();
        for (text, minutes) in [
            ("3", 1440),
            ("3d", 1440),
            (" 2.5 days ", 1200),
            ("2,5d", 1200),
            ("4h", 240),
            ("4 HRS", 240),
            ("30m", 30),
            ("2w", 4800),
            ("0", 0),
            ("0.1h", 6),
        ] {
            assert_eq!(u.parse_duration(text), Ok(minutes), "{text}");
        }
        for bad in ["", "d", "3x", "three", "3 d d", "1e400"] {
            assert_eq!(u.parse_duration(bad), Err(TextError::BadDuration), "{bad:?}");
        }
        assert_eq!(u.parse_duration("-1d"), Err(TextError::NegativeDuration));
        assert_eq!(u.parse_signed("-1d"), Ok(-480));
    }

    #[test]
    fn formatting() {
        let u = Units::default();
        assert_eq!(u.format(0), "0d");
        assert_eq!(u.format(480), "1d");
        assert_eq!(u.format(1200), "2.5d");
        assert_eq!(u.format(240), "4h");
        assert_eq!(u.format(90), "1.5h");
        assert_eq!(u.format(25), "25m");
        assert_eq!(u.format(-480), "-1d");
        assert_eq!(u.format(540), "9h", "1.125 days is not exact in two decimals");
        assert_eq!(u.format(481), "481m");
    }

    #[test]
    fn predecessors() {
        let u = Units::default();
        assert_eq!(u.parse_predecessors(""), Ok(vec![]));
        assert_eq!(
            u.parse_predecessors("2, 3ss+1d; 7FF -4h,9sf"),
            Ok(vec![
                PredecessorEntry { row: 2, kind: FinishToStart, lag: 0 },
                PredecessorEntry { row: 3, kind: StartToStart, lag: 480 },
                PredecessorEntry { row: 7, kind: FinishToFinish, lag: -240 },
                PredecessorEntry { row: 9, kind: StartToFinish, lag: 0 },
            ])
        );
        assert_eq!(u.parse_predecessors("4+2d"), Ok(vec![PredecessorEntry { row: 4, kind: FinishToStart, lag: 960 }]));
        for bad in ["x", "3XX", "3SS 2d", "SS", "3SS+q"] {
            assert_eq!(u.parse_predecessors(bad), Err(TextError::BadPredecessor), "{bad:?}");
        }
    }

    proptest! {
        #[test]
        fn predecessor_text_roundtrips(row in 1usize..10_000, k in 0usize..4, lag_minutes in -100_000i64..100_000) {
            let u = Units::default();
            let kind = [FinishToStart, StartToStart, FinishToFinish, StartToFinish][k];
            let e = PredecessorEntry { row, kind, lag: lag_minutes };
            let text = u.format_predecessor(&e);
            prop_assert_eq!(u.parse_predecessors(&text), Ok(vec![e]), "{}", text);
        }
    }
}
