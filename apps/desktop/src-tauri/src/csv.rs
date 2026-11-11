//! File > Export Task List as CSV: the task list as a spreadsheet opens it (Excel, Numbers,
//! Google Sheets). UTF-8 with a byte-order mark, so Excel reads accents and emoji correctly.

use crate::ipc::{Row, Snapshot};

/// One field, quoted when it holds a comma, quote or line break (RFC 4180).
fn field(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_owned()
    }
}

fn date(iso: &Option<String>) -> String {
    iso.as_deref().map(|d| d.replace('T', " ")[..16.min(d.len())].to_owned()).unwrap_or_default()
}

fn days(minutes: i64, per_day: i64) -> String {
    let d = minutes as f64 / per_day.max(1) as f64;
    if d.fract() == 0.0 { format!("{d:.0}") } else { format!("{d:.2}") }
}

fn line(r: &Row, number: usize, per_day: i64) -> String {
    let cells = [
        number.to_string(),
        r.wbs.clone(),
        format!("{}{}", "  ".repeat(r.depth as usize), r.name),
        (r.depth + 1).to_string(),
        if r.is_milestone { "0".into() } else { days(r.duration_minutes, per_day) },
        date(&r.start),
        date(&r.finish),
        r.percent_complete.to_string(),
        r.predecessors.clone(),
        r.resource_names.clone(),
        format!("{:.2}", r.cost_cents as f64 / 100.0),
        date(&r.deadline),
        if r.critical { "Yes".into() } else { String::new() },
        r.notes.clone(),
    ];
    cells.iter().map(|c| field(c)).collect::<Vec<_>>().join(",")
}

/// The whole list, header first.
pub fn task_list(s: &Snapshot) -> String {
    let header = "#,WBS,Name,Outline Level,Duration (days),Start,Finish,% Complete,Predecessors,Resources,Cost,Deadline,Critical,Notes";
    let mut out = String::from("\u{feff}");
    out.push_str(header);
    for (i, r) in s.rows.iter().enumerate() {
        out.push_str("\r\n");
        out.push_str(&line(r, i + 1, s.project.minutes_per_day));
    }
    out.push_str("\r\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fields_are_quoted_only_when_needed() {
        assert_eq!(field("Survey"), "Survey");
        assert_eq!(field("Paint, walls"), "\"Paint, walls\"");
        assert_eq!(field("The \"big\" one"), "\"The \"\"big\"\" one\"");
        assert_eq!(field("two\nlines"), "\"two\nlines\"");
    }

    #[test]
    fn the_sample_project_exports_one_line_per_task() {
        let dir = std::env::temp_dir().join(format!("super-pm-csv-{}", uuid::Uuid::now_v7()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = crate::sample::office_move(pm_core::Moment::from_datetime(jiff::civil::date(2026, 11, 2).at(8, 0, 0, 0)));
        let doc = crate::document::Document::create(&dir.join("a.pmproj"), p).unwrap();
        let snap = crate::ipc::snapshot(&doc);
        let csv = task_list(&snap);
        let lines: Vec<&str> = csv.trim_end().split("\r\n").collect();
        assert!(lines[0].starts_with("\u{feff}#,WBS,Name"));
        assert_eq!(lines.len(), snap.rows.len() + 1);
        assert!(lines[1].starts_with("1,1,"), "{}", lines[1]);
        let _ = std::fs::remove_dir_all(dir);
    }
}
