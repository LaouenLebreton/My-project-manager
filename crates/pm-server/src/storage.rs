//! Where the server keeps its data. A trait, so a team server can use one SQLite file and a
//! cloud deployment a managed Postgres (iteration 3, step 7) without touching the handlers.

use std::sync::Mutex;

use rusqlite::{params, Connection, OptionalExtension};

use crate::access::{Limits, Role};
use pm_sync::Change;
use serde::Serialize;
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("there is already an account for {0}")]
    EmailTaken(String),
    #[error("this project is already on the server")]
    ProjectExists,
    #[error("stored data is damaged: {0}")]
    Damaged(String),
    #[error("database: {0}")]
    Database(#[from] rusqlite::Error),
}

pub type Result<T> = std::result::Result<T, StorageError>;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct User {
    pub id: Uuid,
    pub name: String,
    /// Lower-case, trimmed.
    pub email: String,
    /// Server administrators create accounts.
    pub is_admin: bool,
    /// RFC 3339.
    pub created_at: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Team {
    pub id: Uuid,
    pub name: String,
    pub created_at: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TeamRole {
    /// Adds and removes members, creates channels.
    Owner,
    Member,
}

/// A shared project, without its contents.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectRecord {
    pub id: Uuid,
    pub team_id: Uuid,
    pub name: String,
    pub owner_id: Uuid,
    /// Number of changes applied since it was shared.
    pub version: i64,
    pub created_at: String,
    pub updated_at: String,
}

/// A chat channel in a team (General, a team project's channel, or one people made).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Channel {
    pub id: Uuid,
    pub team_id: Uuid,
    pub name: String,
    /// Set for the channel that belongs to a team project.
    pub project_id: Option<Uuid>,
    pub created_at: String,
    /// Posts by others the caller has not seen (filled in per request).
    pub unread: u32,
}

/// A post in a channel, or a reply to one (`parent_id`). Ids are UUID v7, so they sort by time.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Post {
    pub id: Uuid,
    pub channel_id: Uuid,
    pub parent_id: Option<Uuid>,
    pub author_id: Uuid,
    pub author_name: String,
    pub at: String,
    pub body: String,
}

/// Everything the handlers need from storage. Calls are blocking; handlers run them on
/// Tokio's blocking pool.
pub trait Storage: Send + Sync + 'static {
    fn user_count(&self) -> Result<u64>;
    /// Creates the account only if there are none yet, atomically; false if there were.
    fn create_first_user(&self, user: &User, password_hash: &str) -> Result<bool>;
    /// Fails with `EmailTaken` if the email is in use.
    fn create_user(&self, user: &User, password_hash: &str) -> Result<()>;
    /// The user and their stored password hash.
    fn user_by_email(&self, email: &str) -> Result<Option<(User, String)>>;
    fn create_session(&self, token_hash: &str, user: Uuid, created_at: &str) -> Result<()>;
    fn session_user(&self, token_hash: &str) -> Result<Option<User>>;
    fn delete_session(&self, token_hash: &str) -> Result<()>;
    fn user(&self, id: Uuid) -> Result<Option<User>>;

    /// Creates a team with its first owner.
    fn create_team(&self, team: &Team, owner: Uuid) -> Result<()>;
    fn teams_of(&self, user: Uuid) -> Result<Vec<(Team, TeamRole)>>;
    fn team(&self, id: Uuid) -> Result<Option<Team>>;
    fn team_role(&self, team: Uuid, user: Uuid) -> Result<Option<TeamRole>>;
    /// Adds or changes a member.
    fn set_team_member(&self, team: Uuid, user: Uuid, role: TeamRole) -> Result<()>;
    fn team_members(&self, team: Uuid) -> Result<Vec<(User, TeamRole)>>;

    /// Stores a newly shared project (`project` is `pm_core::Project` as JSON). Its owner gets
    /// the Owner role. Fails with `ProjectExists` if the id is taken.
    fn create_project(&self, record: &ProjectRecord, project: &str) -> Result<()>;
    /// Projects of every team the user belongs to.
    fn projects_of(&self, user: Uuid) -> Result<Vec<ProjectRecord>>;
    /// The record and the contents as JSON.
    fn project(&self, id: Uuid) -> Result<Option<(ProjectRecord, String)>>;
    fn project_record(&self, id: Uuid) -> Result<Option<ProjectRecord>>;
    /// A member's role and limits when the owner has set them.
    fn project_member(&self, project: Uuid, user: Uuid) -> Result<Option<(Role, Limits)>>;
    fn set_project_member(&self, project: Uuid, user: Uuid, role: Role, limits: &Limits) -> Result<()>;

    /// The last saved copy of the project (JSON) and the version it is at. Changes after that
    /// version are replayed on top.
    fn project_snapshot(&self, id: Uuid) -> Result<Option<(String, i64)>>;
    /// Records change `change.version`, which must follow the stored version directly; false
    /// if another server instance got there first. `snapshot` also saves the whole project.
    fn append_change(&self, project: Uuid, change: &Change, inverse: &str, snapshot: Option<&str>) -> Result<bool>;
    /// The newest changes before version `before` (all if `None`), newest first.
    fn changes_before(&self, project: Uuid, before: Option<i64>, limit: u32) -> Result<Vec<Change>>;
    /// A change and the command that undoes it (JSON; empty for changes made before the
    /// history recorded it).
    fn change_with_inverse(&self, project: Uuid, version: i64) -> Result<Option<(Change, String)>>;
    /// Changes after `since`, oldest first, at most `limit`.
    fn changes_since(&self, project: Uuid, since: i64, limit: u32) -> Result<Vec<Change>>;
    /// The change made from this client edit, if it was accepted (resends are idempotent).
    fn change_by_client(&self, project: Uuid, client_change: Uuid) -> Result<Option<Change>>;

    /// A team's channels (unread counts left at 0).
    fn channels(&self, team: Uuid) -> Result<Vec<Channel>>;
    fn channel(&self, id: Uuid) -> Result<Option<Channel>>;
    fn create_channel(&self, channel: &Channel) -> Result<()>;
    fn add_post(&self, post: &Post) -> Result<()>;
    fn post(&self, id: Uuid) -> Result<Option<Post>>;
    /// Posts (not replies) before `before`, newest first.
    fn posts(&self, channel: Uuid, before: Option<Uuid>, limit: u32) -> Result<Vec<Post>>;
    /// Replies to these posts, oldest first.
    fn replies(&self, parents: &[Uuid]) -> Result<Vec<Post>>;
    /// The user has seen the channel up to this id.
    fn mark_read(&self, user: Uuid, channel: Uuid, upto: Uuid) -> Result<()>;
    /// Per channel of the team: posts and replies by others the user has not seen.
    fn unread(&self, user: Uuid, team: Uuid) -> Result<Vec<(Uuid, u32)>>;
}

/// Forward-only migrations; `PRAGMA user_version` holds the version.
const MIGRATIONS: &[&str] = &[
    // 0 -> 1: accounts and sessions.
    r#"
    CREATE TABLE user (
        id            TEXT PRIMARY KEY,
        name          TEXT NOT NULL,
        email         TEXT NOT NULL UNIQUE,
        password_hash TEXT NOT NULL,
        is_admin      INTEGER NOT NULL,
        created_at    TEXT NOT NULL
    );
    CREATE TABLE session (
        token_hash TEXT PRIMARY KEY,
        user_id    TEXT NOT NULL REFERENCES user (id) ON DELETE CASCADE,
        created_at TEXT NOT NULL
    );
    "#,
    // 1 -> 2: teams and shared projects. `change` is filled by live sync (step 3).
    r#"
    CREATE TABLE team (
        id         TEXT PRIMARY KEY,
        name       TEXT NOT NULL,
        created_at TEXT NOT NULL
    );
    CREATE TABLE team_member (
        team_id TEXT NOT NULL REFERENCES team (id) ON DELETE CASCADE,
        user_id TEXT NOT NULL REFERENCES user (id) ON DELETE CASCADE,
        role    TEXT NOT NULL CHECK (role IN ('owner', 'member')),
        PRIMARY KEY (team_id, user_id)
    );
    CREATE TABLE project (
        id         TEXT PRIMARY KEY,
        team_id    TEXT NOT NULL REFERENCES team (id),
        name       TEXT NOT NULL,
        owner_id   TEXT NOT NULL REFERENCES user (id),
        version    INTEGER NOT NULL,
        data       TEXT NOT NULL,
        created_at TEXT NOT NULL,
        updated_at TEXT NOT NULL
    );
    CREATE TABLE project_member (
        project_id TEXT NOT NULL REFERENCES project (id) ON DELETE CASCADE,
        user_id    TEXT NOT NULL REFERENCES user (id) ON DELETE CASCADE,
        role       TEXT NOT NULL CHECK (role IN ('owner', 'editor', 'viewer')),
        limits     TEXT NOT NULL,
        PRIMARY KEY (project_id, user_id)
    );
    CREATE TABLE change (
        project_id TEXT NOT NULL REFERENCES project (id) ON DELETE CASCADE,
        version    INTEGER NOT NULL,
        author_id  TEXT NOT NULL REFERENCES user (id),
        at         TEXT NOT NULL,
        label      TEXT NOT NULL,
        command    TEXT NOT NULL,
        PRIMARY KEY (project_id, version)
    );
    "#,
    // 2 -> 3: live sync. Changes remember the client edit they came from; projects remember
    // which version their saved copy is at.
    r#"
    ALTER TABLE change ADD COLUMN client_change TEXT NOT NULL DEFAULT '';
    CREATE UNIQUE INDEX change_client ON change (project_id, client_change);
    ALTER TABLE project ADD COLUMN data_version INTEGER NOT NULL DEFAULT 0;
    "#,
    // 3 -> 4: the History view. Each change keeps its sentence and the command that undoes it,
    // so the owner can revert it.
    r#"
    ALTER TABLE change ADD COLUMN summary TEXT NOT NULL DEFAULT '';
    ALTER TABLE change ADD COLUMN inverse TEXT NOT NULL DEFAULT '';
    "#,
    // 4 -> 5: chat. Ids are UUID v7 text, which sorts in time order.
    r#"
    CREATE TABLE channel (
        id         TEXT PRIMARY KEY,
        team_id    TEXT NOT NULL REFERENCES team (id) ON DELETE CASCADE,
        name       TEXT NOT NULL,
        project_id TEXT,
        created_at TEXT NOT NULL
    );
    CREATE UNIQUE INDEX channel_name ON channel (team_id, name COLLATE NOCASE);
    CREATE TABLE post (
        id         TEXT PRIMARY KEY,
        channel_id TEXT NOT NULL REFERENCES channel (id) ON DELETE CASCADE,
        parent_id  TEXT REFERENCES post (id) ON DELETE CASCADE,
        author_id  TEXT NOT NULL REFERENCES user (id),
        at         TEXT NOT NULL,
        body       TEXT NOT NULL
    );
    CREATE INDEX post_channel ON post (channel_id, parent_id, id);
    CREATE INDEX post_parent ON post (parent_id, id);
    CREATE TABLE channel_read (
        user_id    TEXT NOT NULL REFERENCES user (id) ON DELETE CASCADE,
        channel_id TEXT NOT NULL REFERENCES channel (id) ON DELETE CASCADE,
        upto       TEXT NOT NULL,
        PRIMARY KEY (user_id, channel_id)
    );
    "#,
];

/// A team server's storage: one SQLite file.
pub struct SqliteStorage {
    conn: Mutex<Connection>,
}

impl SqliteStorage {
    pub fn open(path: &std::path::Path) -> Result<Self> {
        Self::init(Connection::open(path)?)
    }

    /// For tests.
    pub fn in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self> {
        conn.execute_batch("PRAGMA foreign_keys = ON; PRAGMA journal_mode = WAL; PRAGMA busy_timeout = 5000;")?;
        let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
        for (k, sql) in MIGRATIONS.iter().enumerate().skip(version.max(0) as usize) {
            conn.execute_batch(&format!("BEGIN; {sql} PRAGMA user_version = {}; COMMIT;", k + 1))?;
        }
        Ok(SqliteStorage { conn: Mutex::new(conn) })
    }

    fn conn(&self) -> std::sync::MutexGuard<'_, Connection> {
        // A panic while holding the lock leaves the connection usable; carry on.
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }
}

fn user_from_row(r: &rusqlite::Row) -> rusqlite::Result<User> {
    let id: String = r.get(0)?;
    Ok(User {
        id: Uuid::parse_str(&id).map_err(|e| rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e)))?,
        name: r.get(1)?,
        email: r.get(2)?,
        is_admin: r.get(3)?,
        created_at: r.get(4)?,
    })
}

fn uuid_at(r: &rusqlite::Row, i: usize) -> rusqlite::Result<Uuid> {
    let s: String = r.get(i)?;
    Uuid::parse_str(&s).map_err(|e| rusqlite::Error::FromSqlConversionFailure(i, rusqlite::types::Type::Text, Box::new(e)))
}

fn team_role_code(r: TeamRole) -> &'static str {
    match r {
        TeamRole::Owner => "owner",
        TeamRole::Member => "member",
    }
}

fn team_role_at(r: &rusqlite::Row, i: usize) -> rusqlite::Result<TeamRole> {
    let s: String = r.get(i)?;
    Ok(if s == "owner" { TeamRole::Owner } else { TeamRole::Member })
}

const PROJECT_COLUMNS: &str = "p.id, p.team_id, p.name, p.owner_id, p.version, p.created_at, p.updated_at";

fn project_from_row(r: &rusqlite::Row) -> rusqlite::Result<ProjectRecord> {
    Ok(ProjectRecord {
        id: uuid_at(r, 0)?,
        team_id: uuid_at(r, 1)?,
        name: r.get(2)?,
        owner_id: uuid_at(r, 3)?,
        version: r.get(4)?,
        created_at: r.get(5)?,
        updated_at: r.get(6)?,
    })
}

/// A stored change; the inner error is a damaged command.
fn change_from_row(r: &rusqlite::Row) -> rusqlite::Result<std::result::Result<Change, String>> {
    let command: String = r.get(6)?;
    let version = r.get(0)?;
    let client_change = uuid_at(r, 1)?;
    let author = uuid_at(r, 2)?;
    let (author_name, at, label, summary) = (r.get(3)?, r.get(4)?, r.get(5)?, r.get(7)?);
    Ok(serde_json::from_str(&command)
        .map(|command| Change { version, client_change, author, author_name, at, label, summary, command })
        .map_err(|e| format!("change {version}: {e}")))
}

fn opt_uuid_at(r: &rusqlite::Row, i: usize) -> rusqlite::Result<Option<Uuid>> {
    let s: Option<String> = r.get(i)?;
    s.map(|s| Uuid::parse_str(&s).map_err(|e| rusqlite::Error::FromSqlConversionFailure(i, rusqlite::types::Type::Text, Box::new(e)))).transpose()
}

fn channel_from_row(r: &rusqlite::Row) -> rusqlite::Result<Channel> {
    Ok(Channel { id: uuid_at(r, 0)?, team_id: uuid_at(r, 1)?, name: r.get(2)?, project_id: opt_uuid_at(r, 3)?, created_at: r.get(4)?, unread: 0 })
}

const POST_COLUMNS: &str = "p.id, p.channel_id, p.parent_id, p.author_id, u.name, p.at, p.body";

fn post_from_row(r: &rusqlite::Row) -> rusqlite::Result<Post> {
    Ok(Post {
        id: uuid_at(r, 0)?,
        channel_id: uuid_at(r, 1)?,
        parent_id: opt_uuid_at(r, 2)?,
        author_id: uuid_at(r, 3)?,
        author_name: r.get(4)?,
        at: r.get(5)?,
        body: r.get(6)?,
    })
}

impl Storage for SqliteStorage {
    fn user_count(&self) -> Result<u64> {
        let n: i64 = self.conn().query_row("SELECT COUNT(*) FROM user", [], |r| r.get(0))?;
        Ok(n as u64)
    }

    fn create_first_user(&self, u: &User, password_hash: &str) -> Result<bool> {
        let n = self.conn().execute(
            "INSERT INTO user (id, name, email, password_hash, is_admin, created_at)
             SELECT ?1, ?2, ?3, ?4, ?5, ?6 WHERE NOT EXISTS (SELECT 1 FROM user)",
            params![u.id.to_string(), u.name, u.email, password_hash, u.is_admin, u.created_at],
        )?;
        Ok(n == 1)
    }

    fn create_user(&self, u: &User, password_hash: &str) -> Result<()> {
        let result = self.conn().execute(
            "INSERT INTO user (id, name, email, password_hash, is_admin, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![u.id.to_string(), u.name, u.email, password_hash, u.is_admin, u.created_at],
        );
        match result {
            Err(rusqlite::Error::SqliteFailure(e, _)) if e.code == rusqlite::ErrorCode::ConstraintViolation => Err(StorageError::EmailTaken(u.email.clone())),
            other => other.map(|_| ()).map_err(Into::into),
        }
    }

    fn user_by_email(&self, email: &str) -> Result<Option<(User, String)>> {
        Ok(self
            .conn()
            .query_row("SELECT id, name, email, is_admin, created_at, password_hash FROM user WHERE email = ?1", [email], |r| Ok((user_from_row(r)?, r.get(5)?)))
            .optional()?)
    }

    fn create_session(&self, token_hash: &str, user: Uuid, created_at: &str) -> Result<()> {
        self.conn().execute("INSERT INTO session (token_hash, user_id, created_at) VALUES (?1, ?2, ?3)", params![token_hash, user.to_string(), created_at])?;
        Ok(())
    }

    fn session_user(&self, token_hash: &str) -> Result<Option<User>> {
        Ok(self
            .conn()
            .query_row(
                "SELECT u.id, u.name, u.email, u.is_admin, u.created_at FROM session s JOIN user u ON u.id = s.user_id WHERE s.token_hash = ?1",
                [token_hash],
                user_from_row,
            )
            .optional()?)
    }

    fn delete_session(&self, token_hash: &str) -> Result<()> {
        self.conn().execute("DELETE FROM session WHERE token_hash = ?1", [token_hash])?;
        Ok(())
    }

    fn user(&self, id: Uuid) -> Result<Option<User>> {
        Ok(self.conn().query_row("SELECT id, name, email, is_admin, created_at FROM user WHERE id = ?1", [id.to_string()], user_from_row).optional()?)
    }

    fn create_team(&self, team: &Team, owner: Uuid) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        tx.execute("INSERT INTO team (id, name, created_at) VALUES (?1, ?2, ?3)", params![team.id.to_string(), team.name, team.created_at])?;
        tx.execute("INSERT INTO team_member (team_id, user_id, role) VALUES (?1, ?2, 'owner')", params![team.id.to_string(), owner.to_string()])?;
        tx.commit()?;
        Ok(())
    }

    fn teams_of(&self, user: Uuid) -> Result<Vec<(Team, TeamRole)>> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT t.id, t.name, t.created_at, m.role FROM team t JOIN team_member m ON m.team_id = t.id WHERE m.user_id = ?1 ORDER BY t.name COLLATE NOCASE")?;
        let rows = stmt.query_map([user.to_string()], |r| Ok((Team { id: uuid_at(r, 0)?, name: r.get(1)?, created_at: r.get(2)? }, team_role_at(r, 3)?)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    fn team(&self, id: Uuid) -> Result<Option<Team>> {
        Ok(self
            .conn()
            .query_row("SELECT id, name, created_at FROM team WHERE id = ?1", [id.to_string()], |r| Ok(Team { id: uuid_at(r, 0)?, name: r.get(1)?, created_at: r.get(2)? }))
            .optional()?)
    }

    fn team_role(&self, team: Uuid, user: Uuid) -> Result<Option<TeamRole>> {
        Ok(self
            .conn()
            .query_row("SELECT role FROM team_member WHERE team_id = ?1 AND user_id = ?2", [team.to_string(), user.to_string()], |r| team_role_at(r, 0))
            .optional()?)
    }

    fn set_team_member(&self, team: Uuid, user: Uuid, role: TeamRole) -> Result<()> {
        self.conn().execute(
            "INSERT INTO team_member (team_id, user_id, role) VALUES (?1, ?2, ?3) ON CONFLICT (team_id, user_id) DO UPDATE SET role = ?3",
            params![team.to_string(), user.to_string(), team_role_code(role)],
        )?;
        Ok(())
    }

    fn team_members(&self, team: Uuid) -> Result<Vec<(User, TeamRole)>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT u.id, u.name, u.email, u.is_admin, u.created_at, m.role FROM team_member m JOIN user u ON u.id = m.user_id WHERE m.team_id = ?1 ORDER BY u.name COLLATE NOCASE",
        )?;
        let rows = stmt.query_map([team.to_string()], |r| Ok((user_from_row(r)?, team_role_at(r, 5)?)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    fn create_project(&self, p: &ProjectRecord, data: &str) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let result = tx.execute(
            "INSERT INTO project (id, team_id, name, owner_id, version, data, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![p.id.to_string(), p.team_id.to_string(), p.name, p.owner_id.to_string(), p.version, data, p.created_at, p.updated_at],
        );
        if let Err(rusqlite::Error::SqliteFailure(e, _)) = &result
            && e.code == rusqlite::ErrorCode::ConstraintViolation
        {
            return Err(StorageError::ProjectExists);
        }
        result?;
        let limits = serde_json::to_string(&Limits::default()).expect("limits serialize");
        tx.execute(
            "INSERT INTO project_member (project_id, user_id, role, limits) VALUES (?1, ?2, 'owner', ?3)",
            params![p.id.to_string(), p.owner_id.to_string(), limits],
        )?;
        tx.commit()?;
        Ok(())
    }

    fn projects_of(&self, user: Uuid) -> Result<Vec<ProjectRecord>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT {PROJECT_COLUMNS} FROM project p JOIN team_member m ON m.team_id = p.team_id WHERE m.user_id = ?1 ORDER BY p.name COLLATE NOCASE"
        ))?;
        let rows = stmt.query_map([user.to_string()], project_from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    fn project(&self, id: Uuid) -> Result<Option<(ProjectRecord, String)>> {
        Ok(self
            .conn()
            .query_row(&format!("SELECT {PROJECT_COLUMNS}, p.data FROM project p WHERE p.id = ?1"), [id.to_string()], |r| Ok((project_from_row(r)?, r.get(7)?)))
            .optional()?)
    }

    fn project_record(&self, id: Uuid) -> Result<Option<ProjectRecord>> {
        Ok(self.conn().query_row(&format!("SELECT {PROJECT_COLUMNS} FROM project p WHERE p.id = ?1"), [id.to_string()], project_from_row).optional()?)
    }

    fn project_member(&self, project: Uuid, user: Uuid) -> Result<Option<(Role, Limits)>> {
        let row: Option<(String, String)> = self
            .conn()
            .query_row("SELECT role, limits FROM project_member WHERE project_id = ?1 AND user_id = ?2", [project.to_string(), user.to_string()], |r| Ok((r.get(0)?, r.get(1)?)))
            .optional()?;
        row.map(|(role, limits)| {
            let role = Role::from_code(&role).ok_or_else(|| StorageError::Damaged(format!("role {role}")))?;
            let limits = serde_json::from_str(&limits).map_err(|e| StorageError::Damaged(e.to_string()))?;
            Ok((role, limits))
        })
        .transpose()
    }

    fn set_project_member(&self, project: Uuid, user: Uuid, role: Role, limits: &Limits) -> Result<()> {
        let limits = serde_json::to_string(limits).expect("limits serialize");
        self.conn().execute(
            "INSERT INTO project_member (project_id, user_id, role, limits) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT (project_id, user_id) DO UPDATE SET role = ?3, limits = ?4",
            params![project.to_string(), user.to_string(), role.code(), limits],
        )?;
        Ok(())
    }

    fn project_snapshot(&self, id: Uuid) -> Result<Option<(String, i64)>> {
        Ok(self.conn().query_row("SELECT data, data_version FROM project WHERE id = ?1", [id.to_string()], |r| Ok((r.get(0)?, r.get(1)?))).optional()?)
    }

    fn append_change(&self, project: Uuid, c: &Change, inverse: &str, snapshot: Option<&str>) -> Result<bool> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let updated = match snapshot {
            Some(data) => tx.execute(
                "UPDATE project SET version = ?1, updated_at = ?2, data = ?3, data_version = ?1 WHERE id = ?4 AND version = ?5",
                params![c.version, c.at, data, project.to_string(), c.version - 1],
            )?,
            None => tx.execute(
                "UPDATE project SET version = ?1, updated_at = ?2 WHERE id = ?3 AND version = ?4",
                params![c.version, c.at, project.to_string(), c.version - 1],
            )?,
        };
        if updated == 0 {
            return Ok(false);
        }
        let command = serde_json::to_string(&c.command).map_err(|e| StorageError::Damaged(e.to_string()))?;
        tx.execute(
            "INSERT INTO change (project_id, version, author_id, at, label, command, client_change, summary, inverse) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![project.to_string(), c.version, c.author.to_string(), c.at, c.label, command, c.client_change.to_string(), c.summary, inverse],
        )?;
        tx.commit()?;
        Ok(true)
    }

    fn changes_since(&self, project: Uuid, since: i64, limit: u32) -> Result<Vec<Change>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT c.version, c.client_change, c.author_id, u.name, c.at, c.label, c.command, c.summary FROM change c JOIN user u ON u.id = c.author_id
             WHERE c.project_id = ?1 AND c.version > ?2 ORDER BY c.version LIMIT ?3",
        )?;
        let rows = stmt.query_map(params![project.to_string(), since, limit], change_from_row)?;
        rows.map(|r| r?.map_err(StorageError::Damaged)).collect()
    }

    fn changes_before(&self, project: Uuid, before: Option<i64>, limit: u32) -> Result<Vec<Change>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT c.version, c.client_change, c.author_id, u.name, c.at, c.label, c.command, c.summary FROM change c JOIN user u ON u.id = c.author_id
             WHERE c.project_id = ?1 AND c.version < ?2 ORDER BY c.version DESC LIMIT ?3",
        )?;
        let rows = stmt.query_map(params![project.to_string(), before.unwrap_or(i64::MAX), limit], change_from_row)?;
        rows.map(|r| r?.map_err(StorageError::Damaged)).collect()
    }

    fn change_with_inverse(&self, project: Uuid, version: i64) -> Result<Option<(Change, String)>> {
        let found = self
            .conn()
            .query_row(
                "SELECT c.version, c.client_change, c.author_id, u.name, c.at, c.label, c.command, c.summary, c.inverse FROM change c JOIN user u ON u.id = c.author_id
                 WHERE c.project_id = ?1 AND c.version = ?2",
                params![project.to_string(), version],
                |r| Ok((change_from_row(r)?, r.get::<_, String>(8)?)),
            )
            .optional()?;
        found.map(|(c, inv)| c.map(|c| (c, inv)).map_err(StorageError::Damaged)).transpose()
    }

    fn change_by_client(&self, project: Uuid, client_change: Uuid) -> Result<Option<Change>> {
        let found = self
            .conn()
            .query_row(
                "SELECT c.version, c.client_change, c.author_id, u.name, c.at, c.label, c.command, c.summary FROM change c JOIN user u ON u.id = c.author_id
                 WHERE c.project_id = ?1 AND c.client_change = ?2",
                [project.to_string(), client_change.to_string()],
                change_from_row,
            )
            .optional()?;
        found.transpose().map_err(StorageError::Damaged)
    }

    fn channels(&self, team: Uuid) -> Result<Vec<Channel>> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT id, team_id, name, project_id, created_at FROM channel WHERE team_id = ?1")?;
        let rows = stmt.query_map([team.to_string()], channel_from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    fn channel(&self, id: Uuid) -> Result<Option<Channel>> {
        Ok(self.conn().query_row("SELECT id, team_id, name, project_id, created_at FROM channel WHERE id = ?1", [id.to_string()], channel_from_row).optional()?)
    }

    fn create_channel(&self, c: &Channel) -> Result<()> {
        self.conn().execute(
            "INSERT INTO channel (id, team_id, name, project_id, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![c.id.to_string(), c.team_id.to_string(), c.name, c.project_id.map(|p| p.to_string()), c.created_at],
        )?;
        Ok(())
    }

    fn add_post(&self, p: &Post) -> Result<()> {
        self.conn().execute(
            "INSERT INTO post (id, channel_id, parent_id, author_id, at, body) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![p.id.to_string(), p.channel_id.to_string(), p.parent_id.map(|x| x.to_string()), p.author_id.to_string(), p.at, p.body],
        )?;
        Ok(())
    }

    fn post(&self, id: Uuid) -> Result<Option<Post>> {
        Ok(self
            .conn()
            .query_row(&format!("SELECT {POST_COLUMNS} FROM post p JOIN user u ON u.id = p.author_id WHERE p.id = ?1"), [id.to_string()], post_from_row)
            .optional()?)
    }

    fn posts(&self, channel: Uuid, before: Option<Uuid>, limit: u32) -> Result<Vec<Post>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT {POST_COLUMNS} FROM post p JOIN user u ON u.id = p.author_id
             WHERE p.channel_id = ?1 AND p.parent_id IS NULL AND p.id < ?2 ORDER BY p.id DESC LIMIT ?3"
        ))?;
        // "~" sorts after every hex digit, so no `before` means everything.
        let before = before.map_or_else(|| "~".to_owned(), |b| b.to_string());
        let rows = stmt.query_map(params![channel.to_string(), before, limit], post_from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    fn replies(&self, parents: &[Uuid]) -> Result<Vec<Post>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!("SELECT {POST_COLUMNS} FROM post p JOIN user u ON u.id = p.author_id WHERE p.parent_id = ?1 ORDER BY p.id"))?;
        let mut out = Vec::new();
        for parent in parents {
            let rows = stmt.query_map([parent.to_string()], post_from_row)?;
            out.extend(rows.collect::<rusqlite::Result<Vec<_>>>()?);
        }
        Ok(out)
    }

    fn mark_read(&self, user: Uuid, channel: Uuid, upto: Uuid) -> Result<()> {
        self.conn().execute(
            "INSERT INTO channel_read (user_id, channel_id, upto) VALUES (?1, ?2, ?3)
             ON CONFLICT (user_id, channel_id) DO UPDATE SET upto = MAX(upto, ?3)",
            params![user.to_string(), channel.to_string(), upto.to_string()],
        )?;
        Ok(())
    }

    fn unread(&self, user: Uuid, team: Uuid) -> Result<Vec<(Uuid, u32)>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT c.id, COUNT(p.id) FROM channel c
             JOIN post p ON p.channel_id = c.id AND p.author_id <> ?1
             LEFT JOIN channel_read r ON r.channel_id = c.id AND r.user_id = ?1
             WHERE c.team_id = ?2 AND (r.upto IS NULL OR p.id > r.upto)
             GROUP BY c.id",
        )?;
        let rows = stmt.query_map([user.to_string(), team.to_string()], |r| Ok((uuid_at(r, 0)?, r.get::<_, u32>(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }
}
