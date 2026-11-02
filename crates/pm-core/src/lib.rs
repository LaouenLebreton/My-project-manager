//! Pure scheduling engine, data model and commands. No I/O, no clock, no randomness.
//!
//! Every instant is a [`Moment`]: whole minutes on a naive local timeline. Durations and
//! lags are whole working minutes. See `docs/scheduling-rules.md` for the rules (R1 to R29)
//! that the tests in this crate enforce.

pub mod calendar;
pub mod command;
pub mod earned;
pub mod leveling;
pub mod model;
pub mod outline;
pub mod portfolio;
pub mod resources;
pub mod schedule;
pub mod text;
pub mod time;

pub use calendar::{Calendar, CalendarError, DayPattern, WorkRange};
pub use command::{apply, Command, CommandError, LinkChange, ProjectChange, ResourceChange, TaskChange, Touched};
pub use model::*;
pub use outline::{outline, OutlineRow};
pub use text::{format_resource, parse_money, parse_resource_list, PredecessorEntry, ResourceEntry, TextError, Units};
pub use earned::{earned_value, EarnedValue, Ev};
pub use leveling::{level, Leveled, Unresolved};
pub use resources::{assignment_days, capacity, capture_baseline, loads, Loads, Overload, ResourceLoad};
pub use schedule::{check, schedule, ConstraintConflict, Schedule, ScheduleError, TaskSchedule};
pub use time::{Moment, MINUTES_PER_DAY};
