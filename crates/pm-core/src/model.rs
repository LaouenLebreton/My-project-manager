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

/// Scheduling constraint. The date is part of the variant, so "as soon as possible with a
/// date" cannot be represented.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "date")]
pub enum Constraint {
    #[default]
    AsSoonAsPossible,
    StartNoEarlierThan(Moment),
    MustStartOn(Moment),
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
}

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
    /// Working minutes; negative lag overlaps the tasks (R13).
    pub lag: i64,
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
    pub notes: String,
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
