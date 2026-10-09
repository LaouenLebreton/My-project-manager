//! Schema and forward-only migrations (spec 4.7). `PRAGMA user_version` holds the version.

/// "PMPJ": marks a SQLite file as a Super PM project.
pub const APPLICATION_ID: i32 = 0x504D_504A;

/// Migration `k` takes the schema from version `k` to `k + 1`. Never edit a shipped one.
pub const MIGRATIONS: &[&str] = &[
    // 0 -> 1: first schema.
    r#"
    CREATE TABLE project (
        singleton    INTEGER PRIMARY KEY CHECK (singleton = 1),
        id           TEXT NOT NULL,
        name         TEXT NOT NULL,
        start_min    INTEGER NOT NULL,
        calendar_id  TEXT NOT NULL
    );
    -- Working week and exceptions as JSON: small, always read and written whole.
    CREATE TABLE calendar (
        id    TEXT PRIMARY KEY,
        name  TEXT NOT NULL,
        data  TEXT NOT NULL
    );
    CREATE TABLE task (
        id                TEXT PRIMARY KEY,
        parent_id         TEXT,
        sort_key          TEXT NOT NULL,
        name              TEXT NOT NULL,
        duration_min      INTEGER NOT NULL CHECK (duration_min >= 0),
        is_milestone      INTEGER NOT NULL CHECK (is_milestone IN (0, 1)),
        constraint_type   TEXT NOT NULL CHECK (constraint_type IN ('ASAP', 'SNET', 'MSO')),
        constraint_min    INTEGER,
        percent_complete  INTEGER NOT NULL CHECK (percent_complete BETWEEN 0 AND 100),
        notes             TEXT NOT NULL,
        CHECK ((constraint_type = 'ASAP') = (constraint_min IS NULL))
    );
    CREATE INDEX task_parent ON task (parent_id, sort_key);
    CREATE TABLE dependency (
        id              TEXT PRIMARY KEY,
        predecessor_id  TEXT NOT NULL,
        successor_id    TEXT NOT NULL,
        kind            TEXT NOT NULL CHECK (kind IN ('FS', 'SS', 'FF', 'SF')),
        lag_min         INTEGER NOT NULL
    );
    -- Append-only record of every applied command, including undo and redo.
    CREATE TABLE change_log (
        seq         INTEGER PRIMARY KEY AUTOINCREMENT,
        entry_id    TEXT NOT NULL UNIQUE,
        at          TEXT NOT NULL,
        origin      TEXT NOT NULL CHECK (origin IN ('edit', 'undo', 'redo')),
        target_seq  INTEGER REFERENCES change_log (seq),
        label       TEXT NOT NULL,
        payload     TEXT NOT NULL,
        inverse     TEXT NOT NULL
    );
    -- The undo stack: edits that can be undone (undone = 0) or redone (undone = 1).
    CREATE TABLE undo_history (
        pos      INTEGER PRIMARY KEY AUTOINCREMENT,
        log_seq  INTEGER NOT NULL REFERENCES change_log (seq),
        undone   INTEGER NOT NULL DEFAULT 0 CHECK (undone IN (0, 1))
    );
    -- Key/value settings, e.g. the future sync cursor (spec 4.10).
    CREATE TABLE meta (
        key    TEXT PRIMARY KEY,
        value  TEXT NOT NULL
    );
    "#,
    // 1 -> 2: resources and assignments (iteration 2).
    r#"
    CREATE TABLE resource (
        id              TEXT PRIMARY KEY,
        sort_key        TEXT NOT NULL,
        name            TEXT NOT NULL,
        initials        TEXT NOT NULL,
        kind            TEXT NOT NULL CHECK (kind IN ('work', 'material')),
        max_units       INTEGER NOT NULL CHECK (max_units >= 0),
        material_label  TEXT NOT NULL,
        notes           TEXT NOT NULL
    );
    CREATE TABLE assignment (
        id           TEXT PRIMARY KEY,
        task_id      TEXT NOT NULL,
        resource_id  TEXT NOT NULL,
        units        INTEGER NOT NULL CHECK (units > 0)
    );
    CREATE INDEX assignment_task ON assignment (task_id);
    CREATE INDEX assignment_resource ON assignment (resource_id);
    "#,
    // 2 -> 3: leveling delay per task (iteration 2).
    r#"
    ALTER TABLE task ADD COLUMN leveling_delay_min INTEGER NOT NULL DEFAULT 0 CHECK (leveling_delay_min >= 0);
    "#,
    // 3 -> 4: costs (iteration 2). Money in cents.
    r#"
    ALTER TABLE project ADD COLUMN currency TEXT NOT NULL DEFAULT '';
    ALTER TABLE task ADD COLUMN fixed_cost_cents INTEGER NOT NULL DEFAULT 0;
    ALTER TABLE resource ADD COLUMN rate_cents INTEGER NOT NULL DEFAULT 0 CHECK (rate_cents >= 0);
    "#,
    // 4 -> 5: every Microsoft Project constraint type. SQLite cannot change a CHECK in
    // place, so the task table is rebuilt and its rows copied.
    r#"
    CREATE TABLE task_v5 (
        id                  TEXT PRIMARY KEY,
        parent_id           TEXT,
        sort_key            TEXT NOT NULL,
        name                TEXT NOT NULL,
        duration_min        INTEGER NOT NULL CHECK (duration_min >= 0),
        is_milestone        INTEGER NOT NULL CHECK (is_milestone IN (0, 1)),
        constraint_type     TEXT NOT NULL CHECK (constraint_type IN ('ASAP', 'ALAP', 'SNET', 'SNLT', 'FNET', 'FNLT', 'MSO', 'MFO')),
        constraint_min      INTEGER,
        percent_complete    INTEGER NOT NULL CHECK (percent_complete BETWEEN 0 AND 100),
        notes               TEXT NOT NULL,
        leveling_delay_min  INTEGER NOT NULL DEFAULT 0 CHECK (leveling_delay_min >= 0),
        fixed_cost_cents    INTEGER NOT NULL DEFAULT 0,
        CHECK ((constraint_type IN ('ASAP', 'ALAP')) = (constraint_min IS NULL))
    );
    INSERT INTO task_v5 (id, parent_id, sort_key, name, duration_min, is_milestone, constraint_type, constraint_min,
                         percent_complete, notes, leveling_delay_min, fixed_cost_cents)
        SELECT id, parent_id, sort_key, name, duration_min, is_milestone, constraint_type, constraint_min,
               percent_complete, notes, leveling_delay_min, fixed_cost_cents FROM task;
    DROP TABLE task;
    ALTER TABLE task_v5 RENAME TO task;
    CREATE INDEX task_parent ON task (parent_id, sort_key);
    "#,
    // 5 -> 6: manually scheduled tasks (their start; NULL = automatic).
    r#"
    ALTER TABLE task ADD COLUMN manual_start_min INTEGER;
    "#,
    // 6 -> 7: the saved baseline.
    r#"
    ALTER TABLE project ADD COLUMN baseline_saved_at TEXT;
    CREATE TABLE baseline_task (
        task_id       TEXT PRIMARY KEY,
        start_min     INTEGER NOT NULL,
        finish_min    INTEGER NOT NULL,
        duration_min  INTEGER NOT NULL,
        work_min      INTEGER NOT NULL,
        cost_cents    INTEGER NOT NULL
    );
    "#,
    // 7 -> 8: progress tracking.
    r#"
    ALTER TABLE task ADD COLUMN actual_start_min INTEGER;
    ALTER TABLE task ADD COLUMN actual_finish_min INTEGER;
    ALTER TABLE project ADD COLUMN status_date_min INTEGER;
    "#,
];

pub const CURRENT_VERSION: i64 = MIGRATIONS.len() as i64;
