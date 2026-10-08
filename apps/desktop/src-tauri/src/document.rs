//! The open project: file, in-memory model and current schedule.

use std::path::{Path, PathBuf};

use pm_core::{schedule, Command, Moment, Project, Schedule, ScheduleError};
use pm_store::{Applied, EntryMeta, Store, StoreError};
use uuid::Uuid;

/// New ids are UUID v7: sortable and safe to create offline (spec 4.3).
pub fn new_id() -> Uuid {
    Uuid::now_v7()
}

fn entry_meta() -> EntryMeta {
    EntryMeta { id: new_id(), at: jiff::Timestamp::now().to_string() }
}

/// Monday of the current week at 08:00, local time.
pub fn this_monday_morning() -> Moment {
    let today = jiff::Zoned::now().date();
    let monday = today
        .checked_sub(jiff::Span::new().days(i64::from(today.weekday().to_monday_zero_offset())))
        .unwrap_or(today);
    Moment::from_datetime(monday.at(8, 0, 0, 0))
}

pub struct Document {
    pub path: PathBuf,
    store: Store,
    pub project: Project,
    pub schedule: Result<Schedule, ScheduleError>,
    /// Change-log position of the last backup, so unchanged projects are not copied again.
    pub backed_up_seq: Option<i64>,
}

impl Document {
    pub fn open(path: &Path) -> Result<Document, StoreError> {
        let (store, project) = Store::open(path)?;
        Ok(Document::new(path, store, project))
    }

    pub fn create(path: &Path, project: Project) -> Result<Document, StoreError> {
        let store = Store::create(path, &project)?;
        Ok(Document::new(path, store, project))
    }

    fn new(path: &Path, store: Store, project: Project) -> Document {
        let schedule = schedule(&project);
        Document { path: path.to_owned(), store, project, schedule, backed_up_seq: None }
    }

    pub fn apply(&mut self, cmd: &Command) -> Result<Applied, StoreError> {
        let applied = self.store.apply(&mut self.project, cmd, &entry_meta())?;
        self.reschedule();
        Ok(applied)
    }

    pub fn undo(&mut self) -> Result<Option<Applied>, StoreError> {
        let applied = self.store.undo(&mut self.project, &entry_meta())?;
        self.reschedule();
        Ok(applied)
    }

    pub fn redo(&mut self) -> Result<Option<Applied>, StoreError> {
        let applied = self.store.redo(&mut self.project, &entry_meta())?;
        self.reschedule();
        Ok(applied)
    }

    pub fn undo_label(&self) -> Option<String> {
        self.store.undo_label().ok().flatten()
    }

    pub fn redo_label(&self) -> Option<String> {
        self.store.redo_label().ok().flatten()
    }

    pub fn last_seq(&self) -> i64 {
        self.store.last_seq().unwrap_or(0)
    }

    pub fn save_copy(&self, path: &Path) -> Result<(), StoreError> {
        self.store.save_copy(path)
    }

    fn reschedule(&mut self) {
        self.schedule = schedule(&self.project);
    }
}
