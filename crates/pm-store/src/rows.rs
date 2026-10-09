//! Mapping between model types and table rows.

use std::collections::BTreeMap;

use pm_core::{
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
        "INSERT INTO project (singleton, id, name, start_min, calendar_id) VALUES (1, ?1, ?2, ?3, ?4)
         ON CONFLICT (singleton) DO UPDATE SET id = ?1, name = ?2, start_min = ?3, calendar_id = ?4",
        params![p.id.0.to_string(), p.name, p.start.0, p.calendar_id.0.to_string()],
    )?;
    conn.execute(
        "INSERT INTO calendar (id, name, data) VALUES (?1, 'Project calendar', ?2)
         ON CONFLICT (id) DO UPDATE SET data = ?2",
        params![p.calendar_id.0.to_string(), serde_json::to_string(&p.calendar)?],
    )?;
    Ok(())
}

pub fn upsert_task(conn: &Connection, t: &Task) -> Result<(), StoreError> {
    let (ctype, cdate) = match t.constraint {
        Constraint::AsSoonAsPossible => ("ASAP", None),
        Constraint::StartNoEarlierThan(m) => ("SNET", Some(m.0)),
        Constraint::MustStartOn(m) => ("MSO", Some(m.0)),
    };
    conn.execute(
        "INSERT OR REPLACE INTO task
         (id, parent_id, sort_key, name, duration_min, is_milestone, constraint_type, constraint_min, percent_complete, notes)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
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
        "INSERT OR REPLACE INTO dependency (id, predecessor_id, successor_id, kind, lag_min) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![d.id.0.to_string(), d.predecessor.0.to_string(), d.successor.0.to_string(), link_code(d.kind), d.lag],
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
        "INSERT OR REPLACE INTO resource (id, sort_key, name, initials, kind, max_units, material_label, notes)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![r.id.0.to_string(), r.sort_key.0, r.name, r.initials, kind, r.max_units.0, r.material_label, r.notes],
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
        ("SNET", Some(m)) => Constraint::StartNoEarlierThan(Moment(m)),
        ("MSO", Some(m)) => Constraint::MustStartOn(Moment(m)),
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
        notes: r.get(9)?,
    })
}

pub fn read_project(conn: &Connection) -> Result<Project, StoreError> {
    let (id, name, start, calendar_id): (String, String, i64, String) = conn
        .query_row("SELECT id, name, start_min, calendar_id FROM project WHERE singleton = 1", [], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
        })
        .map_err(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => StoreError::Damaged("the project record is missing".into()),
            e => e.into(),
        })?;
    let data: String = conn.query_row("SELECT data FROM calendar WHERE id = ?1", [&calendar_id], |r| r.get(0))?;

    let mut tasks = BTreeMap::new();
    let mut stmt = conn.prepare(
        "SELECT id, parent_id, sort_key, name, duration_min, is_milestone, constraint_type, constraint_min, percent_complete, notes FROM task",
    )?;
    let mut q = stmt.query([])?;
    while let Some(r) = q.next()? {
        let t = task_from_row(r)?;
        tasks.insert(t.id, t);
    }

    let mut dependencies = BTreeMap::new();
    let mut stmt = conn.prepare("SELECT id, predecessor_id, successor_id, kind, lag_min FROM dependency")?;
    let mut q = stmt.query([])?;
    while let Some(r) = q.next()? {
        let d = Dependency {
            id: DependencyId(uuid(r.get(0)?)?),
            predecessor: TaskId(uuid(r.get(1)?)?),
            successor: TaskId(uuid(r.get(2)?)?),
            kind: link_type(&r.get::<_, String>(3)?)?,
            lag: r.get(4)?,
        };
        dependencies.insert(d.id, d);
    }

    let mut resources = BTreeMap::new();
    let mut stmt = conn.prepare("SELECT id, sort_key, name, initials, kind, max_units, material_label, notes FROM resource")?;
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
            notes: r.get(7)?,
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
    })
}
