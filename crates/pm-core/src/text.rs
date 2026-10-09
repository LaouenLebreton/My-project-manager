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
    #[error("type resources as names, like Alex, Sam[50%], Concrete[2.5 tons]")]
    BadResource,
}

/// One entry of a typed resource list: a name and, if given, units in hundredths (50% = 50,
/// 2.5 tons = 250).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResourceEntry {
    pub name: String,
    pub units: Option<u32>,
}

/// Reads `Alex, Sam[50%], Concrete[2.5 tons]`, as in Microsoft Project's Resource Names.
pub fn parse_resource_list(text: &str) -> Result<Vec<ResourceEntry>, TextError> {
    text.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|entry| {
            let (name, units) = match entry.split_once('[') {
                None => (entry, None),
                Some((name, rest)) => {
                    let inner = rest.strip_suffix(']').ok_or(TextError::BadResource)?.trim();
                    // "50%", "0.5", "2.5 tons": the number, then an optional % or unit word.
                    let end = inner.find(|c: char| !(c.is_ascii_digit() || c == '.' || c == ',')).unwrap_or(inner.len());
                    let value: f64 = inner[..end].replace(',', ".").parse().map_err(|_| TextError::BadResource)?;
                    let hundredths = if inner[end..].trim_start().starts_with('%') { value } else { value * 100.0 };
                    if !(hundredths.is_finite() && (1.0..1e7).contains(&hundredths)) {
                        return Err(TextError::BadResource);
                    }
                    (name, Some(hundredths.round() as u32))
                }
            };
            let name = name.trim();
            if name.is_empty() || name.contains(']') {
                return Err(TextError::BadResource);
            }
            Ok(ResourceEntry { name: name.to_owned(), units })
        })
        .collect()
}

/// `Alex`, `Sam[50%]`, `Concrete[2.5 tons]`: full-time work and one unit of a material are
/// written without brackets.
pub fn format_resource(name: &str, material: Option<&str>, units: u32) -> String {
    let number = || {
        let s = format!("{:.2}", f64::from(units) / 100.0);
        s.trim_end_matches('0').trim_end_matches('.').to_owned()
    };
    match material {
        None if units == 100 => name.to_owned(),
        None => format!("{name}[{units}%]"),
        Some(_) if units == 100 => name.to_owned(),
        Some("") => format!("{name}[{}]", number()),
        Some(label) => format!("{name}[{} {label}]", number()),
    }
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

    #[test]
    fn resource_lists() {
        let e = |name: &str, units: Option<u32>| ResourceEntry { name: name.into(), units };
        assert_eq!(parse_resource_list(""), Ok(vec![]));
        assert_eq!(
            parse_resource_list("Alex, Sam[50%], Concrete[2.5 tons],Licence[3], Pat [0.5]"),
            Ok(vec![e("Alex", None), e("Sam", Some(50)), e("Concrete", Some(250)), e("Licence", Some(300)), e("Pat", Some(50))])
        );
        assert_eq!(parse_resource_list("Ana María Ortiz[150 %]"), Ok(vec![e("Ana María Ortiz", Some(150))]));
        for bad in ["[50%]", "Sam[", "Sam[x]", "Sam[0%]", "Sam]"] {
            assert_eq!(parse_resource_list(bad), Err(TextError::BadResource), "{bad:?}");
        }
        assert_eq!(format_resource("Alex", None, 100), "Alex");
        assert_eq!(format_resource("Sam", None, 50), "Sam[50%]");
        assert_eq!(format_resource("Concrete", Some("tons"), 250), "Concrete[2.5 tons]");
        assert_eq!(format_resource("Licence", Some(""), 300), "Licence[3]");
        assert_eq!(format_resource("Licence", Some("seats"), 100), "Licence");
    }

    proptest! {
        #[test]
        fn resource_text_roundtrips(name in "[A-Za-zÀ-ÿ][A-Za-zÀ-ÿ .'-]{0,20}[A-Za-zÀ-ÿ]", units in 1u32..100_000, material in any::<bool>()) {
            let label = if material { Some("tons") } else { None };
            let text = format_resource(&name, label, units);
            let parsed = parse_resource_list(&text).unwrap();
            prop_assert_eq!(parsed.len(), 1);
            prop_assert_eq!(&parsed[0].name, &name);
            prop_assert_eq!(parsed[0].units.unwrap_or(100), units);
        }

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
