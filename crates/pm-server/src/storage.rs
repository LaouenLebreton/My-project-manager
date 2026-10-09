//! Where the server keeps its data. A trait, so a team server can use one SQLite file and a
//! cloud deployment a managed Postgres (iteration 3, step 7) without touching the handlers.

use std::sync::Mutex;

use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("there is already an account for {0}")]
    EmailTaken(String),
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
}
