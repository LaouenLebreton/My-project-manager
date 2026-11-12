//! MSPDI → Super PM project.

use std::collections::{BTreeMap, HashMap};

use jiff::civil::{Date, DateTime};
use pm_core::{Actual, Money, 
    apply, Assignment, AssignmentId, Calendar, CalendarId, Command, Constraint, DayPattern, Dependency, DependencyId,
    Duration, LinkType, Moment, Percent, Project, ProjectId, Resource, ResourceId, ResourceKind, ResourceUnits, SortKey,
    Task, TaskId, WorkRange,
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
    "StartSlack", "FinishSlack", "Critical", "OutlineNumber", "WBS", "Summary", "RemainingDuration", "Work", "Cost",
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
        resources: Default::default(),
        assignments: Default::default(),
        currency: root.raw_of("CurrencySymbol").unwrap_or("").trim().to_owned(),
        baseline: None,
        status_date: root.text_of("StatusDate").and_then(parse_datetime),
    };

    // Tasks, in file order (ID). The outline comes from OutlineLevel.
    let mut file_tasks: Vec<&Element> = root
        .child("Tasks")
        .map(|t| t.children("Task").collect())
        .unwrap_or_default();
    file_tasks.sort_by_key(|t| t.int_of("ID").unwrap_or(i64::MAX));
    let mut by_uid: HashMap<i64, TaskId> = HashMap::new();
    let mut baseline: BTreeMap<TaskId, pm_core::BaselineTask> = BTreeMap::new();
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
        let minutes = t.text_of("Duration").and_then(parse_iso_duration).unwrap_or(0);
        // Elapsed durations count clock time (R36); the file gives them as clock time too.
        let elapsed = ELAPSED_FORMATS.contains(&format);
        // Task type (R39): 0 fixed units, 1 fixed duration, 2 fixed work.
        let task_type = match t.int_of("Type") {
            Some(1) => pm_core::TaskType::FixedDuration,
            Some(2) => pm_core::TaskType::FixedWork,
            _ => pm_core::TaskType::FixedUnits,
        };
        let is_milestone = t.flag("Milestone");
        if is_milestone && minutes > 0 {
            r.approximated("Milestones with a duration became zero-duration milestones", Some(&label));
        }
        let constraint = import_constraint(t, &label, &mut r);
        let percent = t.int_of("PercentComplete").unwrap_or(0).clamp(0, 100) as u8;
        // Manually scheduled tasks keep their start (ManualStart, or Start in older files).
        let manual_start = t.flag("Manual").then(|| t.text_of("ManualStart").or(t.text_of("Start")).and_then(parse_datetime)).flatten();
        if t.text_of("Active") == Some("0") {
            r.approximated("Inactive tasks were imported as active tasks", Some(&label));
        }
        report_task_extras(t, &label, &mut r);
        let actual = match (t.text_of("ActualStart").and_then(parse_datetime), t.text_of("ActualFinish").and_then(parse_datetime)) {
            (Some(start), Some(finish)) => Actual::Finished { start, finish: finish.max(start) },
            (Some(start), None) => Actual::Started { start },
            _ => Actual::NotStarted,
        };

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
                leveling_delay: Duration::ZERO,
                fixed_cost: Money(money(t.text_of("FixedCost"))),
                manual_start,
                deadline: t.text_of("Deadline").and_then(parse_datetime),
                elapsed,
                task_type,
                effort_driven: t.flag("EffortDriven"),
                actual,
                notes: t.raw_of("Notes").unwrap_or("").to_owned(),
            },
        );
        by_uid.insert(uid, id);
        for b in t.children("Baseline") {
            if b.int_of("Number").unwrap_or(0) != 0 {
                r.ignored("Baselines 1 to 10 (Super PM keeps the main baseline, Baseline 0)", Some(&label));
                continue;
            }
            if let (Some(start), Some(finish)) = (b.text_of("Start").and_then(parse_datetime), b.text_of("Finish").and_then(parse_datetime)) {
                baseline.insert(
                    id,
                    pm_core::BaselineTask {
                        start,
                        finish,
                        duration: b.text_of("Duration").and_then(parse_iso_duration).unwrap_or(0),
                        work: b.text_of("Work").and_then(parse_iso_duration).unwrap_or(0),
                        cost: Money(money(b.text_of("Cost"))),
                    },
                );
            }
        }
        parents.push((level, id));
        tasks_imported += 1;
    }
    r.imported("Tasks, with names, outline, durations, milestones, notes and % complete", tasks_imported);
    r.imported("Baseline dates, work and cost (Baseline 0)", baseline.len());
    if !baseline.is_empty() {
        project.baseline = Some(pm_core::Baseline { saved_at: "imported".into(), tasks: baseline });
    }
    // A summary's progress comes from its tasks, so only work tasks keep actual dates.
    let parents_of: std::collections::BTreeSet<TaskId> = project.tasks.values().filter_map(|t| t.parent).collect();
    for id in &parents_of {
        if let Some(t) = project.tasks.get_mut(id) {
            t.actual = Actual::NotStarted;
        }
    }
    let started = project.tasks.values().filter(|t| t.actual.start().is_some()).count();
    if started > 0 {
        r.imported("Actual start and finish dates", started);
    }

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
            // LinkLag is in tenths: of a minute, or of a percent for percentage lags (R37).
            let (lag, format) = if PERCENT_FORMATS.contains(&lag_format) {
                ((raw as f64 / 10.0).round() as i64, pm_core::LagFormat::Percent)
            } else if ELAPSED_FORMATS.contains(&lag_format) {
                (raw / 10, pm_core::LagFormat::Elapsed)
            } else {
                (raw / 10, pm_core::LagFormat::Working)
            };
            let dependency = Dependency { id: DependencyId(new_id()), predecessor, successor, kind, lag, lag_format: format };
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
    import_resources(&root, &mut project, &by_uid, new_id, &mut r);
    Ok((project, r.finish()))
}

fn import_constraint(t: &Element, label: &str, r: &mut ReportBuilder) -> Constraint {
    let date = t.text_of("ConstraintDate").and_then(parse_datetime);
    let kind = t.int_of("ConstraintType").unwrap_or(0);
    let constraint = match (kind, date) {
        (0, _) => Some(Constraint::AsSoonAsPossible),
        (1, _) => Some(Constraint::AsLateAsPossible),
        (2, Some(d)) => Some(Constraint::MustStartOn(d)),
        (3, Some(d)) => Some(Constraint::MustFinishOn(d)),
        (4, Some(d)) => Some(Constraint::StartNoEarlierThan(d)),
        (5, Some(d)) => Some(Constraint::StartNoLaterThan(d)),
        (6, Some(d)) => Some(Constraint::FinishNoEarlierThan(d)),
        (7, Some(d)) => Some(Constraint::FinishNoLaterThan(d)),
        _ => None,
    };
    constraint.unwrap_or_else(|| {
        r.approximated("Constraints without a date (or of an unknown type) became “as soon as possible”", Some(label));
        Constraint::AsSoonAsPossible
    })
}

/// Task data Super PM does not hold yet, named in plain words.
fn report_task_extras(t: &Element, label: &str, r: &mut ReportBuilder) {
    let named: &[(&str, &str)] = &[
        ("ExtendedAttribute", "Custom fields"),
        ("Hyperlink", "Hyperlinks"),
        ("HyperlinkAddress", "Hyperlinks"),
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

/// Resources and assignments (iteration 2). Assignments go through the same checks as typed
/// ones, so anything Super PM cannot hold is reported with the reason.
fn import_resources(root: &Element, p: &mut Project, tasks: &HashMap<i64, TaskId>, new_id: &mut dyn FnMut() -> Uuid, r: &mut ReportBuilder) {
    let mut by_uid: HashMap<i64, ResourceId> = HashMap::new();
    let mut last_key: Option<SortKey> = None;
    let mut resources = Vec::new();
    let mut file_resources: Vec<&Element> = root.child("Resources").map(|x| x.children("Resource").collect()).unwrap_or_default();
    file_resources.sort_by_key(|x| x.int_of("ID").unwrap_or(i64::MAX));
    for res in file_resources {
        let uid = res.int_of("UID").unwrap_or(-1);
        let name = res.raw_of("Name").unwrap_or("").trim().to_owned();
        if uid <= 0 || res.flag("IsNull") || name.is_empty() {
            continue; // the blank resource Microsoft Project keeps at UID 0
        }
        let kind = match res.int_of("Type").unwrap_or(1) {
            0 => ResourceKind::Material,
            1 => ResourceKind::Work,
            _ => {
                r.ignored("Cost resources (costs arrive later in iteration 2)", Some(&name));
                continue;
            }
        };
        // Names separate assignments in the Resources column, so they must be usable there.
        let mut clean: String = name.chars().map(|c| if matches!(c, ',' | '[' | ']') { ' ' } else { c }).collect();
        clean = clean.split_whitespace().collect::<Vec<_>>().join(" ");
        if clean != name {
            r.approximated("Resource names with commas or square brackets had them replaced by spaces", Some(&name));
        }
        if resources.iter().any(|x: &Resource| x.name.eq_ignore_ascii_case(&clean)) {
            clean = format!("{clean} ({uid})");
            r.approximated("Resources with the same name got their file number added", Some(&clean));
        }
        let extra_rates = ["OvertimeRate", "CostPerUse"].iter().any(|f| res.child(f).is_some_and(Element::has_content));
        if extra_rates {
            r.ignored("Overtime rates and cost per use (only the standard rate is used)", Some(&name));
        }
        let max = res.text_of("MaxUnits").and_then(|t| t.parse::<f64>().ok()).unwrap_or(1.0);
        let key = SortKey::between(last_key.as_ref(), None);
        last_key = Some(key.clone());
        let id = ResourceId(new_id());
        by_uid.insert(uid, id);
        resources.push(Resource {
            id,
            sort_key: key,
            initials: res.raw_of("Initials").unwrap_or("").trim().to_owned(),
            name: clean,
            kind,
            max_units: ResourceUnits(if kind == ResourceKind::Work { (max * 100.0).round().max(0.0) as u32 } else { 0 }),
            material_label: res.raw_of("MaterialLabel").unwrap_or("").trim().to_owned(),
            // StandardRate is read as an amount per hour whatever unit Project displays it in.
            // *verify* with a real file (docs/scheduling-rules.md, costs).
            rate: Money(money(res.text_of("StandardRate")).max(0)),
            notes: res.raw_of("Notes").unwrap_or("").to_owned(),
            availability: availability_of(root, res, &p.calendar, &name, r),
        });
    }
    r.imported("People's own calendars: days off and weekdays they do not work", resources.iter().filter(|x| !x.availability.is_full()).count());
    let count = resources.len();
    if count > 0 {
        apply(p, &Command::InsertResources { resources, assignments: vec![] }).expect("fresh resources with unique ids");
    }
    r.imported("Resources (people, equipment and materials), with maximum units", count);

    let mut assigned = 0;
    for a in root.child("Assignments").into_iter().flat_map(|x| x.children("Assignment")) {
        let Some(&resource) = a.int_of("ResourceUID").and_then(|u| by_uid.get(&u)) else { continue }; // unassigned or a cost resource
        let Some(&task) = a.int_of("TaskUID").and_then(|u| tasks.get(&u)) else { continue };
        let units = a.text_of("Units").and_then(|t| t.parse::<f64>().ok()).unwrap_or(1.0);
        let hundredths = (units * 100.0).round();
        let name = p.tasks[&task].name.clone();
        if hundredths < 1.0 {
            r.ignored("Assignments with 0% units", Some(&name));
            continue;
        }
        let assignment = Assignment { id: AssignmentId(new_id()), task, resource, units: ResourceUnits(hundredths as u32) };
        match apply(p, &Command::InsertAssignment { assignment }) {
            Ok(_) => assigned += 1,
            Err(e) => r.ignored(&format!("Assignments that Super PM cannot hold ({e})"), Some(&name)),
        }
    }
    r.imported("Assignments: which resources work on which tasks, at what units", assigned);
}

/// A person's own calendar (R38): weekdays they do not work and days off. Their hours are the
/// project's; other differences are reported.
fn availability_of(root: &Element, res: &Element, project: &Calendar, name: &str, r: &mut ReportBuilder) -> pm_core::Availability {
    let mut a = pm_core::Availability::default();
    let Some(uid) = res.int_of("CalendarUID").filter(|&u| u >= 0 && Some(u) != root.int_of("CalendarUID")) else { return a };
    let Some(cal) = root.child("Calendars").into_iter().flat_map(|c| c.children("Calendar")).find(|c| c.int_of("UID") == Some(uid)) else { return a };
    let mut hours_differ = false;
    for wd in cal.child("WeekDays").into_iter().flat_map(|w| w.children("WeekDay")) {
        let day_type = wd.int_of("DayType").unwrap_or(-1);
        if (1..=7).contains(&day_type) {
            let index = ((day_type + 5) % 7) as usize;
            if !wd.flag("DayWorking") {
                a.off_weekdays[index] = project.week()[index].is_working();
            } else if day_pattern(wd, r) != project.week()[index] {
                hours_differ = true;
            }
        } else if day_type == 0 && !wd.flag("DayWorking") {
            add_days_off(&mut a, wd);
        }
    }
    for ex in cal.child("Exceptions").into_iter().flat_map(|e| e.children("Exception")) {
        if ex.flag("DayWorking") {
            hours_differ = true;
        } else {
            add_days_off(&mut a, ex);
        }
    }
    if hours_differ {
        r.approximated("People's own working hours (Super PM uses the project's hours on the days they work)", Some(name));
    }
    a
}

fn add_days_off(a: &mut pm_core::Availability, ex: &Element) {
    let period = ex.child("TimePeriod");
    let from = period.and_then(|p| p.text_of("FromDate")).and_then(parse_date);
    let to = period.and_then(|p| p.text_of("ToDate")).and_then(parse_date).or(from);
    if let (Some(from), Some(to)) = (from, to) {
        let (from, to) = (pm_core::time::day_number(from), pm_core::time::day_number(to));
        a.days_off.extend((from..=to).take(3660));
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
        "Tables", "Maps", "Reports", "Drawings", "DataLinks", "VBAProjects", "CurrencySymbol", "CurrencyCode",
        "CurrencyDigits", "CurrencySymbolPosition",
    ];
    for c in &root.children {
        if !HANDLED.contains(&c.name.as_str()) && c.has_content() {
            r.ignored("Project options Super PM does not use", Some(&c.name));
        }
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
    // People's own calendars are read with the people (R38); any others are not used.
    let people: std::collections::HashSet<i64> =
        root.child("Resources").into_iter().flat_map(|x| x.children("Resource")).filter_map(|x| x.int_of("CalendarUID")).collect();
    let others = calendars.iter().filter(|c| !std::ptr::eq(**c, cal) && !c.int_of("UID").is_some_and(|u| people.contains(&u))).count();
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

/// A decimal currency amount ("1500.5") in cents; 0 if absent or unreadable.
fn money(text: Option<&str>) -> i64 {
    text.and_then(|t| t.trim().parse::<f64>().ok()).filter(|v| v.is_finite()).map_or(0, |v| (v * 100.0).round() as i64)
}

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
