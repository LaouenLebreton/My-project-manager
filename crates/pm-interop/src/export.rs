//! Super PM project → MSPDI that Microsoft Project opens.

use std::collections::HashMap;
use std::io::Cursor;

use pm_core::{outline, Constraint, DayPattern, LinkType, Moment, Project, ResourceKind, Schedule, TaskId};
use quick_xml::events::{BytesDecl, BytesEnd, BytesStart, BytesText, Event};
use quick_xml::Writer;

use crate::NAMESPACE;

type W = Writer<Cursor<Vec<u8>>>;

fn leaf(w: &mut W, name: &str, text: &str) {
    w.create_element(name).write_text_content(BytesText::new(text)).expect("writing to memory");
}

fn open(w: &mut W, name: &str) {
    w.write_event(Event::Start(BytesStart::new(name))).expect("writing to memory");
}

fn close(w: &mut W, name: &str) {
    w.write_event(Event::End(BytesEnd::new(name))).expect("writing to memory");
}

fn datetime(m: Moment) -> String {
    m.to_datetime().strftime("%Y-%m-%dT%H:%M:%S").to_string()
}

fn time_of_day(minutes: u16) -> String {
    if minutes >= 1440 {
        "00:00:00".into() // MSPDI writes the end of the day as midnight
    } else {
        format!("{:02}:{:02}:00", minutes / 60, minutes % 60)
    }
}

/// "PT16H0M0S": working time as MSPDI writes it.
fn iso_duration(minutes: i64) -> String {
    format!("PT{}H{}M0S", minutes / 60, minutes % 60)
}

/// Cents as a decimal amount: 150050 → "1500.5".
fn cents(c: i64) -> String {
    let s = format!("{}.{:02}", c / 100, (c % 100).abs());
    let s = if c < 0 && c > -100 { format!("-{s}") } else { s };
    s.trim_end_matches('0').trim_end_matches('.').to_owned()
}

fn working_times(w: &mut W, pattern: &DayPattern) {
    if pattern.is_working() {
        open(w, "WorkingTimes");
        for range in pattern.ranges() {
            open(w, "WorkingTime");
            leaf(w, "FromTime", &time_of_day(range.start()));
            leaf(w, "ToTime", &time_of_day(range.end()));
            close(w, "WorkingTime");
        }
        close(w, "WorkingTimes");
    }
}

/// Writes the project as MSPDI. `schedule` supplies the start and finish dates Microsoft
/// Project shows before it recalculates; without one, dates are omitted.
pub fn export_mspdi(p: &Project, schedule: Option<&Schedule>) -> String {
    let mut w = Writer::new_with_indent(Cursor::new(Vec::new()), b' ', 2);
    w.write_event(Event::Decl(BytesDecl::new("1.0", Some("UTF-8"), Some("yes")))).expect("writing to memory");
    w.write_event(Event::Start(BytesStart::new("Project").with_attributes([("xmlns", NAMESPACE)])))
        .expect("writing to memory");
    leaf(&mut w, "SaveVersion", "14");
    leaf(&mut w, "Name", &format!("{}.xml", p.name));
    leaf(&mut w, "Title", &p.name);
    leaf(&mut w, "ScheduleFromStart", "1");
    leaf(&mut w, "StartDate", &datetime(p.start));
    if let Some(s) = schedule {
        leaf(&mut w, "FinishDate", &datetime(s.project_finish));
    }
    leaf(&mut w, "CalendarUID", "1");
    let monday = &p.calendar.week()[0];
    let first = monday.ranges().first().map_or(480, |r| r.start());
    let last = monday.ranges().last().map_or(1020, |r| r.end());
    leaf(&mut w, "DefaultStartTime", &time_of_day(first));
    leaf(&mut w, "DefaultFinishTime", &time_of_day(last));
    leaf(&mut w, "MinutesPerDay", "480");
    leaf(&mut w, "MinutesPerWeek", "2400");
    leaf(&mut w, "DaysPerMonth", "20");
    if !p.currency.is_empty() {
        leaf(&mut w, "CurrencySymbol", &p.currency);
    }
    if let Some(d) = p.status_date {
        leaf(&mut w, "StatusDate", &datetime(d));
    }

    // Calendar: one base calendar.
    open(&mut w, "Calendars");
    open(&mut w, "Calendar");
    leaf(&mut w, "UID", "1");
    leaf(&mut w, "Name", "Standard");
    leaf(&mut w, "IsBaseCalendar", "1");
    leaf(&mut w, "BaseCalendarUID", "-1");
    open(&mut w, "WeekDays");
    for day_type in 1..=7 {
        // MSPDI: 1 = Sunday … 7 = Saturday. Super PM: Monday first.
        let pattern = &p.calendar.week()[((day_type + 5) % 7) as usize];
        open(&mut w, "WeekDay");
        leaf(&mut w, "DayType", &day_type.to_string());
        leaf(&mut w, "DayWorking", if pattern.is_working() { "1" } else { "0" });
        working_times(&mut w, pattern);
        close(&mut w, "WeekDay");
    }
    close(&mut w, "WeekDays");
    let exceptions: Vec<_> = p.calendar.exceptions().collect();
    if !exceptions.is_empty() {
        open(&mut w, "Exceptions");
        for (date, pattern) in exceptions {
            open(&mut w, "Exception");
            leaf(&mut w, "EnteredByOccurrences", "0");
            open(&mut w, "TimePeriod");
            leaf(&mut w, "FromDate", &format!("{date}T00:00:00"));
            leaf(&mut w, "ToDate", &format!("{date}T23:59:00"));
            close(&mut w, "TimePeriod");
            leaf(&mut w, "Occurrences", "1");
            leaf(&mut w, "Name", if pattern.is_working() { "Changed working hours" } else { "Non-working day" });
            leaf(&mut w, "Type", "1");
            leaf(&mut w, "DayWorking", if pattern.is_working() { "1" } else { "0" });
            working_times(&mut w, pattern);
            close(&mut w, "Exception");
        }
        close(&mut w, "Exceptions");
    }
    close(&mut w, "Calendar");
    // People's own working time (R38): a calendar based on the project's, with the weekdays
    // they do not work and their days off. UID 100 + their resource UID.
    let mut people: Vec<&pm_core::Resource> = p.resources.values().collect();
    people.sort_by(|a, b| a.sort_key.cmp(&b.sort_key).then(a.id.cmp(&b.id)));
    for (i, r) in people.iter().enumerate().filter(|(_, r)| !r.availability.is_full()) {
        open(&mut w, "Calendar");
        leaf(&mut w, "UID", &(101 + i).to_string());
        leaf(&mut w, "Name", &r.name);
        leaf(&mut w, "IsBaseCalendar", "0");
        leaf(&mut w, "BaseCalendarUID", "1");
        if r.availability.off_weekdays.contains(&true) {
            open(&mut w, "WeekDays");
            for day_type in 1..=7 {
                if r.availability.off_weekdays[((day_type + 5) % 7) as usize] {
                    open(&mut w, "WeekDay");
                    leaf(&mut w, "DayType", &day_type.to_string());
                    leaf(&mut w, "DayWorking", "0");
                    close(&mut w, "WeekDay");
                }
            }
            close(&mut w, "WeekDays");
        }
        // Consecutive days off become one exception.
        let mut ranges: Vec<(i64, i64)> = Vec::new();
        for &d in &r.availability.days_off {
            match ranges.last_mut() {
                Some((_, to)) if *to + 1 == d => *to = d,
                _ => ranges.push((d, d)),
            }
        }
        if !ranges.is_empty() {
            open(&mut w, "Exceptions");
            for (from, to) in ranges {
                open(&mut w, "Exception");
                leaf(&mut w, "EnteredByOccurrences", "0");
                open(&mut w, "TimePeriod");
                leaf(&mut w, "FromDate", &format!("{}T00:00:00", pm_core::time::date_from_day_number(from)));
                leaf(&mut w, "ToDate", &format!("{}T23:59:00", pm_core::time::date_from_day_number(to)));
                close(&mut w, "TimePeriod");
                leaf(&mut w, "Occurrences", "1");
                leaf(&mut w, "Name", "Day off");
                leaf(&mut w, "Type", "1");
                leaf(&mut w, "DayWorking", "0");
                close(&mut w, "Exception");
            }
            close(&mut w, "Exceptions");
        }
        close(&mut w, "Calendar");
    }
    close(&mut w, "Calendars");

    // Tasks in outline order. UIDs are row numbers; 0 is the project summary.
    let rows = outline(p);
    let uid: HashMap<TaskId, usize> = rows.iter().enumerate().map(|(i, r)| (r.id, i + 1)).collect();
    let mut outline_numbers: Vec<usize> = Vec::new();
    open(&mut w, "Tasks");
    open(&mut w, "Task");
    leaf(&mut w, "UID", "0");
    leaf(&mut w, "ID", "0");
    leaf(&mut w, "Name", &p.name);
    leaf(&mut w, "OutlineLevel", "0");
    leaf(&mut w, "Summary", "1");
    close(&mut w, "Task");
    for (i, row) in rows.iter().enumerate() {
        let t = &p.tasks[&row.id];
        outline_numbers.truncate(row.depth + 1);
        while outline_numbers.len() < row.depth + 1 {
            outline_numbers.push(0);
        }
        outline_numbers[row.depth] += 1;
        let duration = if t.is_milestone { 0 } else { t.duration.get() };
        let sched = schedule.and_then(|s| s.tasks.get(&row.id));
        open(&mut w, "Task");
        leaf(&mut w, "UID", &(i + 1).to_string());
        leaf(&mut w, "ID", &(i + 1).to_string());
        leaf(&mut w, "Name", &t.name);
        leaf(&mut w, "Type", "1"); // fixed duration
        leaf(&mut w, "IsNull", "0");
        leaf(&mut w, "OutlineNumber", &outline_numbers.iter().map(usize::to_string).collect::<Vec<_>>().join("."));
        leaf(&mut w, "OutlineLevel", &(row.depth + 1).to_string());
        if let Some(s) = sched {
            leaf(&mut w, "Start", &datetime(s.early_start));
            leaf(&mut w, "Finish", &datetime(s.early_finish));
        }
        let summary_duration = sched.filter(|_| row.has_children).map(|s| p.calendar.working_between(s.early_start, s.early_finish));
        let written = summary_duration.unwrap_or(duration);
        leaf(&mut w, "Duration", &iso_duration(written));
        // Microsoft Project's codes: 7 days, 5 hours; elapsed 8 days, 6 hours, 4 minutes.
        let format = if t.elapsed && !row.has_children {
            if written % 1440 == 0 { "8" } else if written % 60 == 0 { "6" } else { "4" }
        } else if written % 480 == 0 {
            "7"
        } else {
            "5"
        };
        leaf(&mut w, "DurationFormat", format);
        leaf(&mut w, "Manual", if t.manual_start.is_some() && !row.has_children { "1" } else { "0" });
        if let Some(m) = t.manual_start.filter(|_| !row.has_children) {
            leaf(&mut w, "ManualStart", &datetime(m));
        }
        // A summary's dates come from its tasks, so a milestone flag on it means nothing.
        leaf(&mut w, "Milestone", if t.is_milestone && !row.has_children { "1" } else { "0" });
        leaf(&mut w, "Summary", if row.has_children { "1" } else { "0" });
        leaf(&mut w, "PercentComplete", &t.percent_complete.get().to_string());
        if let Some(start) = t.actual.start().filter(|_| !row.has_children) {
            leaf(&mut w, "ActualStart", &datetime(start));
        }
        if let Some(finish) = t.actual.finish().filter(|_| !row.has_children) {
            leaf(&mut w, "ActualFinish", &datetime(finish));
        }
        let ctype = match t.constraint {
            Constraint::AsSoonAsPossible => "0",
            Constraint::AsLateAsPossible => "1",
            Constraint::MustStartOn(_) => "2",
            Constraint::MustFinishOn(_) => "3",
            Constraint::StartNoEarlierThan(_) => "4",
            Constraint::StartNoLaterThan(_) => "5",
            Constraint::FinishNoEarlierThan(_) => "6",
            Constraint::FinishNoLaterThan(_) => "7",
        };
        let cdate = t.constraint.date();
        leaf(&mut w, "ConstraintType", ctype);
        if let Some(d) = cdate {
            leaf(&mut w, "ConstraintDate", &datetime(d));
        }
        // Microsoft Project's order: Deadline comes after ConstraintDate.
        if let Some(d) = t.deadline {
            leaf(&mut w, "Deadline", &datetime(d));
        }
        leaf(&mut w, "CalendarUID", "-1");
        if t.fixed_cost.0 != 0 {
            leaf(&mut w, "FixedCost", &cents(t.fixed_cost.0));
        }
        if !t.notes.is_empty() {
            leaf(&mut w, "Notes", &t.notes);
        }
        if let Some(b) = p.baseline.as_ref().and_then(|b| b.tasks.get(&row.id)) {
            open(&mut w, "Baseline");
            leaf(&mut w, "Number", "0");
            leaf(&mut w, "Start", &datetime(b.start));
            leaf(&mut w, "Finish", &datetime(b.finish));
            leaf(&mut w, "Duration", &iso_duration(b.duration));
            leaf(&mut w, "Work", &iso_duration(b.work));
            leaf(&mut w, "Cost", &cents(b.cost.0));
            close(&mut w, "Baseline");
        }
        for d in p.dependencies.values().filter(|d| d.successor == row.id) {
            let Some(pred) = uid.get(&d.predecessor) else { continue };
            open(&mut w, "PredecessorLink");
            leaf(&mut w, "PredecessorUID", &pred.to_string());
            let kind = match d.kind {
                LinkType::FinishToFinish => "0",
                LinkType::FinishToStart => "1",
                LinkType::StartToFinish => "2",
                LinkType::StartToStart => "3",
            };
            leaf(&mut w, "Type", kind);
            leaf(&mut w, "CrossProject", "0");
            // Tenths of a minute, or of a percent (format 19).
            leaf(&mut w, "LinkLag", &(d.lag * 10).to_string());
            let format = match d.lag_format {
                pm_core::LagFormat::Percent => "19",
                pm_core::LagFormat::Elapsed => if d.lag % 1440 == 0 { "8" } else { "4" },
                pm_core::LagFormat::Working => if d.lag % 480 == 0 { "7" } else { "5" },
            };
            leaf(&mut w, "LagFormat", format);
            close(&mut w, "PredecessorLink");
        }
        close(&mut w, "Task");
    }
    close(&mut w, "Tasks");

    // Resources in Resource Sheet order (UIDs from 1), then assignments.
    let mut resources: Vec<&pm_core::Resource> = p.resources.values().collect();
    resources.sort_by(|a, b| a.sort_key.cmp(&b.sort_key).then(a.id.cmp(&b.id)));
    let resource_uid: HashMap<pm_core::ResourceId, usize> = resources.iter().enumerate().map(|(i, r)| (r.id, i + 1)).collect();
    let decimal = |hundredths: u32| {
        let s = format!("{:.2}", f64::from(hundredths) / 100.0);
        s.trim_end_matches('0').trim_end_matches('.').to_owned()
    };
    if !resources.is_empty() {
        open(&mut w, "Resources");
        for (i, r) in resources.iter().enumerate() {
            open(&mut w, "Resource");
            leaf(&mut w, "UID", &(i + 1).to_string());
            leaf(&mut w, "ID", &(i + 1).to_string());
            leaf(&mut w, "Name", &r.name);
            leaf(&mut w, "Type", if r.kind == ResourceKind::Material { "0" } else { "1" });
            leaf(&mut w, "IsNull", "0");
            leaf(&mut w, "Initials", &r.initials);
            if r.kind == ResourceKind::Material {
                leaf(&mut w, "MaterialLabel", &r.material_label);
            } else {
                leaf(&mut w, "MaxUnits", &decimal(r.max_units.0));
            }
            if r.rate.0 != 0 {
                leaf(&mut w, "StandardRate", &cents(r.rate.0));
                leaf(&mut w, "StandardRateFormat", "2"); // per hour
            }
            if !r.availability.is_full() {
                leaf(&mut w, "CalendarUID", &(100 + i + 1).to_string());
            }
            if !r.notes.is_empty() {
                leaf(&mut w, "Notes", &r.notes);
            }
            close(&mut w, "Resource");
        }
        close(&mut w, "Resources");
    }
    let mut assignments: Vec<&pm_core::Assignment> = p.assignments.values().filter(|a| uid.contains_key(&a.task) && resource_uid.contains_key(&a.resource)).collect();
    assignments.sort_by_key(|a| (uid[&a.task], resource_uid[&a.resource]));
    if !assignments.is_empty() {
        open(&mut w, "Assignments");
        for (i, a) in assignments.iter().enumerate() {
            open(&mut w, "Assignment");
            leaf(&mut w, "UID", &(i + 1).to_string());
            leaf(&mut w, "TaskUID", &uid[&a.task].to_string());
            leaf(&mut w, "ResourceUID", &resource_uid[&a.resource].to_string());
            leaf(&mut w, "Units", &decimal(a.units.0));
            let task = &p.tasks[&a.task];
            let minutes = if task.is_milestone { 0 } else { task.duration.get() * i64::from(a.units.0) / 100 };
            if p.resources[&a.resource].kind == ResourceKind::Work {
                leaf(&mut w, "Work", &iso_duration(minutes));
            }
            close(&mut w, "Assignment");
        }
        close(&mut w, "Assignments");
    }
    close(&mut w, "Project");
    String::from_utf8(w.into_inner().into_inner()).expect("quick-xml writes UTF-8")
}
