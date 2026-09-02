use std::path::Path;

use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::params;

use crate::error::AppResult;

use super::message_search;

pub type DbPool = Pool<SqliteConnectionManager>;
pub type DbConn = r2d2::PooledConnection<SqliteConnectionManager>;

/// Squashed baseline is 28; 29 adds message/session FTS search indexes;
/// 30 makes file snapshots persist at write time; 31 stores OpenRouter
/// per-model route provider pins.
const SCHEMA_VERSION: i64 = 31;

const MIGRATION_001: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/migrations/001_init.sql"
));

const MIGRATION_030: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/migrations/030_file_snapshot_request_binding.sql"
));

pub fn open_pool(db_path: &Path) -> AppResult<DbPool> {
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let manager = SqliteConnectionManager::file(db_path).with_init(|c| {
        c.execute_batch(
            // busy_timeout defaults to 0, which turns any overlap with the
            // backup job's `BEGIN IMMEDIATE` into an instant "database is
            // locked" instead of a short wait.
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             PRAGMA busy_timeout = 5000;
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

/// Idempotent: rebuild `file_snapshots` with the write-time binding columns.
/// Guarded on the column rather than the version stamp so a DB created from
/// the current `001_init.sql` skips the rebuild entirely.
fn ensure_file_snapshot_binding_schema(conn: &rusqlite::Connection) -> AppResult<()> {
    if !table_exists(conn, "file_snapshots")
        || column_exists(conn, "file_snapshots", "request_message_id")
    {
        return Ok(());
    }
    conn.execute_batch(MIGRATION_030)?;
    Ok(())
}

/// Idempotent: catalog columns + user overlay table for OpenRouter routing slugs.
fn ensure_model_route_schema(conn: &rusqlite::Connection) -> AppResult<()> {
    if table_exists(conn, "llm_sdk_model")
        && !column_exists(conn, "llm_sdk_model", "route_providers_json")
    {
        conn.execute(
            "ALTER TABLE llm_sdk_model ADD COLUMN route_providers_json TEXT NOT NULL DEFAULT '[]'",
            params![],
        )?;
    }
    if table_exists(conn, "llm_supplier_model")
        && !column_exists(conn, "llm_supplier_model", "route_providers_json")
    {
        conn.execute(
            "ALTER TABLE llm_supplier_model ADD COLUMN route_providers_json TEXT NOT NULL DEFAULT '[]'",
            params![],
        )?;
    }
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS llm_model_route (
           provider_id TEXT NOT NULL,
           model_id TEXT NOT NULL,
           route_providers_json TEXT NOT NULL DEFAULT '[]',
           PRIMARY KEY (provider_id, model_id)
         );",
    )?;
    Ok(())
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
        conn.execute("INSERT INTO schema_version(version) VALUES (?1)", params![29])?;
    }

    ensure_file_snapshot_binding_schema(conn)?;
    if cur < 30 {
        conn.execute("INSERT INTO schema_version(version) VALUES (?1)", params![30])?;
    }

    ensure_model_route_schema(conn)?;
    if cur < 31 {
        crate::data::llm_catalog::backfill_route_overlay_from_settings(conn)?;
        conn.execute(
            "INSERT INTO schema_version(version) VALUES (?1)",
            params![SCHEMA_VERSION],
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The pre-30 table: `message_id` mandatory, no request binding.
    const LEGACY_FILE_SNAPSHOTS: &str = "
        DROP TABLE file_snapshots;
        CREATE TABLE file_snapshots (
          id              INTEGER PRIMARY KEY AUTOINCREMENT,
          session_id      TEXT NOT NULL,
          message_id      TEXT NOT NULL,
          path            TEXT NOT NULL,
          op              TEXT NOT NULL,
          before_existed  INTEGER NOT NULL,
          before_content  TEXT,
          restorable      INTEGER NOT NULL,
          created_at      INTEGER NOT NULL,
          before_encoding TEXT,
          before_had_bom  INTEGER NOT NULL DEFAULT 0
        );
        INSERT INTO file_snapshots(
          session_id, message_id, path, op, before_existed, restorable, created_at)
          VALUES('s1', 'a1', 'proj/one.md', 'create', 0, 1, 1700000000000);
        DELETE FROM schema_version WHERE version >= 30;";

    #[test]
    fn upgrading_a_pre_binding_database_keeps_its_snapshots() {
        let dir = std::env::temp_dir().join(format!("atelier-migrate-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("legacy.db");

        {
            let pool = open_pool(&db_path).unwrap();
            let conn = pool.get().unwrap();
            conn.execute_batch(LEGACY_FILE_SNAPSHOTS).unwrap();
            assert!(!column_exists(&conn, "file_snapshots", "request_message_id"));
        }

        let pool = open_pool(&db_path).unwrap();
        let conn = pool.get().unwrap();
        assert!(column_exists(&conn, "file_snapshots", "request_message_id"));
        assert!(column_exists(&conn, "file_snapshots", "rollback_error"));

        let (message_id, path, request): (Option<String>, String, Option<String>) = conn
            .query_row(
                "SELECT message_id, path, request_message_id FROM file_snapshots",
                params![],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .expect("existing snapshot survives the rebuild");
        assert_eq!(message_id.as_deref(), Some("a1"));
        assert_eq!(path, "proj/one.md");
        assert!(request.is_none());

        // A second open must not rebuild again.
        drop(conn);
        drop(pool);
        let pool = open_pool(&db_path).unwrap();
        let conn = pool.get().unwrap();
        let rows: i64 = conn
            .query_row("SELECT COUNT(*) FROM file_snapshots", params![], |r| r.get(0))
            .unwrap();
        assert_eq!(rows, 1);

        drop(conn);
        drop(pool);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn fresh_database_has_model_route_schema() {
        let db = test_support::TempDb::new("route-schema");
        let conn = db.conn();
        assert!(column_exists(&conn, "llm_sdk_model", "route_providers_json"));
        assert!(column_exists(
            &conn,
            "llm_supplier_model",
            "route_providers_json"
        ));
        assert!(table_exists(&conn, "llm_model_route"));
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU32, Ordering};

    use super::{open_pool, DbConn, DbPool};

    static COUNTER: AtomicU32 = AtomicU32::new(0);

    /// Throwaway on-disk database with the full schema applied, removed when
    /// the handle drops. SQLite in-memory databases are per-connection, so a
    /// pooled test needs a real file.
    pub(crate) struct TempDb {
        pool: Option<DbPool>,
        dir: PathBuf,
    }

    impl TempDb {
        pub(crate) fn new(tag: &str) -> Self {
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let dir = std::env::temp_dir()
                .join(format!("atelier-db-{tag}-{}-{n}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("create temp db dir");
            let pool = open_pool(&dir.join("test.db")).expect("open temp db");
            Self {
                pool: Some(pool),
                dir,
            }
        }

        pub(crate) fn conn(&self) -> DbConn {
            self.pool.as_ref().expect("pool alive").get().expect("conn")
        }

        pub(crate) fn pool(&self) -> DbPool {
            self.pool.as_ref().expect("pool alive").clone()
        }
    }

    impl Drop for TempDb {
        fn drop(&mut self) {
            // Windows keeps the .db file locked until every connection closes.
            drop(self.pool.take());
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }
}

pub fn now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}
