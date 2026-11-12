//! Project data model: user inputs only (spec 4.4). Computed dates live in the schedule.
//!
//! Ids are UUID v7 created by the caller. This crate never generates them, so it stays
//! free of clocks and randomness.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::calendar::Calendar;
use crate::time::Moment;

macro_rules! id_type {
    ($name:ident) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub Uuid);
    };
}

id_type!(ProjectId);
id_type!(TaskId);
id_type!(DependencyId);
id_type!(CalendarId);
id_type!(ResourceId);
id_type!(AssignmentId);

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ModelError {
    #[error("a duration cannot be negative ({0} minutes)")]
    NegativeDuration(i64),
}

/// Non-negative whole working minutes (R1).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "i64", into = "i64")]
pub struct Duration(i64);

impl Duration {
    pub const ZERO: Duration = Duration(0);

    pub fn minutes(m: i64) -> Result<Self, ModelError> {
        if m < 0 { Err(ModelError::NegativeDuration(m)) } else { Ok(Duration(m)) }
    }

    pub fn get(self) -> i64 {
        self.0
    }
}

impl TryFrom<i64> for Duration {
    type Error = ModelError;
    fn try_from(m: i64) -> Result<Self, ModelError> {
        Duration::minutes(m)
    }
}

impl From<Duration> for i64 {
    fn from(d: Duration) -> i64 {
        d.0
    }
}

/// Scheduling constraint, as in Microsoft Project. The date is part of the variant, so a
/// constraint can never lack its date (or have one it does not use).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "date")]
pub enum Constraint {
    #[default]
    AsSoonAsPossible,
    AsLateAsPossible,
    StartNoEarlierThan(Moment),
    StartNoLaterThan(Moment),
    FinishNoEarlierThan(Moment),
    FinishNoLaterThan(Moment),
    MustStartOn(Moment),
    MustFinishOn(Moment),
}

impl Constraint {
    pub fn date(self) -> Option<Moment> {
        match self {
            Constraint::AsSoonAsPossible | Constraint::AsLateAsPossible => None,
            Constraint::StartNoEarlierThan(d)
            | Constraint::StartNoLaterThan(d)
            | Constraint::FinishNoEarlierThan(d)
            | Constraint::FinishNoLaterThan(d)
            | Constraint::MustStartOn(d)
            | Constraint::MustFinishOn(d) => Some(d),
        }
    }

    /// Must start on and must finish on fix the task in time; leveling never moves it.
    pub fn is_fixed(self) -> bool {
        matches!(self, Constraint::MustStartOn(_) | Constraint::MustFinishOn(_))
    }
}

/// Position among siblings: a fractional index string, compared bytewise.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SortKey(pub String);

/// Percent complete, 0 to 100. Stored only; not used by scheduling in iteration 1.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "u8", into = "u8")]
pub struct Percent(u8);

impl Percent {
    pub fn new(p: u8) -> Option<Self> {
        (p <= 100).then_some(Percent(p))
    }

    pub fn get(self) -> u8 {
        self.0
    }
}

impl TryFrom<u8> for Percent {
    type Error = String;
    fn try_from(p: u8) -> Result<Self, String> {
        Percent::new(p).ok_or_else(|| format!("percent complete must be 0 to 100, got {p}"))
    }
}

impl From<Percent> for u8 {
    fn from(p: Percent) -> u8 {
        p.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Task {
    pub id: TaskId,
    pub parent: Option<TaskId>,
    pub sort_key: SortKey,
    pub name: String,
    /// Ignored for milestones, which always have zero duration (R14). Kept so that
    /// turning a milestone back into a task restores its duration.
    pub duration: Duration,
    pub is_milestone: bool,
    pub constraint: Constraint,
    pub percent_complete: Percent,
    pub notes: String,
    /// Working time leveling added before the task starts (Microsoft Project's "Leveling
    /// Delay"). Set by Level Resources, cleared by Clear Leveling.
    #[serde(default)]
    pub leveling_delay: Duration,
    /// A one-off cost on top of the resources' cost (a venue fee, a permit).
    #[serde(default)]
    pub fixed_cost: Money,
    /// Manually scheduled (R34): the task starts here whatever its links and constraints say.
    /// `None` means scheduled automatically.
    #[serde(default)]
    pub manual_start: Option<Moment>,
    /// When it should be finished by (Microsoft Project's Deadline): unlike a constraint it does
    /// not move the task; a later finish is flagged.
    #[serde(default)]
    pub deadline: Option<Moment>,
    /// The duration counts every minute, nights and weekends included (Microsoft Project's
    /// elapsed duration, "3ed"); otherwise only working time.
    #[serde(default)]
    pub elapsed: bool,
    /// Which of duration, units and work stays when another changes (R39).
    #[serde(default)]
    pub task_type: TaskType,
    /// Adding or removing people keeps the total work (R39).
    #[serde(default)]
    pub effort_driven: bool,
    /// What really happened (R35): once a task has started, its dates are facts.
    #[serde(default)]
    pub actual: Actual,
}

/// Actual start and finish. A finish without a start is not representable.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state")]
pub enum Actual {
    #[default]
    NotStarted,
    Started { start: Moment },
    Finished { start: Moment, finish: Moment },
}

impl Actual {
    pub fn start(self) -> Option<Moment> {
        match self {
            Actual::NotStarted => None,
            Actual::Started { start } | Actual::Finished { start, .. } => Some(start),
        }
    }

    pub fn finish(self) -> Option<Moment> {
        match self {
            Actual::Finished { finish, .. } => Some(finish),
            _ => None,
        }
    }
}

/// An amount in hundredths of the project's currency (cents), so totals never pick up
/// floating-point rounding.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Money(pub i64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum LinkType {
    FinishToStart,
    StartToStart,
    FinishToFinish,
    StartToFinish,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Dependency {
    pub id: DependencyId,
    pub predecessor: TaskId,
    pub successor: TaskId,
    pub kind: LinkType,
    /// Working minutes; negative lag overlaps the tasks (R13). With `lag_format` Elapsed,
    /// clock minutes; with Percent, a percentage of the predecessor's duration (50 = 50%).
    pub lag: i64,
    #[serde(default)]
    pub lag_format: LagFormat,
}

/// Microsoft Project's task types (R39): work = duration × units, and the type says which one
/// stays when another is changed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaskType {
    /// Units stay: a new duration changes the work; new units or work change the duration.
    #[default]
    FixedUnits,
    /// Work stays: a new duration changes the units; new units change the duration.
    FixedWork,
    /// Duration stays: new units change the work; new work changes the units.
    FixedDuration,
}

/// How a link's lag is measured, as in Microsoft Project (R37).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum LagFormat {
    /// Working time (the usual: "+2d").
    #[default]
    Working,
    /// Clock time, nights and weekends included ("+2ed").
    Elapsed,
    /// A share of the predecessor's duration ("+50%").
    Percent,
}

/// People and machines work for a share of their time; materials are used up by quantity.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ResourceKind {
    #[default]
    Work,
    Material,
}

/// Hundredths: 100 is 100% of a work resource's time, or 1.00 of a material.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ResourceUnits(pub u32);

impl ResourceUnits {
    pub const FULL_TIME: ResourceUnits = ResourceUnits(100);
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Resource {
    pub id: ResourceId,
    pub sort_key: SortKey,
    pub name: String,
    pub initials: String,
    pub kind: ResourceKind,
    /// Most a work resource can give at once: 100 = one full-time person, 300 = a team of
    /// three. Ignored for materials.
    pub max_units: ResourceUnits,
    /// What a material is counted in ("tons", "licences"). Empty for work resources.
    pub material_label: String,
    /// Cost per working hour (work resources) or per unit (materials).
    #[serde(default)]
    pub rate: Money,
    pub notes: String,
    /// When this person does not work, beyond the project calendar (R38).
    #[serde(default)]
    pub availability: Availability,
}

/// A person's own working time (R38): weekdays they do not work (part-time) and days off
/// (holidays, leave). Their working hours on the days they work are the project's.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Availability {
    /// Monday first: true where they do not work.
    #[serde(default)]
    pub off_weekdays: [bool; 7],
    /// Days off, as day numbers (days since 1970-01-01).
    #[serde(default)]
    pub days_off: std::collections::BTreeSet<i64>,
}

impl Availability {
    /// Works whenever the project does.
    pub fn is_full(&self) -> bool {
        !self.off_weekdays.contains(&true) && self.days_off.is_empty()
    }
}

/// A resource working on (or used by) a task.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Assignment {
    pub id: AssignmentId,
    pub task: TaskId,
    pub resource: ResourceId,
    /// Share of the resource's time for work resources (100 = full time), quantity for
    /// materials (250 = 2.5 tons).
    pub units: ResourceUnits,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Project {
    pub id: ProjectId,
    pub name: String,
    pub start: Moment,
    pub calendar_id: CalendarId,
    pub calendar: Calendar,
    /// Ordered maps keep every iteration deterministic (R29).
    pub tasks: BTreeMap<TaskId, Task>,
    pub dependencies: BTreeMap<DependencyId, Dependency>,
    /// The resource pool (iteration 2).
    #[serde(default)]
    pub resources: BTreeMap<ResourceId, Resource>,
    #[serde(default)]
    pub assignments: BTreeMap<AssignmentId, Assignment>,
    /// Currency symbol for costs ("€", "$", "CHF"); empty until the user sets one.
    #[serde(default)]
    pub currency: String,
    /// The plan as it was when the baseline was saved (Project > Set Baseline).
    #[serde(default)]
    pub baseline: Option<Baseline>,
    /// "As of" date for progress and earned value (Project Info).
    #[serde(default)]
    pub status_date: Option<Moment>,
}

/// A saved copy of the plan, to compare progress against.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Baseline {
    /// When it was saved, as given by the caller (RFC 3339); the core has no clock.
    pub saved_at: String,
    pub tasks: BTreeMap<TaskId, BaselineTask>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BaselineTask {
    pub start: Moment,
    pub finish: Moment,
    /// Working minutes.
    pub duration: i64,
    /// Working minutes of resource work.
    pub work: i64,
    pub cost: Money,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn negative_duration_is_unrepresentable() {
        assert_eq!(Duration::minutes(-1), Err(ModelError::NegativeDuration(-1)));
        assert!(serde_json::from_str::<Duration>("-5").is_err());
        assert_eq!(serde_json::from_str::<Duration>("480").unwrap().get(), 480);
    }

    #[test]
    fn percent_is_bounded() {
        assert!(Percent::new(101).is_none());
        assert!(serde_json::from_str::<Percent>("101").is_err());
    }

    #[test]
    fn constraint_serializes_with_its_date() {
        let c = Constraint::MustStartOn(Moment(1_000));
        let json = serde_json::to_string(&c).unwrap();
        assert_eq!(json, r#"{"type":"MustStartOn","date":1000}"#);
        assert_eq!(serde_json::from_str::<Constraint>(&json).unwrap(), c);
    }
}
