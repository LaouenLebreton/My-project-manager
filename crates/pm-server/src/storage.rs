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
    /// Empty once deleted.
    pub body: String,
    /// When the author last edited it.
    pub edited_at: Option<String>,
    pub deleted: bool,
    pub reactions: Vec<Reaction>,
    /// Written by a rule (shown as from Super PM); `author_id` is whoever set the rule.
    #[serde(default)]
    pub automatic: bool,
}

/// One person's emoji on a post.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Reaction {
    pub emoji: String,
    pub user_id: Uuid,
    pub user_name: String,
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
    /// The session's user, unless it has gone unused for `SESSION_IDLE_DAYS`. Use keeps it
    /// alive (recorded at most once a day).
    fn session_user(&self, token_hash: &str, now: jiff::Timestamp) -> Result<Option<User>>;
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
    /// Takes someone out of a team and all its projects.
    fn remove_team_member(&self, team: Uuid, user: Uuid) -> Result<()>;
    /// Whether the user owns a project of the team.
    fn owns_projects_in(&self, team: Uuid, user: Uuid) -> Result<bool>;

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
    fn edit_post(&self, id: Uuid, body: &str, at: &str) -> Result<()>;
    /// Keeps the post (its replies stay) but empties it.
    fn delete_post(&self, id: Uuid) -> Result<()>;
    /// Adds the user's emoji to the post, or takes it away if it was there.
    fn toggle_reaction(&self, post: Uuid, user: Uuid, emoji: &str) -> Result<()>;

    /// Everything about a team, to move it to another server (projects' `data` is the last
    /// saved copy; the caller replaces it with the latest state).
    fn export_team(&self, team: Uuid) -> Result<crate::transfer::TeamExport>;
    /// Creates a team from an export, in one transaction: `new_users` are created first, the
    /// importer becomes an owner. Fails with `ProjectExists` if it was imported already.
    fn import_team(&self, bundle: &crate::transfer::TeamExport, importer: Uuid, new_users: &[(User, String)]) -> Result<Uuid>;
    fn set_team_moved(&self, team: Uuid, to: Option<&str>) -> Result<()>;
    /// Where the team went, if it moved.
    fn team_moved(&self, team: Uuid) -> Result<Option<String>>;

    /// A project's rules (JSON), who set them, and the day the last weekly summary went out.
    fn rules(&self, project: Uuid) -> Result<Option<StoredRules>>;
    fn set_rules(&self, project: Uuid, rules: &str, set_by: Uuid) -> Result<()>;
    /// Every project that has rules.
    fn all_rules(&self) -> Result<Vec<(Uuid, StoredRules)>>;
    fn set_last_summary(&self, project: Uuid, day: &str) -> Result<()>;
    /// The chat channel that belongs to a team project.
    fn project_channel(&self, project: Uuid) -> Result<Option<Channel>>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredRules {
    pub rules: String,
    pub set_by: Uuid,
    pub last_summary: Option<String>,
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
    // 5 -> 6: editing and deleting posts, and reactions.
    r#"
    ALTER TABLE post ADD COLUMN edited_at TEXT;
    ALTER TABLE post ADD COLUMN deleted INTEGER NOT NULL DEFAULT 0;
    CREATE TABLE reaction (
        post_id TEXT NOT NULL REFERENCES post (id) ON DELETE CASCADE,
        user_id TEXT NOT NULL REFERENCES user (id) ON DELETE CASCADE,
        emoji   TEXT NOT NULL,
        PRIMARY KEY (post_id, user_id, emoji)
    );
    "#,
    // 6 -> 7: teams moved to another server.
    r#"
    ALTER TABLE team ADD COLUMN moved_to TEXT;
    "#,
    // 8 -> 9 below. 7 -> 8: automation rules (iteration 4). Posts written by a rule are marked.
    r#"
    ALTER TABLE post ADD COLUMN automatic INTEGER NOT NULL DEFAULT 0;
    CREATE TABLE project_rules (
        project_id   TEXT PRIMARY KEY REFERENCES project (id) ON DELETE CASCADE,
        rules        TEXT NOT NULL,
        set_by       TEXT NOT NULL REFERENCES user (id),
        last_summary TEXT
    );
    "#,
    // 8 -> 9: sessions end after a long time unused (security review, 2026-11-11).
    r#"
    ALTER TABLE session ADD COLUMN last_used TEXT;
    UPDATE session SET last_used = created_at;
    "#,
];

/// A session nobody has used for this long ends; the person signs in again.
pub const SESSION_IDLE_DAYS: i64 = 90;

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

const POST_COLUMNS: &str = "p.id, p.channel_id, p.parent_id, p.author_id, CASE WHEN p.automatic THEN 'Super PM' ELSE u.name END, p.at, p.body, p.edited_at, p.deleted, p.automatic";

fn post_from_row(r: &rusqlite::Row) -> rusqlite::Result<Post> {
    Ok(Post {
        id: uuid_at(r, 0)?,
        channel_id: uuid_at(r, 1)?,
        parent_id: opt_uuid_at(r, 2)?,
        author_id: uuid_at(r, 3)?,
        author_name: r.get(4)?,
        at: r.get(5)?,
        body: r.get(6)?,
        edited_at: r.get(7)?,
        deleted: r.get(8)?,
        reactions: Vec::new(),
        automatic: r.get(9)?,
    })
}

/// Fills in each post's reactions, oldest first.
fn with_reactions(conn: &Connection, posts: &mut [Post]) -> rusqlite::Result<()> {
    let mut stmt = conn.prepare("SELECT r.emoji, r.user_id, u.name FROM reaction r JOIN user u ON u.id = r.user_id WHERE r.post_id = ?1 ORDER BY r.rowid")?;
    for p in posts {
        let rows = stmt.query_map([p.id.to_string()], |r| Ok(Reaction { emoji: r.get(0)?, user_id: uuid_at(r, 1)?, user_name: r.get(2)? }))?;
        p.reactions = rows.collect::<rusqlite::Result<_>>()?;
    }
    Ok(())
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
        self.conn().execute(
            "INSERT INTO session (token_hash, user_id, created_at, last_used) VALUES (?1, ?2, ?3, ?3)",
            params![token_hash, user.to_string(), created_at],
        )?;
        Ok(())
    }

    fn session_user(&self, token_hash: &str, now: jiff::Timestamp) -> Result<Option<User>> {
        let day = jiff::SignedDuration::from_hours(24);
        let idle_since = (now - day * SESSION_IDLE_DAYS as i32).to_string();
        let conn = self.conn();
        let user = conn
            .query_row(
                "SELECT u.id, u.name, u.email, u.is_admin, u.created_at FROM session s JOIN user u ON u.id = s.user_id
                 WHERE s.token_hash = ?1 AND s.last_used >= ?2",
                params![token_hash, idle_since],
                user_from_row,
            )
            .optional()?;
        if user.is_some() {
            conn.execute(
                "UPDATE session SET last_used = ?2 WHERE token_hash = ?1 AND last_used < ?3",
                params![token_hash, now.to_string(), (now - day).to_string()],
            )?;
        }
        Ok(user)
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

    fn remove_team_member(&self, team: Uuid, user: Uuid) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        tx.execute(
            "DELETE FROM project_member WHERE user_id = ?2 AND project_id IN (SELECT id FROM project WHERE team_id = ?1)",
            params![team.to_string(), user.to_string()],
        )?;
        tx.execute("DELETE FROM team_member WHERE team_id = ?1 AND user_id = ?2", params![team.to_string(), user.to_string()])?;
        tx.commit()?;
        Ok(())
    }

    fn owns_projects_in(&self, team: Uuid, user: Uuid) -> Result<bool> {
        Ok(self.conn().query_row(
            "SELECT EXISTS (SELECT 1 FROM project WHERE team_id = ?1 AND owner_id = ?2)",
            params![team.to_string(), user.to_string()],
            |r| r.get(0),
        )?)
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
            "INSERT INTO post (id, channel_id, parent_id, author_id, at, body, automatic) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![p.id.to_string(), p.channel_id.to_string(), p.parent_id.map(|x| x.to_string()), p.author_id.to_string(), p.at, p.body, p.automatic],
        )?;
        Ok(())
    }

    fn post(&self, id: Uuid) -> Result<Option<Post>> {
        let conn = self.conn();
        let mut found = conn.query_row(&format!("SELECT {POST_COLUMNS} FROM post p JOIN user u ON u.id = p.author_id WHERE p.id = ?1"), [id.to_string()], post_from_row).optional()?;
        if let Some(p) = found.as_mut() {
            with_reactions(&conn, std::slice::from_mut(p))?;
        }
        Ok(found)
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
        let mut posts: Vec<Post> = rows.collect::<rusqlite::Result<_>>()?;
        with_reactions(&conn, &mut posts)?;
        Ok(posts)
    }

    fn replies(&self, parents: &[Uuid]) -> Result<Vec<Post>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!("SELECT {POST_COLUMNS} FROM post p JOIN user u ON u.id = p.author_id WHERE p.parent_id = ?1 ORDER BY p.id"))?;
        let mut out = Vec::new();
        for parent in parents {
            let rows = stmt.query_map([parent.to_string()], post_from_row)?;
            out.extend(rows.collect::<rusqlite::Result<Vec<_>>>()?);
        }
        with_reactions(&conn, &mut out)?;
        Ok(out)
    }

    fn edit_post(&self, id: Uuid, body: &str, at: &str) -> Result<()> {
        self.conn().execute("UPDATE post SET body = ?2, edited_at = ?3 WHERE id = ?1 AND deleted = 0", params![id.to_string(), body, at])?;
        Ok(())
    }

    fn delete_post(&self, id: Uuid) -> Result<()> {
        let conn = self.conn();
        conn.execute("UPDATE post SET body = '', deleted = 1 WHERE id = ?1", [id.to_string()])?;
        conn.execute("DELETE FROM reaction WHERE post_id = ?1", [id.to_string()])?;
        Ok(())
    }

    fn toggle_reaction(&self, post: Uuid, user: Uuid, emoji: &str) -> Result<()> {
        let conn = self.conn();
        let removed = conn.execute("DELETE FROM reaction WHERE post_id = ?1 AND user_id = ?2 AND emoji = ?3", params![post.to_string(), user.to_string(), emoji])?;
        if removed == 0 {
            conn.execute("INSERT INTO reaction (post_id, user_id, emoji) VALUES (?1, ?2, ?3)", params![post.to_string(), user.to_string(), emoji])?;
        }
        Ok(())
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
             JOIN post p ON p.channel_id = c.id AND (p.author_id <> ?1 OR p.automatic)
             LEFT JOIN channel_read r ON r.channel_id = c.id AND r.user_id = ?1
             WHERE c.team_id = ?2 AND (r.upto IS NULL OR p.id > r.upto)
             GROUP BY c.id",
        )?;
        let rows = stmt.query_map([user.to_string(), team.to_string()], |r| Ok((uuid_at(r, 0)?, r.get::<_, u32>(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    fn rules(&self, project: Uuid) -> Result<Option<StoredRules>> {
        Ok(self
            .conn()
            .query_row("SELECT rules, set_by, last_summary FROM project_rules WHERE project_id = ?1", [project.to_string()], |r| {
                Ok(StoredRules { rules: r.get(0)?, set_by: uuid_at(r, 1)?, last_summary: r.get(2)? })
            })
            .optional()?)
    }

    fn set_rules(&self, project: Uuid, rules: &str, set_by: Uuid) -> Result<()> {
        self.conn().execute(
            "INSERT INTO project_rules (project_id, rules, set_by) VALUES (?1, ?2, ?3)
             ON CONFLICT (project_id) DO UPDATE SET rules = excluded.rules, set_by = excluded.set_by",
            params![project.to_string(), rules, set_by.to_string()],
        )?;
        Ok(())
    }

    fn all_rules(&self) -> Result<Vec<(Uuid, StoredRules)>> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT project_id, rules, set_by, last_summary FROM project_rules")?;
        let rows = stmt.query_map([], |r| Ok((uuid_at(r, 0)?, StoredRules { rules: r.get(1)?, set_by: uuid_at(r, 2)?, last_summary: r.get(3)? })))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    fn set_last_summary(&self, project: Uuid, day: &str) -> Result<()> {
        self.conn().execute("UPDATE project_rules SET last_summary = ?2 WHERE project_id = ?1", params![project.to_string(), day])?;
        Ok(())
    }

    fn project_channel(&self, project: Uuid) -> Result<Option<Channel>> {
        Ok(self
            .conn()
            .query_row("SELECT id, team_id, name, project_id, created_at FROM channel WHERE project_id = ?1", [project.to_string()], channel_from_row)
            .optional()?)
    }

    fn export_team(&self, team: Uuid) -> Result<crate::transfer::TeamExport> {
        use crate::transfer::*;
        let conn = self.conn();
        let t = team.to_string();
        let name: String = conn.query_row("SELECT name FROM team WHERE id = ?1", [&t], |r| r.get(0))?;
        let emails: std::collections::HashMap<String, String> = conn
            .prepare("SELECT id, email FROM user")?
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        let email = |id: &str| emails.get(id).cloned().unwrap_or_default();
        let members = conn
            .prepare("SELECT u.name, u.email, m.role FROM team_member m JOIN user u ON u.id = m.user_id WHERE m.team_id = ?1")?
            .query_map([&t], |r| Ok(ExMember { name: r.get(0)?, email: r.get(1)?, role: team_role_at(r, 2)? }))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut projects = Vec::new();
        let rows: Vec<(String, String, String, String, i64, String)> = conn
            .prepare("SELECT id, name, owner_id, created_at, version, data FROM project WHERE team_id = ?1 ORDER BY created_at")?
            .query_map([&t], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)))?
            .collect::<rusqlite::Result<_>>()?;
        for (id, pname, owner, created_at, version, data) in rows {
            let changes = conn
                .prepare("SELECT version, client_change, author_id, at, label, summary, command, inverse FROM change WHERE project_id = ?1 ORDER BY version")?
                .query_map([&id], |r| {
                    Ok(ExChange {
                        version: r.get(0)?,
                        client_change: uuid_at(r, 1)?,
                        author_email: email(&r.get::<_, String>(2)?),
                        at: r.get(3)?,
                        label: r.get(4)?,
                        summary: r.get(5)?,
                        command: r.get(6)?,
                        inverse: r.get(7)?,
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            let pm = conn
                .prepare("SELECT user_id, role, limits FROM project_member WHERE project_id = ?1")?
                .query_map([&id], |r| Ok((email(&r.get::<_, String>(0)?), r.get(1)?, r.get(2)?)))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            let id = Uuid::parse_str(&id).map_err(|e| StorageError::Damaged(e.to_string()))?;
            projects.push(ExProject { id, name: pname, owner_email: email(&owner), created_at, version, data, changes, members: pm });
        }
        let mut channels = Vec::new();
        let rows: Vec<Channel> = conn
            .prepare("SELECT id, team_id, name, project_id, created_at FROM channel WHERE team_id = ?1 ORDER BY created_at")?
            .query_map([&t], channel_from_row)?
            .collect::<rusqlite::Result<_>>()?;
        for c in rows {
            let mut posts = conn
                .prepare("SELECT id, parent_id, author_id, at, body, edited_at, deleted FROM post WHERE channel_id = ?1 ORDER BY id")?
                .query_map([c.id.to_string()], |r| {
                    Ok(ExPost {
                        id: uuid_at(r, 0)?,
                        parent_id: opt_uuid_at(r, 1)?,
                        author_email: email(&r.get::<_, String>(2)?),
                        at: r.get(3)?,
                        body: r.get(4)?,
                        edited_at: r.get(5)?,
                        deleted: r.get(6)?,
                        reactions: Vec::new(),
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            let mut stmt = conn.prepare("SELECT emoji, user_id FROM reaction WHERE post_id = ?1 ORDER BY rowid")?;
            for p in &mut posts {
                p.reactions = stmt.query_map([p.id.to_string()], |r| Ok((r.get::<_, String>(0)?, email(&r.get::<_, String>(1)?))))?.collect::<rusqlite::Result<_>>()?;
            }
            channels.push(ExChannel { id: c.id, name: c.name, project_id: c.project_id, created_at: c.created_at, posts });
        }
        Ok(TeamExport { name, members, projects, channels })
    }

    fn import_team(&self, b: &crate::transfer::TeamExport, importer: Uuid, new_users: &[(User, String)]) -> Result<Uuid> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        for (u, hash) in new_users {
            tx.execute(
                "INSERT OR IGNORE INTO user (id, name, email, password_hash, is_admin, created_at) VALUES (?1, ?2, ?3, ?4, 0, ?5)",
                params![u.id.to_string(), u.name, u.email, hash, u.created_at],
            )?;
        }
        let ids: std::collections::HashMap<String, String> = tx
            .prepare("SELECT email, id FROM user")?
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        let me = importer.to_string();
        let id_of = |email: &str| ids.get(email).cloned().unwrap_or_else(|| me.clone());
        let team = Uuid::now_v7();
        let t = team.to_string();
        let now = jiff::Timestamp::now().to_string();
        tx.execute("INSERT INTO team (id, name, created_at) VALUES (?1, ?2, ?3)", params![t, b.name.trim(), now])?;
        for m in &b.members {
            let role = if m.role == TeamRole::Owner { "owner" } else { "member" };
            tx.execute("INSERT OR IGNORE INTO team_member (team_id, user_id, role) VALUES (?1, ?2, ?3)", params![t, id_of(&m.email), role])?;
        }
        tx.execute(
            "INSERT INTO team_member (team_id, user_id, role) VALUES (?1, ?2, 'owner') ON CONFLICT (team_id, user_id) DO UPDATE SET role = 'owner'",
            params![t, me],
        )?;
        for p in &b.projects {
            let inserted = tx.execute(
                "INSERT OR IGNORE INTO project (id, team_id, name, owner_id, version, data, created_at, updated_at, data_version) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?5)",
                params![p.id.to_string(), t, p.name, id_of(&p.owner_email), p.version, p.data, p.created_at, now],
            )?;
            if inserted == 0 {
                return Err(StorageError::ProjectExists);
            }
            for c in &p.changes {
                tx.execute(
                    "INSERT INTO change (project_id, version, author_id, at, label, command, client_change, summary, inverse) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                    params![p.id.to_string(), c.version, id_of(&c.author_email), c.at, c.label, c.command, c.client_change.to_string(), c.summary, c.inverse],
                )?;
            }
            for (email, role, limits) in &p.members {
                tx.execute(
                    "INSERT OR REPLACE INTO project_member (project_id, user_id, role, limits) VALUES (?1, ?2, ?3, ?4)",
                    params![p.id.to_string(), id_of(email), role, limits],
                )?;
            }
        }
        for c in &b.channels {
            tx.execute(
                "INSERT INTO channel (id, team_id, name, project_id, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![c.id.to_string(), t, c.name, c.project_id.map(|x| x.to_string()), c.created_at],
            )?;
            for p in &c.posts {
                tx.execute(
                    "INSERT INTO post (id, channel_id, parent_id, author_id, at, body, edited_at, deleted) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    params![p.id.to_string(), c.id.to_string(), p.parent_id.map(|x| x.to_string()), id_of(&p.author_email), p.at, p.body, p.edited_at, p.deleted],
                )?;
                for (emoji, email) in &p.reactions {
                    tx.execute("INSERT OR IGNORE INTO reaction (post_id, user_id, emoji) VALUES (?1, ?2, ?3)", params![p.id.to_string(), id_of(email), emoji])?;
                }
            }
        }
        tx.commit()?;
        Ok(team)
    }

    fn set_team_moved(&self, team: Uuid, to: Option<&str>) -> Result<()> {
        self.conn().execute("UPDATE team SET moved_to = ?2 WHERE id = ?1", params![team.to_string(), to])?;
        Ok(())
    }

    fn team_moved(&self, team: Uuid) -> Result<Option<String>> {
        Ok(self.conn().query_row("SELECT moved_to FROM team WHERE id = ?1", [team.to_string()], |r| r.get(0)).optional()?.flatten())
    }
}
