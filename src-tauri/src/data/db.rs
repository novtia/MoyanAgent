use std::path::Path;

use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::params;

use crate::error::AppResult;

pub type DbPool = Pool<SqliteConnectionManager>;
pub type DbConn = r2d2::PooledConnection<SqliteConnectionManager>;

const MIGRATION_001: &str = include_str!("../../migrations/001_init.sql");
const MIGRATION_002: &str = include_str!("../../migrations/002_session_system_prompt.sql");
const MIGRATION_003: &str = include_str!("../../migrations/003_session_history_turns.sql");
const MIGRATION_004: &str = include_str!("../../migrations/004_session_llm_params.sql");

pub fn open_pool(db_path: &Path) -> AppResult<DbPool> {
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let manager = SqliteConnectionManager::file(db_path).with_init(|c| {
        c.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             PRAGMA foreign_keys = ON;",
        )
    });
    let pool = Pool::builder().max_size(8).build(manager)?;
    {
        let conn = pool.get()?;
        run_migrations(&conn)?;
    }
    Ok(pool)
}

fn run_migrations(conn: &rusqlite::Connection) -> AppResult<()> {
    conn.execute(
        "CREATE TABLE IF NOT EXISTS schema_version (version INTEGER NOT NULL)",
        params![],
    )?;
    let cur: i64 = conn
        .query_row(
            "SELECT COALESCE(MAX(version), 0) FROM schema_version",
            params![],
            |r| r.get(0),
        )
        .unwrap_or(0);
    if cur < 1 {
        conn.execute_batch(MIGRATION_001)?;
        conn.execute("INSERT INTO schema_version(version) VALUES (1)", params![])?;
    }
    if cur < 2 {
        conn.execute_batch(MIGRATION_002)?;
        conn.execute("INSERT INTO schema_version(version) VALUES (2)", params![])?;
    }
    if cur < 3 {
        conn.execute_batch(MIGRATION_003)?;
        conn.execute("INSERT INTO schema_version(version) VALUES (3)", params![])?;
    }
    if cur < 4 {
        conn.execute_batch(MIGRATION_004)?;
        conn.execute("INSERT INTO schema_version(version) VALUES (4)", params![])?;
    }
    Ok(())
}

pub fn now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}
