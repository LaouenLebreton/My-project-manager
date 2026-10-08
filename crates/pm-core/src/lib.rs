//! Pure scheduling engine, data model and commands. No I/O, no clock, no randomness.
//!
//! Every instant is a [`Moment`]: whole minutes on a naive local timeline. Durations and
//! lags are whole working minutes. See `docs/scheduling-rules.md` for the rules (R1 to R29)
//! that the tests in this crate enforce.

pub mod calendar;
pub mod command;
pub mod model;
pub mod schedule;
pub mod time;

pub use calendar::{Calendar, CalendarError, DayPattern, WorkRange};
pub use command::{apply, Command, CommandError, LinkChange, ProjectChange, TaskChange, Touched};
pub use model::*;
pub use schedule::{schedule, ConstraintConflict, Schedule, ScheduleError, TaskSchedule};
pub use time::{Moment, MINUTES_PER_DAY};
