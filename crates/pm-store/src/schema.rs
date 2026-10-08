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
];

pub const CURRENT_VERSION: i64 = MIGRATIONS.len() as i64;
