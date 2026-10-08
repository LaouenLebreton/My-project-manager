//! MSPDI → Super PM project.

use std::collections::{BTreeMap, HashMap};

use jiff::civil::{Date, DateTime};
use pm_core::{
    apply, Calendar, CalendarId, Command, Constraint, DayPattern, Dependency, DependencyId, Duration, LinkType,
    Moment, Percent, Project, ProjectId, SortKey, Task, TaskId, WorkRange,
};
use uuid::Uuid;

use crate::report::{ImportReport, ReportBuilder};
use crate::xml::{self, Element};

#[derive(Debug, thiserror::Error)]
pub enum ImportError {
    #[error("{0}")]
    Xml(String),
    #[error("this XML file is not a Microsoft Project file (MSPDI)")]
    NotMspdi,
    #[error("the project has no working time in its calendar, so it cannot be scheduled")]
    NoWorkingTime,
}

/// Task fields that Super PM calculates itself. Listed once in the report, not as losses.
const RECALCULATED: &[&str] = &[
    "Start", "Finish", "EarlyStart", "EarlyFinish", "LateStart", "LateFinish", "TotalSlack", "FreeSlack",
    "StartSlack", "FinishSlack", "Critical", "OutlineNumber", "WBS", "Summary", "RemainingDuration",
    "StartVariance", "FinishVariance", "DurationVariance", "PreLeveledStart", "PreLeveledFinish",
];

/// Task fields that carry no project data (bookkeeping, display state or defaults).
const QUIET: &[&str] = &[
    "UID", "ID", "Name", "Type", "IsNull", "CreateDate", "GUID", "Estimated", "IgnoreResourceCalendar",
    "HideBar", "Rollup", "EffortDriven", "OverAllocated", "ResumeValid", "LevelAssignments", "LevelingCanSplit",
    "IsSubproject", "IsSubprojectReadOnly", "ExternalTask", "DurationFormat", "OutlineLevel", "Duration",
    "Milestone", "ConstraintType", "ConstraintDate", "PercentComplete", "Notes", "PredecessorLink", "Manual",
    "ManualStart", "ManualFinish", "ManualDuration", "Active", "CalendarUID", "PercentWorkComplete",
    "Priority", "LevelingDelay", "LevelingDelayFormat", "EarnedValueMethod", "IsPublished", "CommitmentType",
    "FixedCostAccrual", "Recurring", "Work", "Cost", "FixedCost", "RemainingWork", "RemainingCost",
    "ActualWork", "ActualCost", "ACWP", "BCWP", "BCWS", "CV", "SV", "PhysicalPercentComplete", "WorkVariance",
    "ActualOvertimeCost", "ActualOvertimeWork", "OvertimeCost", "OvertimeWork", "RemainingOvertimeCost",
    "RemainingOvertimeWork", "RegularWork", "DurationText", "StartText", "FinishText", "Contact",
    "ActualDuration", "ActualStart", "ActualFinish", "Deadline", "Baseline", "ExtendedAttribute", "Hyperlink",
    "HyperlinkAddress", "HyperlinkSubAddress", "TimephasedData", "FreeformDurationFormat", "StartOnNewDay",
    "IsRecurring", "IsSummary",
];

/// Duration and lag formats measured in elapsed (calendar) time rather than working time.
const ELAPSED_FORMATS: &[i64] = &[4, 6, 8, 10, 12, 20, 36, 38, 40, 42, 44, 52];
const PERCENT_FORMATS: &[i64] = &[19, 20, 51, 52];

/// Reads an MSPDI document. `new_id` supplies ids (UUID v7 in the app; fixed in tests).
pub fn import_mspdi(input: &str, new_id: &mut dyn FnMut() -> Uuid) -> Result<(Project, ImportReport), ImportError> {
    let root = xml::parse(input).map_err(|e| ImportError::Xml(e.0))?;
    if root.name != "Project" || root.child("Tasks").is_none() && root.child("Calendars").is_none() {
        return Err(ImportError::NotMspdi);
    }
    let mut r = ReportBuilder::default();

    let minutes_per_day = root.int_of("MinutesPerDay").unwrap_or(480).max(1);
    if minutes_per_day != 480 {
        r.approximated(
            &format!("The file uses {minutes_per_day} minutes per day; Super PM shows days as 480 minutes, so durations keep their exact length but show as different day counts"),
            None,
        );
    }
    if root.text_of("ScheduleFromStart") == Some("0") {
        r.approximated("The project is scheduled from its finish date in Microsoft Project; Super PM schedules it from its start", None);
    }
    report_project_options(&root, &mut r);

    let calendar = import_calendar(&root, &mut r)?;
    let start = root
        .text_of("StartDate")
        .and_then(parse_datetime)
        .unwrap_or_else(|| Moment::from_datetime(jiff::civil::date(2026, 1, 5).at(8, 0, 0, 0)));
    let name = root
        .raw_of("Title")
        .filter(|t| !t.trim().is_empty())
        .or_else(|| root.text_of("Name").map(|n| n.trim_end_matches(".xml").trim_end_matches(".mpp")))
        .unwrap_or("Imported Project")
        .to_owned();

    let mut project = Project {
        id: ProjectId(new_id()),
        name,
        start,
        calendar_id: CalendarId(new_id()),
        calendar,
        tasks: BTreeMap::new(),
        dependencies: BTreeMap::new(),
    };

    // Tasks, in file order (ID). The outline comes from OutlineLevel.
    let mut file_tasks: Vec<&Element> = root
        .child("Tasks")
        .map(|t| t.children("Task").collect())
        .unwrap_or_default();
    file_tasks.sort_by_key(|t| t.int_of("ID").unwrap_or(i64::MAX));
    let mut by_uid: HashMap<i64, TaskId> = HashMap::new();
    let mut durations: HashMap<TaskId, i64> = HashMap::new();
    let mut parents: Vec<(i64, TaskId)> = Vec::new(); // (outline level, id)
    let mut last_key: HashMap<Option<TaskId>, SortKey> = HashMap::new();
    let mut tasks_imported = 0;
    for t in &file_tasks {
        let uid = t.int_of("UID").unwrap_or(-1);
        let level = t.int_of("OutlineLevel").unwrap_or(1);
        if uid == 0 || level == 0 {
            continue; // the project summary task; Super PM shows the project itself instead
        }
        if t.flag("IsNull") {
            r.ignored("Blank rows", None);
            continue;
        }
        let name = t.raw_of("Name").unwrap_or("").to_owned();
        let label = if name.trim().is_empty() { format!("Task {uid}") } else { name.clone() };
        while parents.last().is_some_and(|&(l, _)| l >= level) {
            parents.pop();
        }
        if parents.last().is_some_and(|&(l, _)| level > l + 1) {
            r.approximated("Outline levels skipped in the file were closed up", Some(&label));
        }
        let parent = parents.last().map(|&(_, id)| id);
        let id = TaskId(new_id());
        let key = SortKey::between(last_key.get(&parent), None);
        last_key.insert(parent, key.clone());

        let format = t.int_of("DurationFormat").unwrap_or(7);
        let mut minutes = t.text_of("Duration").and_then(parse_iso_duration).unwrap_or(0);
        if ELAPSED_FORMATS.contains(&format) {
            minutes = minutes * minutes_per_day / (24 * 60);
            r.approximated("Elapsed durations (counted in calendar days) were converted to working time", Some(&label));
        }
        let is_milestone = t.flag("Milestone");
        if is_milestone && minutes > 0 {
            r.approximated("Milestones with a duration became zero-duration milestones", Some(&label));
        }
        let constraint = import_constraint(t, &label, &mut r);
        let percent = t.int_of("PercentComplete").unwrap_or(0).clamp(0, 100) as u8;
        if t.flag("Manual") {
            r.approximated("Manually scheduled tasks are scheduled automatically", Some(&label));
        }
        if t.text_of("Active") == Some("0") {
            r.approximated("Inactive tasks were imported as active tasks", Some(&label));
        }
        report_task_extras(t, &label, &mut r);

        project.tasks.insert(
            id,
            Task {
                id,
                parent,
                sort_key: key,
                name,
                duration: Duration::minutes(minutes.max(0)).expect("non-negative"),
                is_milestone,
                constraint,
                percent_complete: Percent::new(percent).expect("clamped"),
                notes: t.raw_of("Notes").unwrap_or("").to_owned(),
            },
        );
        by_uid.insert(uid, id);
        durations.insert(id, minutes);
        parents.push((level, id));
        tasks_imported += 1;
    }
    r.imported("Tasks, with names, outline, durations, milestones, notes and % complete", tasks_imported);

    // Links, through the same checks as links typed in the app.
    let mut links_imported = 0;
    for t in &file_tasks {
        let Some(&successor) = t.int_of("UID").and_then(|u| by_uid.get(&u)) else { continue };
        let succ_name = project.tasks[&successor].name.clone();
        for link in t.children("PredecessorLink") {
            if link.flag("CrossProject") {
                r.ignored("Links to tasks in other projects", Some(&succ_name));
                continue;
            }
            let Some(&predecessor) = link.int_of("PredecessorUID").and_then(|u| by_uid.get(&u)) else {
                r.ignored("Links to tasks that are not in the file", Some(&succ_name));
                continue;
            };
            let kind = match link.int_of("Type").unwrap_or(1) {
                0 => LinkType::FinishToFinish,
                2 => LinkType::StartToFinish,
                3 => LinkType::StartToStart,
                _ => LinkType::FinishToStart,
            };
            let lag_format = link.int_of("LagFormat").unwrap_or(7);
            let raw = link.int_of("LinkLag").unwrap_or(0);
            let lag = if PERCENT_FORMATS.contains(&lag_format) {
                r.approximated("Lags given as a percentage of the predecessor were converted to a fixed working time", Some(&succ_name));
                durations.get(&predecessor).copied().unwrap_or(0) * raw / 1000
            } else if ELAPSED_FORMATS.contains(&lag_format) {
                r.approximated("Elapsed lags (counted in calendar days) were converted to working time", Some(&succ_name));
                raw / 10 * minutes_per_day / (24 * 60)
            } else {
                raw / 10 // tenths of a minute
            };
            let dependency = Dependency { id: DependencyId(new_id()), predecessor, successor, kind, lag };
            match apply(&mut project, &Command::InsertLink { dependency }) {
                Ok(_) => links_imported += 1,
                Err(e) => r.ignored(&format!("Links that Super PM cannot hold yet ({e})"), Some(&succ_name)),
            }
        }
    }
    r.imported("Links, with type and lag", links_imported);

    for field in RECALCULATED {
        if file_tasks.iter().any(|t| t.child(field).is_some()) {
            r.recalculated(field);
        }
    }
    report_resources(&root, &mut r);
    Ok((project, r.finish()))
}

fn import_constraint(t: &Element, label: &str, r: &mut ReportBuilder) -> Constraint {
    let date = t.text_of("ConstraintDate").and_then(parse_datetime);
    match (t.int_of("ConstraintType").unwrap_or(0), date) {
        (0, _) => Constraint::AsSoonAsPossible,
        (2, Some(d)) => Constraint::MustStartOn(d),
        (4, Some(d)) => Constraint::StartNoEarlierThan(d),
        (1, _) => {
            r.approximated("“As late as possible” is not supported yet; these tasks are scheduled as soon as possible", Some(label));
            Constraint::AsSoonAsPossible
        }
        (kind, _) => {
            let name = match kind {
                2 => "Must start on (without a date)",
                3 => "Must finish on",
                4 => "Start no earlier than (without a date)",
                5 => "Start no later than",
                6 => "Finish no earlier than",
                7 => "Finish no later than",
                _ => "Unknown",
            };
            r.approximated(&format!("“{name}” constraints are not supported yet; these tasks are scheduled as soon as possible"), Some(label));
            Constraint::AsSoonAsPossible
        }
    }
}

/// Task data Super PM does not hold yet, named in plain words.
fn report_task_extras(t: &Element, label: &str, r: &mut ReportBuilder) {
    let named: &[(&str, &str)] = &[
        ("ActualStart", "Actual start and finish dates (progress tracking arrives in iteration 2)"),
        ("ActualDuration", "Actual durations (progress tracking arrives in iteration 2)"),
        ("Deadline", "Deadlines"),
        ("Baseline", "Baselines (arrive in iteration 2)"),
        ("ExtendedAttribute", "Custom fields"),
        ("Hyperlink", "Hyperlinks"),
        ("HyperlinkAddress", "Hyperlinks"),
        ("Cost", "Costs (arrive in iteration 2)"),
        ("FixedCost", "Costs (arrive in iteration 2)"),
        ("Work", "Work hours (resources arrive in iteration 2)"),
        ("TimephasedData", "Timephased data"),
        ("Contact", "Task contacts"),
    ];
    for (field, what) in named {
        if t.child(field).is_some_and(Element::has_content) {
            r.ignored(what, Some(label));
        }
    }
    if t.text_of("Recurring") == Some("1") || t.text_of("IsRecurring") == Some("1") {
        r.approximated("Recurring tasks were imported as single tasks", Some(label));
    }
    if t.int_of("CalendarUID").is_some_and(|c| c >= 0) {
        r.ignored("Task calendars (each task uses the project calendar)", Some(label));
    }
    if t.int_of("Priority").is_some_and(|p| p != 500) {
        r.ignored("Task priorities (used by resource leveling, iteration 2)", Some(label));
    }
    for c in &t.children {
        let known = QUIET.contains(&c.name.as_str()) || RECALCULATED.contains(&c.name.as_str());
        if !known && c.has_content() {
            r.ignored(&format!("Other task field: {}", c.name), Some(label));
        }
    }
}

fn report_resources(root: &Element, r: &mut ReportBuilder) {
    let resources = root
        .child("Resources")
        .map(|x| x.children("Resource").filter(|res| res.int_of("UID").unwrap_or(0) != 0).count())
        .unwrap_or(0);
    if resources > 0 {
        for _ in 0..resources {
            r.ignored("Resources (arrive in iteration 2)", None);
        }
    }
    let assignments = root
        .child("Assignments")
        .map(|x| x.children("Assignment").filter(|a| a.int_of("ResourceUID").unwrap_or(-65535) >= 0).count())
        .unwrap_or(0);
    for _ in 0..assignments {
        r.ignored("Resource assignments (arrive in iteration 2)", None);
    }
}

/// Project-level options Super PM does not use, as one line with examples.
fn report_project_options(root: &Element, r: &mut ReportBuilder) {
    const HANDLED: &[&str] = &[
        "Name", "Title", "StartDate", "FinishDate", "CalendarUID", "MinutesPerDay", "MinutesPerWeek",
        "DaysPerMonth", "ScheduleFromStart", "Calendars", "Tasks", "Resources", "Assignments", "SaveVersion",
        "CreationDate", "LastSaved", "Author", "Company", "Manager", "Subject", "Category", "Keywords",
        "Comments", "BuildNumber", "UID", "GUID", "CurrentDate", "StatusDate", "DefaultStartTime",
        "DefaultFinishTime", "ExtendedAttributes", "OutlineCodes", "WBSMasks", "Views", "Filters", "Groups",
        "Tables", "Maps", "Reports", "Drawings", "DataLinks", "VBAProjects",
    ];
    for c in &root.children {
        if !HANDLED.contains(&c.name.as_str()) && c.has_content() {
            r.ignored("Project options Super PM does not use", Some(&c.name));
        }
    }
    if root.text_of("StatusDate").is_some() {
        r.ignored("Status date (progress tracking arrives in iteration 2)", None);
    }
    if root.child("ExtendedAttributes").is_some_and(Element::has_content) {
        r.ignored("Custom field definitions", None);
    }
}

// Calendars ---------------------------------------------------------------------------------

fn import_calendar(root: &Element, r: &mut ReportBuilder) -> Result<Calendar, ImportError> {
    let calendars: Vec<&Element> = root.child("Calendars").map(|c| c.children("Calendar").collect()).unwrap_or_default();
    let by_uid = |uid: i64| calendars.iter().copied().find(|c| c.int_of("UID") == Some(uid));
    let Some(cal) = root.int_of("CalendarUID").and_then(by_uid).or_else(|| calendars.first().copied()) else {
        r.approximated("The file has no calendar; the standard calendar (Monday to Friday, 8–12 and 13–17) is used", None);
        return Ok(Calendar::standard());
    };
    let others = calendars.len() - 1;
    for _ in 0..others {
        r.ignored("Other calendars (resource and task calendars; only the project calendar is used)", None);
    }

    // A derived calendar inherits the days it does not define from its base.
    let mut week: [Option<DayPattern>; 7] = Default::default();
    let mut chain = vec![cal];
    while let Some(base) = chain.last().and_then(|c| c.int_of("BaseCalendarUID")).filter(|&u| u >= 0).and_then(by_uid) {
        if chain.len() > 8 || chain.iter().any(|c| std::ptr::eq(*c, base)) {
            break;
        }
        chain.push(base);
    }
    for c in chain.iter().rev() {
        for wd in c.child("WeekDays").into_iter().flat_map(|w| w.children("WeekDay")) {
            let day_type = wd.int_of("DayType").unwrap_or(-1);
            if (1..=7).contains(&day_type) {
                // MSPDI: 1 = Sunday … 7 = Saturday. Super PM: Monday first.
                let index = ((day_type + 5) % 7) as usize;
                week[index] = Some(day_pattern(wd, r));
            }
        }
    }
    let standard = Calendar::standard();
    let mut defaulted = false;
    let week: [DayPattern; 7] = std::array::from_fn(|i| {
        week[i].clone().unwrap_or_else(|| {
            defaulted = true;
            standard.week()[i].clone()
        })
    });
    if defaulted {
        r.approximated("Weekdays missing from the calendar use standard hours (8–12 and 13–17, Monday to Friday)", None);
    }
    let mut calendar = Calendar::new(week).map_err(|_| ImportError::NoWorkingTime)?;
    if cal.child("WorkWeeks").is_some_and(Element::has_content) {
        r.ignored("Alternative work weeks in the calendar", None);
    }

    // Exceptions: the newer <Exceptions> list, or the older form inside <WeekDays>.
    let mut count = 0;
    for c in chain.iter().rev() {
        let modern: Vec<&Element> = c.child("Exceptions").map(|e| e.children("Exception").collect()).unwrap_or_default();
        if !modern.is_empty() {
            for ex in modern {
                let name = ex.text_of("Name").unwrap_or("Exception").to_owned();
                let recurring = ex.int_of("Type").is_some_and(|t| t != 1) || ex.int_of("Period").is_some_and(|p| p > 1);
                if recurring {
                    r.ignored("Recurring calendar exceptions other than single or consecutive days", Some(&name));
                    continue;
                }
                count += apply_exception(&mut calendar, ex, r);
            }
        } else {
            for wd in c.child("WeekDays").into_iter().flat_map(|w| w.children("WeekDay")) {
                if wd.int_of("DayType") == Some(0) {
                    count += apply_exception(&mut calendar, wd, r);
                }
            }
        }
    }
    r.imported("Project calendar: working week and hours", 1);
    r.imported("Calendar exception days (holidays and changed hours)", count);
    Ok(calendar)
}

fn day_pattern(day: &Element, r: &mut ReportBuilder) -> DayPattern {
    if !day.flag("DayWorking") {
        return DayPattern::non_working();
    }
    let ranges: Vec<WorkRange> = day
        .child("WorkingTimes")
        .into_iter()
        .flat_map(|w| w.children("WorkingTime"))
        .filter_map(|wt| {
            let from = parse_time(wt.text_of("FromTime")?)?;
            let to = match parse_time(wt.text_of("ToTime")?)? {
                0 => 1440, // midnight at the end of the day
                t => t,
            };
            WorkRange::new(from, to).ok()
        })
        .collect();
    if ranges.is_empty() {
        // A working day with no hours listed uses the default working day.
        return Calendar::standard().week()[0].clone();
    }
    DayPattern::new(ranges).unwrap_or_else(|_| {
        r.approximated("Overlapping working hours in the calendar were merged", None);
        Calendar::standard().week()[0].clone()
    })
}

/// Applies one exception to every day it covers; returns the number of days.
fn apply_exception(calendar: &mut Calendar, ex: &Element, r: &mut ReportBuilder) -> usize {
    let period = ex.child("TimePeriod");
    let from = period.and_then(|p| p.text_of("FromDate")).and_then(parse_date);
    let to = period.and_then(|p| p.text_of("ToDate")).and_then(parse_date).or(from);
    let (Some(from), Some(to)) = (from, to) else {
        r.ignored("Calendar exceptions without dates", ex.text_of("Name"));
        return 0;
    };
    let pattern = day_pattern(ex, r);
    let mut day = from;
    let mut n = 0;
    while day <= to && n < 3660 {
        calendar.set_exception(day, pattern.clone());
        n += 1;
        day = day.tomorrow().unwrap_or(day);
    }
    n
}

// Values ------------------------------------------------------------------------------------

pub(crate) fn parse_datetime(s: &str) -> Option<Moment> {
    let dt: DateTime = s.trim().get(..19).unwrap_or(s).parse().ok()?;
    Some(Moment::from_datetime(dt))
}

fn parse_date(s: &str) -> Option<Date> {
    s.trim().get(..10)?.parse().ok()
}

/// "08:00:00" → minutes after midnight.
fn parse_time(s: &str) -> Option<u16> {
    let mut parts = s.trim().split(':');
    let h: u16 = parts.next()?.parse().ok()?;
    let m: u16 = parts.next().unwrap_or("0").parse().ok()?;
    (h <= 24 && m < 60).then_some(h * 60 + m)
}

/// ISO 8601 duration as MSPDI writes it ("PT16H0M0S", sometimes with days) → minutes.
pub(crate) fn parse_iso_duration(s: &str) -> Option<i64> {
    let s = s.trim();
    let (negative, s) = s.strip_prefix('-').map_or((false, s), |r| (true, r));
    let s = s.strip_prefix('P')?;
    let (date_part, time_part) = s.split_once('T').unwrap_or((s, ""));
    let mut total = 0.0;
    let mut take = |part: &str, units: &[(char, f64)]| -> Option<()> {
        let mut number = String::new();
        for c in part.chars() {
            if c.is_ascii_digit() || c == '.' {
                number.push(c);
            } else {
                let &(_, minutes) = units.iter().find(|(u, _)| *u == c)?;
                total += number.parse::<f64>().ok()? * minutes;
                number.clear();
            }
        }
        number.is_empty().then_some(())
    };
    take(date_part, &[('D', 1440.0)])?;
    take(time_part, &[('H', 60.0), ('M', 1.0), ('S', 1.0 / 60.0)])?;
    let minutes = total.round() as i64;
    Some(if negative { -minutes } else { minutes })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso_durations() {
        assert_eq!(parse_iso_duration("PT16H0M0S"), Some(960));
        assert_eq!(parse_iso_duration("PT0H30M0S"), Some(30));
        assert_eq!(parse_iso_duration("PT7H59M60S"), Some(480));
        assert_eq!(parse_iso_duration("P1DT2H"), Some(1560));
        assert_eq!(parse_iso_duration("PT1.5H"), Some(90));
        assert_eq!(parse_iso_duration("16 hours"), None);
    }

    #[test]
    fn times_and_dates() {
        assert_eq!(parse_time("08:00:00"), Some(480));
        assert_eq!(parse_time("24:00:00"), Some(1440));
        assert_eq!(parse_date("2026-12-25T00:00:00"), Some(jiff::civil::date(2026, 12, 25)));
        assert!(parse_datetime("2026-10-05T08:00:00").is_some());
    }

    #[test]
    fn not_a_project_file() {
        let mut ids = || Uuid::nil();
        assert!(matches!(import_mspdi("<Invoice><Total>3</Total></Invoice>", &mut ids), Err(ImportError::NotMspdi)));
        assert!(matches!(import_mspdi("{\"json\": true}", &mut ids), Err(ImportError::Xml(_))));
    }
}
