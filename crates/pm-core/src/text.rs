//! Typing durations and predecessors as text, as in Microsoft Project: `3d`, `4h`,
//! `2, 3SS+1d`. One implementation for the grid now and file import later.

use crate::model::{LagFormat, LinkType};

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
    #[error("type an amount like 1500 or 1,500.50")]
    BadMoney,
}

/// Reads an amount as people type it: `1500`, `1,500.50`, `1.500,50`, `€ 45`, `45.5`.
/// The last `.` or `,` followed by one or two digits is the decimal point; other separators
/// group thousands. Returns cents.
pub fn parse_money(text: &str) -> Result<i64, TextError> {
    let t: String = text.chars().filter(|c| c.is_ascii_digit() || matches!(c, '.' | ',' | '-')).collect();
    let negative = t.starts_with('-');
    let t = t.trim_start_matches('-');
    if t.is_empty() || t.contains('-') {
        return Err(TextError::BadMoney);
    }
    let (whole, cents) = match t.rfind(['.', ',']) {
        Some(i) if (1..=2).contains(&(t.len() - i - 1)) => {
            let frac = &t[i + 1..];
            (&t[..i], if frac.len() == 1 { format!("{frac}0") } else { frac.to_owned() })
        }
        _ => (t, "00".to_owned()),
    };
    let digits: String = whole.chars().filter(char::is_ascii_digit).collect();
    let whole: i64 = if digits.is_empty() { 0 } else { digits.parse().map_err(|_| TextError::BadMoney)? };
    let cents: i64 = cents.parse().map_err(|_| TextError::BadMoney)?;
    let total = whole.checked_mul(100).and_then(|w| w.checked_add(cents)).ok_or(TextError::BadMoney)?;
    Ok(if negative { -total } else { total })
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

    /// Like [`Units::parse_signed`], and also elapsed units (R36): `3ed`, `4eh`, `30em`, `2ew`
    /// count clock time, nights and weekends included. Returns the minutes and whether elapsed.
    pub fn parse_signed_spec(&self, text: &str) -> Result<(i64, bool), TextError> {
        let s = text.trim().to_lowercase();
        let digits_end = s.trim_start_matches(['+', '-']).find(|c: char| !(c.is_ascii_digit() || c == '.' || c == ',' || c == ' ')).map(|k| k + (s.len() - s.trim_start_matches(['+', '-']).len()));
        if let Some(k) = digits_end {
            let unit = s[k..].trim();
            let per = match unit {
                "ed" | "eday" | "edays" => Some(1440),
                "eh" | "ehr" | "ehrs" | "ehour" | "ehours" => Some(60),
                "em" | "emin" | "emins" => Some(1),
                "ew" | "ewk" | "ewks" | "eweek" | "eweeks" => Some(10_080),
                _ => None,
            };
            if let Some(per) = per {
                let (sign, number) = match s[..k].trim().strip_prefix('-') {
                    Some(r) => (-1.0, r.to_owned()),
                    None => (1.0, s[..k].trim().trim_start_matches('+').to_owned()),
                };
                let value: f64 = number.trim().replace(',', ".").parse().map_err(|_| TextError::BadDuration)?;
                let minutes = (sign * value * per as f64).round();
                if !minutes.is_finite() || minutes.abs() > 1e12 {
                    return Err(TextError::BadDuration);
                }
                return Ok((minutes as i64, true));
            }
        }
        Ok((self.parse_signed(text)?, false))
    }

    /// A duration as typed: minutes and whether it is elapsed; never negative.
    pub fn parse_duration_spec(&self, text: &str) -> Result<(i64, bool), TextError> {
        match self.parse_signed_spec(text)? {
            (m, _) if m < 0 => Err(TextError::NegativeDuration),
            ok => Ok(ok),
        }
    }

    /// Elapsed minutes as typed: `3ed`, `1.5ed`, `4eh`, `30em`.
    pub fn format_elapsed(&self, minutes: i64) -> String {
        let decimal = |num: i64, den: i64, unit: &str| {
            let s = format!("{:.2}", num as f64 / den as f64);
            format!("{}{unit}", s.trim_end_matches('0').trim_end_matches('.'))
        };
        if minutes == 0 {
            "0ed".into()
        } else if (minutes * 100) % 1440 == 0 {
            decimal(minutes, 1440, "ed")
        } else if (minutes * 100) % 60 == 0 {
            decimal(minutes, 60, "eh")
        } else {
            format!("{minutes}em")
        }
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
                let (lag, lag_format) = if rest.is_empty() {
                    (0, LagFormat::Working)
                } else if !rest.starts_with(['+', '-']) {
                    return Err(TextError::BadPredecessor);
                } else if let Some(p) = rest.strip_suffix('%') {
                    // A share of the predecessor's duration (R37): `+50%`.
                    let v: f64 = p.trim().trim_start_matches('+').replace(',', ".").parse().map_err(|_| TextError::BadPredecessor)?;
                    (v.round() as i64, LagFormat::Percent)
                } else {
                    match self.parse_signed_spec(rest).map_err(|_| TextError::BadPredecessor)? {
                        (m, true) => (m, LagFormat::Elapsed),
                        (m, false) => (m, LagFormat::Working),
                    }
                };
                Ok(PredecessorEntry { row, kind, lag, lag_format })
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
        let amount = |lag: i64| match e.lag_format {
            LagFormat::Working => self.format(lag),
            LagFormat::Elapsed => self.format_elapsed(lag),
            LagFormat::Percent => format!("{lag}%"),
        };
        match (e.kind, e.lag) {
            (LinkType::FinishToStart, 0) => e.row.to_string(),
            (_, 0) => format!("{}{kind}", e.row),
            (_, lag) => format!("{}{kind}{}{}", e.row, if lag > 0 { "+" } else { "-" }, amount(lag.abs())),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PredecessorEntry {
    /// 1-based row number in outline order.
    pub row: usize,
    pub kind: LinkType,
    pub lag: i64,
    pub lag_format: LagFormat,
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
                PredecessorEntry { row: 2, kind: FinishToStart, lag: 0, lag_format: LagFormat::Working },
                PredecessorEntry { row: 3, kind: StartToStart, lag: 480, lag_format: LagFormat::Working },
                PredecessorEntry { row: 7, kind: FinishToFinish, lag: -240, lag_format: LagFormat::Working },
                PredecessorEntry { row: 9, kind: StartToFinish, lag: 0, lag_format: LagFormat::Working },
            ])
        );
        assert_eq!(u.parse_predecessors("4+2d"), Ok(vec![PredecessorEntry { row: 4, kind: FinishToStart, lag: 960, lag_format: LagFormat::Working }]));
        for bad in ["x", "3XX", "3SS 2d", "SS", "3SS+q"] {
            assert_eq!(u.parse_predecessors(bad), Err(TextError::BadPredecessor), "{bad:?}");
        }
    }

    #[test]
    fn money() {
        for (text, cents) in [
            ("1500", 150_000),
            ("1,500.50", 150_050),
            ("1.500,50", 150_050),
            ("€ 45", 4_500),
            ("45.5", 4_550),
            ("0,99", 99),
            ("12,345,678", 1_234_567_800),
            ("CHF 1'200.00", 120_000),
            ("-20", -2_000),
        ] {
            assert_eq!(parse_money(text), Ok(cents), "{text}");
        }
        for bad in ["", "abc", "1-2"] {
            assert_eq!(parse_money(bad), Err(TextError::BadMoney), "{bad:?}");
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
            let e = PredecessorEntry { row, kind, lag: lag_minutes, lag_format: LagFormat::Working };
            let text = u.format_predecessor(&e);
            prop_assert_eq!(u.parse_predecessors(&text), Ok(vec![e]), "{}", text);
        }
    }

    #[test]
    fn elapsed_durations_and_percentage_lags() {
        let u = Units::default();
        assert_eq!(u.parse_duration_spec("3ed"), Ok((3 * 1440, true)));
        assert_eq!(u.parse_duration_spec("2 ew"), Ok((2 * 10_080, true)));
        assert_eq!(u.parse_duration_spec("1.5eh"), Ok((90, true)));
        assert_eq!(u.parse_duration_spec("4h"), Ok((240, false)));
        assert_eq!(u.parse_duration_spec("-1ed"), Err(TextError::NegativeDuration));
        assert_eq!(u.format_elapsed(3 * 1440), "3ed");
        assert_eq!(u.format_elapsed(90), "1.5eh");
        let p = u.parse_predecessors("2FS+50%, 3SS+2ed, 4-1d").unwrap();
        assert_eq!((p[0].lag, p[0].lag_format), (50, LagFormat::Percent));
        assert_eq!((p[1].lag, p[1].lag_format), (2 * 1440, LagFormat::Elapsed));
        assert_eq!((p[2].lag, p[2].lag_format), (-480, LagFormat::Working));
        let back: Vec<String> = p.iter().map(|e| u.format_predecessor(e)).collect();
        assert_eq!(back, ["2FS+50%", "3SS+2ed", "4FS-1d"]);
    }
}
