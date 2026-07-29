use std::path::Path;

use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::params;

use crate::error::AppResult;

use super::message_search;

pub type DbPool = Pool<SqliteConnectionManager>;
pub type DbConn = r2d2::PooledConnection<SqliteConnectionManager>;

/// Squashed baseline is 28; 29 adds message/session FTS search indexes.
const SCHEMA_VERSION: i64 = 29;

const MIGRATION_001: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/migrations/001_init.sql"
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

fn table_exists(conn: &rusqlite::Connection, name: &str) -> bool {
    conn.query_row(
        "SELECT 1 FROM sqlite_master WHERE name=?1 LIMIT 1",
        params![name],
        |_| Ok(()),
    )
    .is_ok()
}

fn column_exists(conn: &rusqlite::Connection, table: &str, column: &str) -> bool {
    let mut stmt = match conn.prepare(&format!("PRAGMA table_info({table})")) {
        Ok(s) => s,
        Err(_) => return false,
    };
    let rows = match stmt.query_map(params![], |r| r.get::<_, String>(1)) {
        Ok(rows) => rows,
        Err(_) => return false,
    };
    for row in rows.flatten() {
        if row == column {
            return true;
        }
    }
    false
}

/// Idempotent: add searchable_text + FTS tables/triggers if missing.
/// Returns true when a full index backfill is required.
fn ensure_message_search_schema(conn: &rusqlite::Connection) -> AppResult<bool> {
    let mut need_backfill = false;

    if !column_exists(conn, "messages", "searchable_text") {
        conn.execute(
            "ALTER TABLE messages ADD COLUMN searchable_text TEXT NOT NULL DEFAULT ''",
            params![],
        )?;
        need_backfill = true;
    }

    if !table_exists(conn, "messages_fts") {
        conn.execute_batch(
            "CREATE VIRTUAL TABLE messages_fts USING fts5(
               searchable_text,
               content='messages',
               content_rowid='rowid',
               tokenize='trigram'
             );",
        )?;
        need_backfill = true;
    }

    if !table_exists(conn, "sessions_fts") {
        conn.execute_batch(
            "CREATE VIRTUAL TABLE sessions_fts USING fts5(
               title,
               content='sessions',
               content_rowid='rowid',
               tokenize='trigram'
             );",
        )?;
        need_backfill = true;
    }

    conn.execute_batch(
        "CREATE TRIGGER IF NOT EXISTS messages_fts_ad AFTER DELETE ON messages BEGIN
           INSERT INTO messages_fts(messages_fts, rowid, searchable_text)
             VALUES('delete', old.rowid, old.searchable_text);
         END;
         CREATE TRIGGER IF NOT EXISTS sessions_fts_ad AFTER DELETE ON sessions BEGIN
           INSERT INTO sessions_fts(sessions_fts, rowid, title)
             VALUES('delete', old.rowid, old.title);
         END;",
    )?;

    Ok(need_backfill)
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

    // Fresh DB: create final schema in one shot (includes FTS tables).
    if cur == 0 {
        conn.execute_batch(MIGRATION_001)?;
    }

    // Fresh installs and legacy DBs (1..=27) stamp to squashed baseline 28 first.
    if cur < 28 {
        conn.execute(
            "INSERT INTO schema_version(version) VALUES (?1)",
            params![28],
        )?;
    }

    let cur: i64 = conn
        .query_row(
            "SELECT COALESCE(MAX(version), 0) FROM schema_version",
            params![],
            |r| r.get(0),
        )
        .unwrap_or(0);

    // Ensure FTS schema exists even if a prior partial upgrade stamped columns
    // without virtual tables, or stamped version 29 prematurely.
    let need_backfill = ensure_message_search_schema(conn)?;
    if cur < 29 || need_backfill {
        message_search::backfill_search_index(conn)?;
    }
    if cur < 29 {
        conn.execute(
            "INSERT INTO schema_version(version) VALUES (?1)",
            params![SCHEMA_VERSION],
        )?;
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
