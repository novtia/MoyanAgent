use std::path::Path;

use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::params;

use crate::error::AppResult;

pub type DbPool = Pool<SqliteConnectionManager>;
pub type DbConn = r2d2::PooledConnection<SqliteConnectionManager>;

const MIGRATION_001: &str =
    include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/migrations/001_init.sql"));
const MIGRATION_002: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/migrations/002_session_system_prompt.sql"
));
const MIGRATION_003: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/migrations/003_session_history_turns.sql"
));
const MIGRATION_004: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/migrations/004_session_llm_params.sql"
));
const MIGRATION_005: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/migrations/005_agent_events.sql"
));
const MIGRATION_006: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/migrations/006_llm_catalog.sql"
));
const MIGRATION_007: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/migrations/007_context_window.sql"
));

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
    if cur < 5 {
        conn.execute_batch(MIGRATION_005)?;
        conn.execute("INSERT INTO schema_version(version) VALUES (5)", params![])?;
    }
    if cur < 6 {
        conn.execute_batch(MIGRATION_006)?;
        conn.execute("INSERT INTO schema_version(version) VALUES (6)", params![])?;
    }
    if cur < 7 {
        conn.execute_batch(MIGRATION_007)?;
        conn.execute("INSERT INTO schema_version(version) VALUES (7)", params![])?;
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
