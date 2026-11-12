//! Mapping between model types and table rows.

use std::collections::BTreeMap;

use pm_core::{Actual, Baseline, BaselineTask, Money, 
    Assignment, AssignmentId, Calendar, CalendarId, Constraint, Dependency, DependencyId, Duration, LinkType, Moment,
    Percent, Project, ProjectId, Resource, ResourceId, ResourceKind, ResourceUnits, SortKey, Task, TaskId,
};
use rusqlite::{params, Connection, Row};
use uuid::Uuid;

use crate::StoreError;

fn uuid(s: String) -> Result<Uuid, StoreError> {
    Uuid::parse_str(&s).map_err(|e| StoreError::Damaged(format!("bad id {s:?}: {e}")))
}

fn link_code(k: LinkType) -> &'static str {
    match k {
        LinkType::FinishToStart => "FS",
        LinkType::StartToStart => "SS",
        LinkType::FinishToFinish => "FF",
        LinkType::StartToFinish => "SF",
    }
}

fn link_type(s: &str) -> Result<LinkType, StoreError> {
    Ok(match s {
        "FS" => LinkType::FinishToStart,
        "SS" => LinkType::StartToStart,
        "FF" => LinkType::FinishToFinish,
        "SF" => LinkType::StartToFinish,
        other => return Err(StoreError::Damaged(format!("unknown link type {other:?}"))),
    })
}

pub fn write_project(conn: &Connection, p: &Project) -> Result<(), StoreError> {
    conn.execute(
        "INSERT INTO project (singleton, id, name, start_min, calendar_id, currency, status_date_min) VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT (singleton) DO UPDATE SET id = ?1, name = ?2, start_min = ?3, calendar_id = ?4, currency = ?5, status_date_min = ?6",
        params![p.id.0.to_string(), p.name, p.start.0, p.calendar_id.0.to_string(), p.currency, p.status_date.map(|m| m.0)],
    )?;
    conn.execute(
        "INSERT INTO calendar (id, name, data) VALUES (?1, 'Project calendar', ?2)
         ON CONFLICT (id) DO UPDATE SET data = ?2",
        params![p.calendar_id.0.to_string(), serde_json::to_string(&p.calendar)?],
    )?;
    Ok(())
}

/// Replaces the saved baseline (or removes it).
pub fn write_baseline(conn: &Connection, b: Option<&Baseline>) -> Result<(), StoreError> {
    conn.execute("DELETE FROM baseline_task", [])?;
    conn.execute("UPDATE project SET baseline_saved_at = ?1 WHERE singleton = 1", [b.map(|b| b.saved_at.as_str())])?;
    if let Some(b) = b {
        let mut stmt = conn.prepare(
            "INSERT INTO baseline_task (task_id, start_min, finish_min, duration_min, work_min, cost_cents) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        )?;
        for (id, t) in &b.tasks {
            stmt.execute(params![id.0.to_string(), t.start.0, t.finish.0, t.duration, t.work, t.cost.0])?;
        }
    }
    Ok(())
}

fn read_baseline(conn: &Connection) -> Result<Option<Baseline>, StoreError> {
    let saved_at: Option<String> = conn.query_row("SELECT baseline_saved_at FROM project WHERE singleton = 1", [], |r| r.get(0))?;
    let Some(saved_at) = saved_at else { return Ok(None) };
    let mut tasks = BTreeMap::new();
    let mut stmt = conn.prepare("SELECT task_id, start_min, finish_min, duration_min, work_min, cost_cents FROM baseline_task")?;
    let mut q = stmt.query([])?;
    while let Some(r) = q.next()? {
        tasks.insert(
            TaskId(uuid(r.get(0)?)?),
            BaselineTask { start: Moment(r.get(1)?), finish: Moment(r.get(2)?), duration: r.get(3)?, work: r.get(4)?, cost: Money(r.get(5)?) },
        );
    }
    Ok(Some(Baseline { saved_at, tasks }))
}

pub fn upsert_task(conn: &Connection, t: &Task) -> Result<(), StoreError> {
    let ctype = match t.constraint {
        Constraint::AsSoonAsPossible => "ASAP",
        Constraint::AsLateAsPossible => "ALAP",
        Constraint::StartNoEarlierThan(_) => "SNET",
        Constraint::StartNoLaterThan(_) => "SNLT",
        Constraint::FinishNoEarlierThan(_) => "FNET",
        Constraint::FinishNoLaterThan(_) => "FNLT",
        Constraint::MustStartOn(_) => "MSO",
        Constraint::MustFinishOn(_) => "MFO",
    };
    let cdate = t.constraint.date().map(|m| m.0);
    conn.execute(
        "INSERT OR REPLACE INTO task
         (id, parent_id, sort_key, name, duration_min, is_milestone, constraint_type, constraint_min, percent_complete, notes, leveling_delay_min, fixed_cost_cents, manual_start_min, actual_start_min, actual_finish_min, deadline_min, elapsed, task_type, effort_driven)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19)",
        params![
            t.id.0.to_string(),
            t.parent.map(|p| p.0.to_string()),
            t.sort_key.0,
            t.name,
            t.duration.get(),
            t.is_milestone,
            ctype,
            cdate,
            t.percent_complete.get(),
            t.notes,
            t.leveling_delay.get(),
            t.fixed_cost.0,
            t.manual_start.map(|m| m.0),
            t.actual.start().map(|m| m.0),
            t.actual.finish().map(|m| m.0),
            t.deadline.map(|m| m.0),
            t.elapsed,
            task_type_code(t.task_type),
            t.effort_driven,
        ],
    )?;
    Ok(())
}

pub fn delete_task(conn: &Connection, id: TaskId) -> Result<(), StoreError> {
    conn.execute("DELETE FROM task WHERE id = ?1", [id.0.to_string()])?;
    Ok(())
}

pub fn upsert_dependency(conn: &Connection, d: &Dependency) -> Result<(), StoreError> {
    conn.execute(
        "INSERT OR REPLACE INTO dependency (id, predecessor_id, successor_id, kind, lag_min, lag_format) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![d.id.0.to_string(), d.predecessor.0.to_string(), d.successor.0.to_string(), link_code(d.kind), d.lag, lag_format_code(d.lag_format)],
    )?;
    Ok(())
}

pub fn delete_dependency(conn: &Connection, id: DependencyId) -> Result<(), StoreError> {
    conn.execute("DELETE FROM dependency WHERE id = ?1", [id.0.to_string()])?;
    Ok(())
}

pub fn upsert_resource(conn: &Connection, r: &Resource) -> Result<(), StoreError> {
    let kind = match r.kind {
        ResourceKind::Work => "work",
        ResourceKind::Material => "material",
    };
    conn.execute(
        "INSERT OR REPLACE INTO resource (id, sort_key, name, initials, kind, max_units, material_label, notes, rate_cents, availability)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            r.id.0.to_string(),
            r.sort_key.0,
            r.name,
            r.initials,
            kind,
            r.max_units.0,
            r.material_label,
            r.notes,
            r.rate.0,
            if r.availability.is_full() { String::new() } else { serde_json::to_string(&r.availability).expect("availability serializes") }
        ],
    )?;
    Ok(())
}

pub fn delete_resource(conn: &Connection, id: ResourceId) -> Result<(), StoreError> {
    conn.execute("DELETE FROM resource WHERE id = ?1", [id.0.to_string()])?;
    Ok(())
}

pub fn upsert_assignment(conn: &Connection, a: &Assignment) -> Result<(), StoreError> {
    conn.execute(
        "INSERT OR REPLACE INTO assignment (id, task_id, resource_id, units) VALUES (?1, ?2, ?3, ?4)",
        params![a.id.0.to_string(), a.task.0.to_string(), a.resource.0.to_string(), a.units.0],
    )?;
    Ok(())
}

pub fn delete_assignment(conn: &Connection, id: AssignmentId) -> Result<(), StoreError> {
    conn.execute("DELETE FROM assignment WHERE id = ?1", [id.0.to_string()])?;
    Ok(())
}

fn task_from_row(r: &Row) -> Result<Task, StoreError> {
    let constraint = match (r.get::<_, String>(6)?.as_str(), r.get::<_, Option<i64>>(7)?) {
        ("ASAP", None) => Constraint::AsSoonAsPossible,
        ("ALAP", None) => Constraint::AsLateAsPossible,
        ("SNET", Some(m)) => Constraint::StartNoEarlierThan(Moment(m)),
        ("SNLT", Some(m)) => Constraint::StartNoLaterThan(Moment(m)),
        ("FNET", Some(m)) => Constraint::FinishNoEarlierThan(Moment(m)),
        ("FNLT", Some(m)) => Constraint::FinishNoLaterThan(Moment(m)),
        ("MSO", Some(m)) => Constraint::MustStartOn(Moment(m)),
        ("MFO", Some(m)) => Constraint::MustFinishOn(Moment(m)),
        (t, d) => return Err(StoreError::Damaged(format!("bad constraint {t} {d:?}"))),
    };
    let percent: u8 = r.get(8)?;
    Ok(Task {
        id: TaskId(uuid(r.get(0)?)?),
        parent: r.get::<_, Option<String>>(1)?.map(uuid).transpose()?.map(TaskId),
        sort_key: SortKey(r.get(2)?),
        name: r.get(3)?,
        duration: Duration::minutes(r.get(4)?).map_err(|e| StoreError::Damaged(e.to_string()))?,
        is_milestone: r.get(5)?,
        constraint,
        percent_complete: Percent::new(percent).ok_or_else(|| StoreError::Damaged(format!("percent {percent}")))?,
        leveling_delay: Duration::minutes(r.get(10)?).map_err(|e| StoreError::Damaged(e.to_string()))?,
        fixed_cost: Money(r.get(11)?),
        manual_start: r.get::<_, Option<i64>>(12)?.map(Moment),
        deadline: r.get::<_, Option<i64>>(15)?.map(Moment),
        elapsed: r.get(16)?,
        task_type: task_type_of(&r.get::<_, String>(17)?)?,
        effort_driven: r.get(18)?,
        actual: match (r.get::<_, Option<i64>>(13)?, r.get::<_, Option<i64>>(14)?) {
            (None, None) => Actual::NotStarted,
            (Some(s), None) => Actual::Started { start: Moment(s) },
            (Some(s), Some(f)) => Actual::Finished { start: Moment(s), finish: Moment(f) },
            (None, Some(_)) => return Err(StoreError::Damaged("an actual finish without a start".into())),
        },
        notes: r.get(9)?,
    })
}

pub fn read_project(conn: &Connection) -> Result<Project, StoreError> {
    let (id, name, start, calendar_id, currency, status_date): (String, String, i64, String, String, Option<i64>) = conn
        .query_row("SELECT id, name, start_min, calendar_id, currency, status_date_min FROM project WHERE singleton = 1", [], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?))
        })
        .map_err(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => StoreError::Damaged("the project record is missing".into()),
            e => e.into(),
        })?;
    let data: String = conn.query_row("SELECT data FROM calendar WHERE id = ?1", [&calendar_id], |r| r.get(0))?;

    let mut tasks = BTreeMap::new();
    let mut stmt = conn.prepare(
        "SELECT id, parent_id, sort_key, name, duration_min, is_milestone, constraint_type, constraint_min, percent_complete, notes, leveling_delay_min, fixed_cost_cents, manual_start_min, actual_start_min, actual_finish_min, deadline_min, elapsed, task_type, effort_driven FROM task",
    )?;
    let mut q = stmt.query([])?;
    while let Some(r) = q.next()? {
        let t = task_from_row(r)?;
        tasks.insert(t.id, t);
    }

    let mut dependencies = BTreeMap::new();
    let mut stmt = conn.prepare("SELECT id, predecessor_id, successor_id, kind, lag_min, lag_format FROM dependency")?;
    let mut q = stmt.query([])?;
    while let Some(r) = q.next()? {
        let d = Dependency {
            id: DependencyId(uuid(r.get(0)?)?),
            predecessor: TaskId(uuid(r.get(1)?)?),
            successor: TaskId(uuid(r.get(2)?)?),
            kind: link_type(&r.get::<_, String>(3)?)?,
            lag: r.get(4)?,
            lag_format: lag_format_of(&r.get::<_, String>(5)?)?,
        };
        dependencies.insert(d.id, d);
    }

    let mut resources = BTreeMap::new();
    let mut stmt = conn.prepare("SELECT id, sort_key, name, initials, kind, max_units, material_label, notes, rate_cents, availability FROM resource")?;
    let mut q = stmt.query([])?;
    while let Some(r) = q.next()? {
        let kind = match r.get::<_, String>(4)?.as_str() {
            "work" => ResourceKind::Work,
            "material" => ResourceKind::Material,
            other => return Err(StoreError::Damaged(format!("unknown resource type {other:?}"))),
        };
        let res = Resource {
            id: ResourceId(uuid(r.get(0)?)?),
            sort_key: SortKey(r.get(1)?),
            name: r.get(2)?,
            initials: r.get(3)?,
            kind,
            max_units: ResourceUnits(r.get(5)?),
            material_label: r.get(6)?,
            rate: Money(r.get(8)?),
            notes: r.get(7)?,
            availability: match r.get::<_, String>(9)? {
                text if text.is_empty() => Default::default(),
                text => serde_json::from_str(&text).map_err(|e| StoreError::Damaged(format!("working time of a resource: {e}")))?,
            },
        };
        resources.insert(res.id, res);
    }
    let mut assignments = BTreeMap::new();
    let mut stmt = conn.prepare("SELECT id, task_id, resource_id, units FROM assignment")?;
    let mut q = stmt.query([])?;
    while let Some(r) = q.next()? {
        let a = Assignment {
            id: AssignmentId(uuid(r.get(0)?)?),
            task: TaskId(uuid(r.get(1)?)?),
            resource: ResourceId(uuid(r.get(2)?)?),
            units: ResourceUnits(r.get(3)?),
        };
        assignments.insert(a.id, a);
    }

    let calendar: Calendar = serde_json::from_str(&data)?;
    Ok(Project {
        id: ProjectId(uuid(id)?),
        name,
        start: Moment(start),
        calendar_id: CalendarId(uuid(calendar_id)?),
        calendar,
        tasks,
        dependencies,
        resources,
        assignments,
        currency,
        baseline: read_baseline(conn)?,
        status_date: status_date.map(Moment),
    })
}

fn lag_format_code(f: pm_core::LagFormat) -> &'static str {
    match f {
        pm_core::LagFormat::Working => "working",
        pm_core::LagFormat::Elapsed => "elapsed",
        pm_core::LagFormat::Percent => "percent",
    }
}

fn lag_format_of(code: &str) -> Result<pm_core::LagFormat, StoreError> {
    match code {
        "working" => Ok(pm_core::LagFormat::Working),
        "elapsed" => Ok(pm_core::LagFormat::Elapsed),
        "percent" => Ok(pm_core::LagFormat::Percent),
        other => Err(StoreError::Damaged(format!("lag format {other}"))),
    }
}

fn task_type_code(t: pm_core::TaskType) -> &'static str {
    match t {
        pm_core::TaskType::FixedUnits => "fixed_units",
        pm_core::TaskType::FixedWork => "fixed_work",
        pm_core::TaskType::FixedDuration => "fixed_duration",
    }
}

fn task_type_of(code: &str) -> Result<pm_core::TaskType, StoreError> {
    match code {
        "fixed_units" => Ok(pm_core::TaskType::FixedUnits),
        "fixed_work" => Ok(pm_core::TaskType::FixedWork),
        "fixed_duration" => Ok(pm_core::TaskType::FixedDuration),
        other => Err(StoreError::Damaged(format!("task type {other}"))),
    }
}
