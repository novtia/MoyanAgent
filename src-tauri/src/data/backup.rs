//! Modular application data backup / restore.
//!
//! Packages are ZIP archives with:
//! - `manifest.json`
//! - `data/{table}.json` (row arrays)
//! - `sessions/...` media files (chat / full only)

use std::collections::HashSet;
use std::fmt;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use chrono::{Datelike, Local, NaiveTime, Timelike};
use rusqlite::{params_from_iter, types::ValueRef, Connection};
use serde::de::{self, Deserializer, SeqAccess, Visitor};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use tauri::{AppHandle, Emitter};
use zip::write::SimpleFileOptions;
use zip::ZipArchive;

use crate::data::db::{now_ms, DbConn, DbPool};
use crate::data::message_search;
use crate::data::paths;
use crate::data::settings::{self, Settings};
use crate::error::{AppError, AppResult};

pub const MANIFEST_FORMAT: &str = "atelier-backup";
pub const MANIFEST_VERSION: &str = "1";
pub const STATE_FILE: &str = ".backup-state.json";

pub const CONFIG_SLOT_TIMES: &[&str] = &["00:00", "06:00"];

const CONFIG_TABLES: &[&str] = &["settings", "custom_agents", "projects"];
const CHAT_TABLES: &[&str] = &[
    "sessions",
    "messages",
    "message_images",
    "role_state_snapshots",
    "file_snapshots",
    "pending_diffs",
];
const USAGE_TABLES: &[&str] = &["token_usage_events"];

static BACKUP_LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackupModule {
    Config,
    Chat,
    Usage,
    Full,
}

impl BackupModule {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Config => "config",
            Self::Chat => "chat",
            Self::Usage => "usage",
            Self::Full => "full",
        }
    }

    pub fn parse(s: &str) -> AppResult<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "config" => Ok(Self::Config),
            "chat" => Ok(Self::Chat),
            "usage" => Ok(Self::Usage),
            "full" => Ok(Self::Full),
            other => Err(AppError::Invalid(format!("unknown backup module: {other}"))),
        }
    }

    fn tables(self) -> Vec<&'static str> {
        match self {
            Self::Config => CONFIG_TABLES.to_vec(),
            Self::Chat => CHAT_TABLES.to_vec(),
            Self::Usage => USAGE_TABLES.to_vec(),
            Self::Full => {
                let mut t = Vec::new();
                t.extend_from_slice(CONFIG_TABLES);
                t.extend_from_slice(CHAT_TABLES);
                t.extend_from_slice(USAGE_TABLES);
                t
            }
        }
    }

    fn includes_sessions_media(self) -> bool {
        matches!(self, Self::Chat | Self::Full)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackupKind {
    Auto,
    Manual,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackupScope {
    Full,
    /// Only sessions changed since `since_ms` (auto chat backups).
    Delta,
}

impl Default for BackupScope {
    fn default() -> Self {
        Self::Full
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupManifest {
    pub format: String,
    pub version: String,
    pub module: BackupModule,
    pub kind: BackupKind,
    pub created_at: i64,
    pub app_version: String,
    pub schema_version: i64,
    pub tables: Vec<String>,
    /// `full` (default) replaces the whole module on restore; `delta` merges changed sessions.
    #[serde(default)]
    pub scope: BackupScope,
    /// Lower bound (exclusive) used to select dirty sessions for delta backups.
    #[serde(default)]
    pub since_ms: Option<i64>,
    /// Session ids included in a delta chat backup.
    #[serde(default)]
    pub session_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupResult {
    pub path: String,
    pub module: BackupModule,
    pub created_at: i64,
    pub kind: BackupKind,
    /// True when auto chat found no dirty sessions and wrote nothing.
    #[serde(default)]
    pub skipped: bool,
    #[serde(default)]
    pub scope: BackupScope,
    #[serde(default)]
    pub session_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RestoreResult {
    pub module: BackupModule,
    pub path: String,
    pub requires_restart: bool,
    /// Non-fatal issues (e.g. usage table restore failed after core data landed).
    #[serde(default)]
    pub warnings: Vec<String>,
}

/// Progress payload for `backup://progress` (backup + restore).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupProgressEvent {
    pub op: String,
    pub phase: String,
    /// Overall progress 0–100 (monotonic within one operation).
    pub percent: u8,
    pub current: u64,
    pub total: u64,
    pub detail: String,
}

/// Tracks overall 0–100% progress; never goes backwards.
struct ProgressClock {
    app: AppHandle,
    op: String,
    last: std::cell::Cell<u8>,
}

impl ProgressClock {
    fn new(app: &AppHandle, op: &str) -> Self {
        Self {
            app: app.clone(),
            op: op.into(),
            last: std::cell::Cell::new(0),
        }
    }

    fn emit(&self, phase: &str, percent: u8, current: u64, total: u64, detail: &str) {
        let p = percent.min(100).max(self.last.get());
        self.last.set(p);
        let _ = self.app.emit(
            "backup://progress",
            BackupProgressEvent {
                op: self.op.clone(),
                phase: phase.into(),
                percent: p,
                current,
                total,
                detail: detail.into(),
            },
        );
    }

    /// Map `current/total` into `[start, end]` overall percent.
    fn span(&self, phase: &str, start: u8, end: u8, current: u64, total: u64, detail: &str) {
        let start = start.min(end);
        let end = end.max(start);
        let pct = if total == 0 {
            start
        } else {
            let span = u64::from(end.saturating_sub(start));
            let cur = current.min(total);
            start.saturating_add(((span * cur) / total) as u8)
        };
        self.emit(phase, pct, current, total, detail);
    }

    fn done(&self) {
        self.emit("done", 100, 1, 1, "");
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupListItem {
    pub path: String,
    pub module: BackupModule,
    pub kind: BackupKind,
    pub created_at: i64,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BackupStateFile {
    /// Last completed config slot id, e.g. `2026-07-29T06:00`.
    #[serde(default)]
    pub last_config_slot: Option<String>,
    #[serde(default)]
    pub last_usage_slot: Option<String>,
    #[serde(default)]
    pub last_chat_at_ms: Option<i64>,
    #[serde(default)]
    pub last_full_at_ms: Option<i64>,
    #[serde(default)]
    pub last_error: Option<String>,
    #[serde(default)]
    pub last_success_at_ms: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupStatus {
    pub enabled: bool,
    pub backup_dir: String,
    pub busy: bool,
    pub config_times: Vec<String>,
    pub chat_interval_minutes: i64,
    pub config_keep: i64,
    pub chat_keep: i64,
    pub last_config_slot: Option<String>,
    pub last_chat_at_ms: Option<i64>,
    pub last_usage_slot: Option<String>,
    pub last_full_at_ms: Option<i64>,
    pub last_success_at_ms: Option<i64>,
    pub last_error: Option<String>,
    pub next_config_slot: Option<String>,
    pub next_chat_at_ms: Option<i64>,
}

fn schema_version(conn: &Connection) -> AppResult<i64> {
    let v: i64 = conn.query_row(
        "SELECT COALESCE(MAX(version), 0) FROM schema_version",
        [],
        |r| r.get(0),
    )?;
    Ok(v)
}

fn sqlite_to_json(v: ValueRef<'_>) -> Value {
    match v {
        ValueRef::Null => Value::Null,
        ValueRef::Integer(i) => json!(i),
        ValueRef::Real(f) => json!(f),
        ValueRef::Text(t) => Value::String(String::from_utf8_lossy(t).into_owned()),
        ValueRef::Blob(b) => {
            use base64::{engine::general_purpose::STANDARD, Engine};
            json!({ "_bin": STANDARD.encode(b) })
        }
    }
}

fn json_to_sql(v: &Value) -> AppResult<rusqlite::types::Value> {
    match v {
        Value::Null => Ok(rusqlite::types::Value::Null),
        Value::Bool(b) => Ok(rusqlite::types::Value::Integer(if *b { 1 } else { 0 })),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(rusqlite::types::Value::Integer(i))
            } else if let Some(u) = n.as_u64() {
                Ok(rusqlite::types::Value::Integer(u as i64))
            } else if let Some(f) = n.as_f64() {
                Ok(rusqlite::types::Value::Real(f))
            } else {
                Err(AppError::Invalid("unsupported number in backup row".into()))
            }
        }
        Value::String(s) => Ok(rusqlite::types::Value::Text(s.clone())),
        Value::Object(map) => {
            if let Some(Value::String(b64)) = map.get("_bin") {
                use base64::{engine::general_purpose::STANDARD, Engine};
                let bytes = STANDARD
                    .decode(b64)
                    .map_err(|e| AppError::Invalid(format!("invalid blob in backup: {e}")))?;
                Ok(rusqlite::types::Value::Blob(bytes))
            } else {
                Ok(rusqlite::types::Value::Text(v.to_string()))
            }
        }
        Value::Array(_) => Ok(rusqlite::types::Value::Text(v.to_string())),
    }
}

fn assert_safe_table(table: &str) -> AppResult<()> {
    let allowed: HashSet<&str> = CONFIG_TABLES
        .iter()
        .chain(CHAT_TABLES.iter())
        .chain(USAGE_TABLES.iter())
        .copied()
        .collect();
    if !allowed.contains(table) {
        return Err(AppError::Invalid(format!("table not allowed in backup: {table}")));
    }
    Ok(())
}

/// Stream a table into an open zip entry as a JSON array (row-by-row, low RAM).
fn zip_write_table_streaming<W: Write + std::io::Seek>(
    zip: &mut zip::ZipWriter<W>,
    opts: SimpleFileOptions,
    conn: &Connection,
    table: &str,
    sql: &str,
    params: impl rusqlite::Params,
    mut on_row: impl FnMut(u64, u64),
) -> AppResult<u64> {
    assert_safe_table(table)?;
    let entry = format!("data/{table}.json");
    zip.start_file(&entry, opts)
        .map_err(|e| AppError::Other(format!("zip: {e}")))?;
    zip.write_all(b"[")?;

    let mut stmt = conn.prepare(sql).map_err(AppError::Db)?;
    let col_count = stmt.column_count();
    let col_names: Vec<String> = (0..col_count)
        .map(|i| stmt.column_name(i).unwrap_or("?").to_string())
        .collect();
    let mut rows = stmt.query(params)?;
    let mut first = true;
    let mut count = 0u64;
    while let Some(row) = rows.next()? {
        if !first {
            zip.write_all(b",")?;
        }
        first = false;
        let mut obj = Map::new();
        for (i, name) in col_names.iter().enumerate() {
            let v = row.get_ref(i)?;
            obj.insert(name.clone(), sqlite_to_json(v));
        }
        serde_json::to_writer(&mut *zip, &Value::Object(obj))?;
        count += 1;
        if count == 1 || count % 200 == 0 {
            on_row(count, 0);
        }
    }
    zip.write_all(b"]")?;
    if count > 0 {
        on_row(count, count);
    }
    Ok(count)
}

fn table_row_count(conn: &Connection, table: &str) -> u64 {
    conn.query_row(&format!("SELECT COUNT(*) FROM \"{table}\""), [], |r| {
        r.get::<_, i64>(0)
    })
    .map(|n| n.max(0) as u64)
    .unwrap_or(0)
}

fn zip_write_full_table<W: Write + std::io::Seek>(
    zip: &mut zip::ZipWriter<W>,
    opts: SimpleFileOptions,
    conn: &Connection,
    table: &str,
    on_row: impl FnMut(u64, u64),
) -> AppResult<u64> {
    zip_write_table_streaming(
        zip,
        opts,
        conn,
        table,
        &format!("SELECT * FROM \"{table}\""),
        [],
        on_row,
    )
}

fn zip_write_chat_table_for_sessions<W: Write + std::io::Seek>(
    zip: &mut zip::ZipWriter<W>,
    opts: SimpleFileOptions,
    conn: &Connection,
    table: &str,
    session_ids: &[String],
    on_row: impl FnMut(u64, u64),
) -> AppResult<u64> {
    if session_ids.is_empty() {
        zip_write_bytes(zip, opts, &format!("data/{table}.json"), b"[]")?;
        return Ok(0);
    }
    let placeholders: Vec<String> = (1..=session_ids.len()).map(|i| format!("?{i}")).collect();
    let col = if table == "sessions" {
        "id"
    } else {
        "session_id"
    };
    let sql = format!(
        "SELECT * FROM \"{table}\" WHERE \"{col}\" IN ({})",
        placeholders.join(", ")
    );
    zip_write_table_streaming(
        zip,
        opts,
        conn,
        table,
        &sql,
        params_from_iter(session_ids.iter()),
        on_row,
    )
}

/// Sessions changed after `since_ms` (exclusive), including activity on related tables.
fn changed_session_ids(conn: &Connection, since_ms: i64) -> AppResult<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT id FROM sessions WHERE updated_at > ?1
         UNION
         SELECT DISTINCT session_id FROM messages WHERE created_at > ?1
         UNION
         SELECT DISTINCT session_id FROM message_images WHERE created_at > ?1
         UNION
         SELECT DISTINCT session_id FROM role_state_snapshots WHERE created_at > ?1
         UNION
         SELECT DISTINCT session_id FROM file_snapshots WHERE created_at > ?1
         UNION
         SELECT DISTINCT session_id FROM pending_diffs WHERE created_at > ?1",
    )?;
    let ids = stmt
        .query_map([since_ms], |r| r.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(ids)
}

fn insert_one_row(conn: &Connection, table: &str, obj: &Map<String, Value>) -> AppResult<()> {
    if obj.is_empty() {
        return Ok(());
    }
    let cols: Vec<&str> = obj.keys().map(|s| s.as_str()).collect();
    let placeholders: Vec<String> = (1..=cols.len()).map(|i| format!("?{i}")).collect();
    let sql = format!(
        "INSERT INTO \"{table}\" ({}) VALUES ({})",
        cols.iter()
            .map(|c| format!("\"{c}\""))
            .collect::<Vec<_>>()
            .join(", "),
        placeholders.join(", ")
    );
    let mut values = Vec::with_capacity(cols.len());
    for c in &cols {
        values.push(json_to_sql(&obj[*c])?);
    }
    conn.execute(&sql, params_from_iter(values))?;
    Ok(())
}

#[allow(dead_code)]
fn insert_table_rows(conn: &Connection, table: &str, rows: &Value) -> AppResult<()> {
    assert_safe_table(table)?;
    let arr = rows
        .as_array()
        .ok_or_else(|| AppError::Invalid(format!("data/{table}.json must be an array")))?;
    for row in arr {
        let obj = row
            .as_object()
            .ok_or_else(|| AppError::Invalid(format!("row in {table} must be an object")))?;
        insert_one_row(conn, table, obj)?;
    }
    Ok(())
}

/// Stream-insert rows from a JSON array without loading the whole document into RAM.
fn insert_table_rows_streaming<R: Read>(
    conn: &Connection,
    table: &str,
    reader: R,
    mut on_row: impl FnMut(u64),
) -> AppResult<u64> {
    assert_safe_table(table)?;

    struct RowsVisitor<'a, F> {
        conn: &'a Connection,
        table: &'a str,
        on_row: &'a mut F,
        count: u64,
    }

    impl<'de, F: FnMut(u64)> Visitor<'de> for RowsVisitor<'_, F> {
        type Value = u64;

        fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
            f.write_str("a JSON array of row objects")
        }

        fn visit_seq<A: SeqAccess<'de>>(mut self, mut seq: A) -> Result<u64, A::Error> {
            while let Some(obj) = seq.next_element::<Map<String, Value>>()? {
                insert_one_row(self.conn, self.table, &obj).map_err(de::Error::custom)?;
                self.count += 1;
                if self.count == 1 || self.count % 100 == 0 {
                    (self.on_row)(self.count);
                }
            }
            Ok(self.count)
        }
    }

    let mut de = serde_json::Deserializer::from_reader(reader);
    let count = de
        .deserialize_seq(RowsVisitor {
            conn,
            table,
            on_row: &mut on_row,
            count: 0,
        })
        .map_err(|e| AppError::Invalid(format!("data/{table}.json: {e}")))?;
    if count > 0 {
        on_row(count);
    }
    Ok(count)
}

fn zip_entry_exists(archive_path: &Path, name: &str) -> bool {
    let Ok(file) = File::open(archive_path) else {
        return false;
    };
    let Ok(mut zip) = ZipArchive::new(file) else {
        return false;
    };
    let ok = zip.by_name(name).is_ok();
    ok
}

/// DELETE + stream INSERT for one table from the archive.
fn restore_table_streaming(
    conn: &Connection,
    archive_path: &Path,
    table: &str,
    progress: &ProgressClock,
    band_start: u8,
    band_end: u8,
) -> AppResult<()> {
    assert_safe_table(table)?;
    let entry_name = format!("data/{table}.json");
    if !zip_entry_exists(archive_path, &entry_name) {
        progress.span(
            "table",
            band_start,
            band_end,
            1,
            1,
            &format!("{table} (skipped)"),
        );
        return Ok(());
    }

    conn.execute(&format!("DELETE FROM \"{table}\""), [])?;

    let file = File::open(archive_path)?;
    let mut zip =
        ZipArchive::new(file).map_err(|e| AppError::Other(format!("zip open: {e}")))?;
    let entry = zip
        .by_name(&entry_name)
        .map_err(|e| AppError::Other(format!("missing {entry_name}: {e}")))?;

    progress.span("table", band_start, band_end, 0, 1, table);
    // Unknown row total in zip: asymptote toward band_end while streaming.
    let count = insert_table_rows_streaming(conn, table, entry, |rows| {
        let denom = (rows / 500).saturating_add(1);
        let within = u64::from(band_end.saturating_sub(band_start));
        let cur = within.saturating_mul(denom.saturating_sub(1)) / denom;
        progress.emit(
            "table_rows",
            band_start.saturating_add(cur as u8),
            rows,
            0,
            &format!("{table} · {rows}"),
        );
    })?;
    progress.span(
        "table",
        band_start,
        band_end,
        1,
        1,
        &format!("{table} ({count})"),
    );
    Ok(())
}

/// Replace data for the listed sessions only (delta restore).
fn restore_chat_delta(
    conn: &Connection,
    session_ids: &[String],
    tables: &[&str],
    path: &Path,
    progress: &ProgressClock,
    band_start: u8,
    band_end: u8,
) -> AppResult<()> {
    for sid in session_ids {
        conn.execute("DELETE FROM pending_diffs WHERE session_id=?1", [sid])?;
        conn.execute("DELETE FROM file_snapshots WHERE session_id=?1", [sid])?;
        conn.execute("DELETE FROM role_state_snapshots WHERE session_id=?1", [sid])?;
        conn.execute("DELETE FROM message_images WHERE session_id=?1", [sid])?;
        conn.execute("DELETE FROM messages WHERE session_id=?1", [sid])?;
        conn.execute("DELETE FROM sessions WHERE id=?1", [sid])?;
    }
    let n = tables.len().max(1) as u64;
    let span = u64::from(band_end.saturating_sub(band_start));
    for (i, table) in tables.iter().enumerate() {
        let t_start = band_start.saturating_add(((span * i as u64) / n) as u8);
        let t_end = band_start.saturating_add(((span * (i as u64 + 1)) / n) as u8);
        let entry = format!("data/{table}.json");
        if !zip_entry_exists(path, &entry) {
            progress.span("table", t_start, t_end, 1, 1, &format!("{table} (skipped)"));
            continue;
        }
        let file = File::open(path)?;
        let mut zip =
            ZipArchive::new(file).map_err(|e| AppError::Other(format!("zip open: {e}")))?;
        let entry_r = zip
            .by_name(&entry)
            .map_err(|e| AppError::Other(format!("missing {entry}: {e}")))?;
        progress.span("table", t_start, t_end, 0, 1, table);
        let count = insert_table_rows_streaming(conn, table, entry_r, |rows| {
            let denom = (rows / 500).saturating_add(1);
            let within = u64::from(t_end.saturating_sub(t_start));
            let cur = within.saturating_mul(denom.saturating_sub(1)) / denom;
            progress.emit(
                "table_rows",
                t_start.saturating_add(cur as u8),
                rows,
                0,
                &format!("{table} · {rows}"),
            );
        })?;
        progress.span(
            "table",
            t_start,
            t_end,
            1,
            1,
            &format!("{table} ({count})"),
        );
    }
    Ok(())
}

fn zip_write_bytes<W: Write + std::io::Seek>(
    zip: &mut zip::ZipWriter<W>,
    opts: SimpleFileOptions,
    name: &str,
    bytes: &[u8],
) -> AppResult<()> {
    zip.start_file(name, opts)
        .map_err(|e| AppError::Other(format!("zip: {e}")))?;
    zip.write_all(bytes)?;
    Ok(())
}

fn add_sessions_dir_to_zip<W: Write + std::io::Seek>(
    zip: &mut zip::ZipWriter<W>,
    opts: SimpleFileOptions,
    sessions_root: &Path,
) -> AppResult<()> {
    add_sessions_subset_to_zip(zip, opts, sessions_root, None)
}

fn add_sessions_subset_to_zip<W: Write + std::io::Seek>(
    zip: &mut zip::ZipWriter<W>,
    opts: SimpleFileOptions,
    sessions_root: &Path,
    only_ids: Option<&[String]>,
) -> AppResult<()> {
    if !sessions_root.exists() {
        return Ok(());
    }
    let allow: Option<HashSet<&str>> =
        only_ids.map(|ids| ids.iter().map(|s| s.as_str()).collect());

    fn walk<W: Write + std::io::Seek>(
        zip: &mut zip::ZipWriter<W>,
        opts: SimpleFileOptions,
        sessions_root: &Path,
        dir: &Path,
        allow: &Option<HashSet<&str>>,
    ) -> AppResult<()> {
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                if path.parent() == Some(sessions_root) {
                    if let Some(set) = allow {
                        let name = path
                            .file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or("");
                        if !set.contains(name) {
                            continue;
                        }
                    }
                }
                walk(zip, opts, sessions_root, &path, allow)?;
                continue;
            }
            if !path.is_file() {
                continue;
            }
            let rel = path
                .strip_prefix(sessions_root)
                .map_err(|_| AppError::Other("sessions path strip failed".into()))?;
            let zip_path = format!(
                "sessions/{}",
                rel.to_string_lossy().replace('\\', "/")
            );
            zip_write_bytes(zip, opts, &zip_path, &fs::read(&path)?)?;
        }
        Ok(())
    }
    walk(zip, opts, sessions_root, sessions_root, &allow)
}

fn clear_dir_contents(dir: &Path) -> AppResult<()> {
    if !dir.exists() {
        fs::create_dir_all(dir)?;
        return Ok(());
    }
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let p = entry.path();
        if p.is_dir() {
            fs::remove_dir_all(&p)?;
        } else {
            fs::remove_file(&p)?;
        }
    }
    Ok(())
}

/// Extract session media from a backup archive.
/// When `only_ids` is `Some`, only those session folders are replaced (delta).
/// When `None`, the entire sessions directory is cleared then extracted.
fn extract_sessions_from_archive(
    archive_path: &Path,
    sessions_root: &Path,
    only_ids: Option<&[String]>,
    progress: &ProgressClock,
    band_start: u8,
    band_end: u8,
) -> AppResult<()> {
    let file = File::open(archive_path)?;
    let mut zip = ZipArchive::new(file).map_err(|e| AppError::Other(format!("zip open: {e}")))?;

    match only_ids {
        None => clear_dir_contents(sessions_root)?,
        Some(ids) => {
            fs::create_dir_all(sessions_root)?;
            for sid in ids {
                let dir = sessions_root.join(sid);
                if dir.exists() {
                    fs::remove_dir_all(&dir)?;
                }
            }
        }
    }

    let total_entries = zip.len() as u64;
    let mut media_total = 0u64;
    for i in 0..zip.len() {
        let entry = zip
            .by_index(i)
            .map_err(|e| AppError::Other(format!("zip entry: {e}")))?;
        let name = entry.name().replace('\\', "/");
        if let Some(rel) = name.strip_prefix("sessions/") {
            if !rel.is_empty() && !rel.ends_with('/') {
                if let Some(ids) = only_ids {
                    let top = rel.split('/').next().unwrap_or("");
                    if !ids.iter().any(|s| s == top) {
                        continue;
                    }
                }
                media_total += 1;
            }
        }
    }
    let media_total = media_total.max(1);

    let mut done = 0u64;
    for i in 0..zip.len() {
        let mut entry = zip
            .by_index(i)
            .map_err(|e| AppError::Other(format!("zip entry: {e}")))?;
        let name = entry.name().replace('\\', "/");
        let Some(rel) = name.strip_prefix("sessions/") else {
            continue;
        };
        if rel.is_empty() || rel.ends_with('/') {
            continue;
        }
        if rel.contains("..") {
            return Err(AppError::Invalid("unsafe path in backup archive".into()));
        }
        if let Some(ids) = only_ids {
            let top = rel.split('/').next().unwrap_or("");
            if !ids.iter().any(|s| s == top) {
                continue;
            }
        }
        let dest = sessions_root.join(rel.replace('/', std::path::MAIN_SEPARATOR_STR));
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut out = File::create(&dest)?;
        std::io::copy(&mut entry, &mut out)?;
        done += 1;
        if done == 1 || done % 10 == 0 || (i as u64 + 1) == total_entries {
            progress.span("media", band_start, band_end, done, media_total, rel);
        }
    }
    progress.span("media", band_start, band_end, 1, 1, "sessions");
    Ok(())
}

pub fn resolve_backup_dir(app: &AppHandle, settings: &Settings) -> AppResult<PathBuf> {
    let custom = settings.auto_backup_dir.trim();
    if custom.is_empty() {
        paths::backups_dir(app)
    } else {
        let dir = PathBuf::from(custom);
        fs::create_dir_all(&dir)?;
        Ok(dir)
    }
}

pub fn state_path(backup_dir: &Path) -> PathBuf {
    backup_dir.join(STATE_FILE)
}

pub fn read_state(backup_dir: &Path) -> BackupStateFile {
    let path = state_path(backup_dir);
    fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn write_state(backup_dir: &Path, state: &BackupStateFile) -> AppResult<()> {
    fs::create_dir_all(backup_dir)?;
    let path = state_path(backup_dir);
    let tmp = backup_dir.join(".backup-state.json.tmp");
    fs::write(&tmp, serde_json::to_string_pretty(state)?)?;
    fs::rename(&tmp, &path)?;
    Ok(())
}

fn module_dir(backup_dir: &Path, module: BackupModule) -> PathBuf {
    backup_dir.join(module.as_str())
}

fn default_dest_path(backup_dir: &Path, module: BackupModule, created_at: i64) -> PathBuf {
    let local = Local::now();
    let stamp = format!(
        "{}{:02}{:02}-{:02}{:02}{:02}",
        local.year(),
        local.month(),
        local.day(),
        local.hour(),
        local.minute(),
        local.second()
    );
    // created_at kept for uniqueness if clock ties
    let name = format!(
        "atelier-{}-{}-{}.zip",
        module.as_str(),
        stamp,
        created_at % 1000
    );
    module_dir(backup_dir, module).join(name)
}

pub fn prune_module_backups(backup_dir: &Path, module: BackupModule, keep: usize) -> AppResult<()> {
    if keep == 0 {
        return Ok(());
    }
    let dir = module_dir(backup_dir, module);
    if !dir.exists() {
        return Ok(());
    }
    let mut files: Vec<(PathBuf, std::time::SystemTime)> = fs::read_dir(&dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.extension()
                .and_then(|x| x.to_str())
                .map(|x| x.eq_ignore_ascii_case("zip"))
                .unwrap_or(false)
        })
        .filter_map(|p| {
            let meta = fs::metadata(&p).ok()?;
            let modified = meta.modified().ok()?;
            Some((p, modified))
        })
        .collect();
    files.sort_by(|a, b| b.1.cmp(&a.1));
    for (path, _) in files.into_iter().skip(keep) {
        let _ = fs::remove_file(path);
    }
    Ok(())
}

fn create_backup_inner(
    app: &AppHandle,
    pool: &DbPool,
    module: BackupModule,
    kind: BackupKind,
    dest_path: Option<&str>,
) -> AppResult<BackupResult> {
    let conn = pool.get()?;
    let _ = conn.execute_batch("PRAGMA wal_checkpoint(PASSIVE);");

    let created_at = now_ms();
    let settings = settings::read(&conn)?;
    let backup_dir = resolve_backup_dir(app, &settings)?;

    // Auto chat backups are delta-only: only dirty sessions since last success.
    let (scope, since_ms, session_ids) = if module == BackupModule::Chat && kind == BackupKind::Auto
    {
        let state = read_state(&backup_dir);
        let since = state.last_chat_at_ms.unwrap_or(0);
        let ids = changed_session_ids(&conn, since)?;
        if ids.is_empty() {
            return Ok(BackupResult {
                path: String::new(),
                module,
                created_at,
                kind,
                skipped: true,
                scope: BackupScope::Delta,
                session_count: 0,
            });
        }
        (BackupScope::Delta, Some(since), ids)
    } else {
        (BackupScope::Full, None, Vec::new())
    };

    let dest = match dest_path {
        Some(p) if !p.trim().is_empty() => PathBuf::from(p),
        _ => {
            fs::create_dir_all(module_dir(&backup_dir, module))?;
            default_dest_path(&backup_dir, module, created_at)
        }
    };
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)?;
    }

    let tables = module.tables();
    let manifest = BackupManifest {
        format: MANIFEST_FORMAT.into(),
        version: MANIFEST_VERSION.into(),
        module,
        kind,
        created_at,
        app_version: env!("CARGO_PKG_VERSION").into(),
        schema_version: schema_version(&conn)?,
        tables: tables.iter().map(|t| (*t).to_string()).collect(),
        scope,
        since_ms,
        session_ids: session_ids.clone(),
    };

    let file = File::create(&dest)?;
    let mut zip = zip::ZipWriter::new(file);
    let opts = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);

    zip_write_bytes(
        &mut zip,
        opts,
        "manifest.json",
        serde_json::to_string_pretty(&manifest)?.as_bytes(),
    )?;

    let progress = ProgressClock::new(app, "backup");
    progress.emit("preparing", 1, 0, 1, "manifest");

    let has_media = module.includes_sessions_media()
        && !(scope == BackupScope::Delta && module == BackupModule::Chat && session_ids.is_empty());
    // Tables: 2–88% (or 2–98% without media). Media: 88–98%. Done: 100%.
    let tables_end: u8 = if has_media || (scope == BackupScope::Delta && module == BackupModule::Chat)
    {
        88
    } else {
        98
    };

    let row_counts: Vec<u64> = tables
        .iter()
        .map(|t| {
            if scope == BackupScope::Delta && module == BackupModule::Chat {
                // Approximate: full table count is fine for weighting; delta may be smaller.
                table_row_count(&conn, t).max(1)
            } else {
                table_row_count(&conn, t).max(1)
            }
        })
        .collect();
    let total_weight: u64 = row_counts.iter().sum::<u64>().max(1);
    let mut completed_weight = 0u64;
    let tables_span = u64::from(tables_end.saturating_sub(2));

    if scope == BackupScope::Delta && module == BackupModule::Chat {
        for (i, table) in tables.iter().enumerate() {
            let weight = row_counts[i];
            let start = 2u8.saturating_add(((tables_span * completed_weight) / total_weight) as u8);
            let end =
                2u8.saturating_add(((tables_span * (completed_weight + weight)) / total_weight) as u8);
            progress.span("table", start, end, 0, 1, table);
            let expected = weight.max(1);
            let n = zip_write_chat_table_for_sessions(
                &mut zip,
                opts,
                &conn,
                table,
                &session_ids,
                |rows, _| {
                    progress.span(
                        "table_rows",
                        start,
                        end,
                        rows.min(expected),
                        expected,
                        &format!("{table} · {rows}"),
                    );
                },
            )?;
            completed_weight += weight;
            progress.span(
                "table",
                start,
                end,
                1,
                1,
                &format!("{table} ({n})"),
            );
        }
        progress.span("media", 88, 98, 0, 1, "sessions");
        let sessions_root = paths::sessions_dir(app)?;
        add_sessions_subset_to_zip(&mut zip, opts, &sessions_root, Some(&session_ids))?;
        progress.span("media", 88, 98, 1, 1, "sessions");
    } else {
        for (i, table) in tables.iter().enumerate() {
            let weight = row_counts[i];
            let start = 2u8.saturating_add(((tables_span * completed_weight) / total_weight) as u8);
            let end =
                2u8.saturating_add(((tables_span * (completed_weight + weight)) / total_weight) as u8);
            progress.span("table", start, end, 0, 1, table);
            let expected = weight.max(1);
            let n = zip_write_full_table(&mut zip, opts, &conn, table, |rows, _| {
                progress.span(
                    "table_rows",
                    start,
                    end,
                    rows.min(expected),
                    expected,
                    &format!("{table} · {rows}"),
                );
            })?;
            completed_weight += weight;
            progress.span(
                "table",
                start,
                end,
                1,
                1,
                &format!("{table} ({n})"),
            );
        }
        if module.includes_sessions_media() {
            progress.span("media", 88, 98, 0, 1, "sessions");
            let sessions_root = paths::sessions_dir(app)?;
            add_sessions_dir_to_zip(&mut zip, opts, &sessions_root)?;
            progress.span("media", 88, 98, 1, 1, "sessions");
        }
    }

    zip.finish()
        .map_err(|e| AppError::Other(format!("zip finish: {e}")))?;

    progress.done();
    Ok(BackupResult {
        path: paths::display_path(&dest),
        module,
        created_at,
        kind,
        skipped: false,
        scope,
        session_count: if scope == BackupScope::Delta {
            session_ids.len()
        } else {
            0
        },
    })
}

pub fn create_backup(
    app: &AppHandle,
    pool: &DbPool,
    module: BackupModule,
    kind: BackupKind,
    dest_path: Option<&str>,
) -> AppResult<BackupResult> {
    let _guard = BACKUP_LOCK
        .lock()
        .map_err(|_| AppError::Other("backup lock poisoned".into()))?;
    create_backup_inner(app, pool, module, kind, dest_path)
}

fn read_zip_entry(archive_path: &Path, name: &str) -> AppResult<Vec<u8>> {
    let file = File::open(archive_path)?;
    let mut zip = ZipArchive::new(file).map_err(|e| AppError::Other(format!("zip open: {e}")))?;
    let mut entry = zip
        .by_name(name)
        .map_err(|e| AppError::Other(format!("missing {name} in archive: {e}")))?;
    let mut buf = Vec::new();
    entry.read_to_end(&mut buf)?;
    Ok(buf)
}

fn read_manifest(archive_path: &Path) -> AppResult<BackupManifest> {
    let bytes = read_zip_entry(archive_path, "manifest.json")?;
    let manifest: BackupManifest = serde_json::from_slice(&bytes)?;
    if manifest.format != MANIFEST_FORMAT {
        return Err(AppError::Invalid(format!(
            "not an atelier backup (format={})",
            manifest.format
        )));
    }
    if manifest.version != MANIFEST_VERSION {
        return Err(AppError::Invalid(format!(
            "unsupported backup version {}",
            manifest.version
        )));
    }
    Ok(manifest)
}

fn restore_backup_inner(
    app: &AppHandle,
    pool: &DbPool,
    archive_path: &str,
) -> AppResult<RestoreResult> {
    let path = PathBuf::from(archive_path);
    if !path.is_file() {
        return Err(AppError::NotFound(format!("backup not found: {archive_path}")));
    }
    let progress = ProgressClock::new(app, "restore");
    progress.emit("preparing", 1, 0, 1, "manifest");
    let manifest = read_manifest(&path)?;
    let module = manifest.module;
    let tables = module.tables();
    let is_delta = manifest.scope == BackupScope::Delta && module == BackupModule::Chat;
    let mut warnings: Vec<String> = Vec::new();

    // Keep huge usage tables out of the core transaction so projects/sessions
    // still land if usage restore OOMs or fails.
    let (core_tables, usage_tables): (Vec<&str>, Vec<&str>) = tables
        .iter()
        .copied()
        .partition(|t| !USAGE_TABLES.contains(t));

    let has_usage = !usage_tables.is_empty() && !is_delta;
    let has_media = module.includes_sessions_media();
    // Bands: core 2–70, usage 70–82, index 82–88, media 88–98, done 100.
    let core_end: u8 = if has_usage {
        70
    } else if has_media {
        82
    } else {
        90
    };

    {
        let conn = pool.get()?;
        let _ = conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);");
    }

    let conn = pool.get()?;
    conn.execute_batch("PRAGMA foreign_keys = OFF;")?;
    conn.execute_batch("BEGIN IMMEDIATE;")?;
    let restore_result = (|| -> AppResult<()> {
        if is_delta {
            let ids = if manifest.session_ids.is_empty() {
                let bytes = read_zip_entry(&path, "data/sessions.json")?;
                let rows: Value = serde_json::from_slice(&bytes)?;
                rows.as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|r| r.get("id").and_then(|v| v.as_str()).map(|s| s.to_string()))
                    .collect::<Vec<_>>()
            } else {
                manifest.session_ids.clone()
            };
            restore_chat_delta(&conn, &ids, &core_tables, &path, &progress, 2, core_end)?;
        } else {
            let n = core_tables.len().max(1) as u64;
            let span = u64::from(core_end.saturating_sub(2));
            for (i, table) in core_tables.iter().enumerate() {
                let t_start = 2u8.saturating_add(((span * i as u64) / n) as u8);
                let t_end = 2u8.saturating_add(((span * (i as u64 + 1)) / n) as u8);
                restore_table_streaming(&conn, &path, table, &progress, t_start, t_end)?;
            }
        }
        Ok(())
    })();

    match restore_result {
        Ok(()) => {
            conn.execute_batch("COMMIT;")?;
            let _ = conn.execute_batch("PRAGMA foreign_keys = ON;");
        }
        Err(e) => {
            let _ = conn.execute_batch("ROLLBACK;");
            let _ = conn.execute_batch("PRAGMA foreign_keys = ON;");
            return Err(e);
        }
    }

    // Usage tables: separate transaction — failure must not wipe core restore.
    if has_usage {
        let usage_conn = pool.get()?;
        usage_conn.execute_batch("PRAGMA foreign_keys = OFF;")?;
        usage_conn.execute_batch("BEGIN IMMEDIATE;")?;
        let usage_ok = (|| -> AppResult<()> {
            let n = usage_tables.len().max(1) as u64;
            let span = u64::from(82u8.saturating_sub(70));
            for (i, table) in usage_tables.iter().enumerate() {
                let t_start = 70u8.saturating_add(((span * i as u64) / n) as u8);
                let t_end = 70u8.saturating_add(((span * (i as u64 + 1)) / n) as u8);
                restore_table_streaming(&usage_conn, &path, table, &progress, t_start, t_end)?;
            }
            Ok(())
        })();
        match usage_ok {
            Ok(()) => {
                usage_conn.execute_batch("COMMIT;")?;
                let _ = usage_conn.execute_batch("PRAGMA foreign_keys = ON;");
            }
            Err(e) => {
                let _ = usage_conn.execute_batch("ROLLBACK;");
                let _ = usage_conn.execute_batch("PRAGMA foreign_keys = ON;");
                progress.emit("table", 82, 0, 1, "usage skipped");
                warnings.push(format!(
                    "usage data restore skipped: {e} (projects/sessions were restored)"
                ));
            }
        }
    }

    progress.emit("index", 85, 0, 1, "search");
    {
        let index_conn = pool.get()?;
        if let Err(e) = message_search::backfill_search_index(&index_conn) {
            warnings.push(format!("search index rebuild failed: {e}"));
        }
    }
    progress.emit("index", 88, 1, 1, "search");

    if has_media {
        let sessions_root = paths::sessions_dir(app)?;
        if is_delta {
            let ids = if !manifest.session_ids.is_empty() {
                manifest.session_ids.clone()
            } else {
                session_ids_in_archive(&path)?
            };
            extract_sessions_from_archive(&path, &sessions_root, Some(&ids), &progress, 88, 98)?;
        } else {
            extract_sessions_from_archive(&path, &sessions_root, None, &progress, 88, 98)?;
        }
    }

    progress.done();
    Ok(RestoreResult {
        module,
        path: paths::display_path(&path),
        requires_restart: true,
        warnings,
    })
}

fn session_ids_in_archive(archive_path: &Path) -> AppResult<Vec<String>> {
    let file = File::open(archive_path)?;
    let mut zip = ZipArchive::new(file).map_err(|e| AppError::Other(format!("zip open: {e}")))?;
    let mut tops: HashSet<String> = HashSet::new();
    for i in 0..zip.len() {
        let entry = zip
            .by_index(i)
            .map_err(|e| AppError::Other(format!("zip entry: {e}")))?;
        let name = entry.name().replace('\\', "/");
        if let Some(rel) = name.strip_prefix("sessions/") {
            if let Some(top) = rel.split('/').next() {
                if !top.is_empty() {
                    tops.insert(top.to_string());
                }
            }
        }
    }
    if tops.is_empty() {
        // Fall back to sessions.json
        let bytes = read_zip_entry(archive_path, "data/sessions.json")?;
        let rows: Value = serde_json::from_slice(&bytes)?;
        if let Some(arr) = rows.as_array() {
            for r in arr {
                if let Some(id) = r.get("id").and_then(|v| v.as_str()) {
                    tops.insert(id.to_string());
                }
            }
        }
    }
    Ok(tops.into_iter().collect())
}

pub fn restore_backup(
    app: &AppHandle,
    pool: &DbPool,
    archive_path: &str,
) -> AppResult<RestoreResult> {
    let _guard = BACKUP_LOCK
        .lock()
        .map_err(|_| AppError::Other("backup lock poisoned".into()))?;
    restore_backup_inner(app, pool, archive_path)
}

pub fn list_backups(
    app: &AppHandle,
    pool: &DbPool,
    module_filter: Option<BackupModule>,
) -> AppResult<Vec<BackupListItem>> {
    let conn = pool.get()?;
    let settings = settings::read(&conn)?;
    let backup_dir = resolve_backup_dir(app, &settings)?;
    let modules: Vec<BackupModule> = match module_filter {
        Some(m) => vec![m],
        None => vec![
            BackupModule::Full,
            BackupModule::Config,
            BackupModule::Chat,
            BackupModule::Usage,
        ],
    };

    let mut items = Vec::new();
    for module in modules {
        let dir = module_dir(&backup_dir, module);
        if !dir.exists() {
            continue;
        }
        for entry in fs::read_dir(&dir)? {
            let entry = entry?;
            let path = entry.path();
            if !path
                .extension()
                .and_then(|x| x.to_str())
                .map(|x| x.eq_ignore_ascii_case("zip"))
                .unwrap_or(false)
            {
                continue;
            }
            let meta = fs::metadata(&path)?;
            let (kind, created_at, modu) = match read_manifest(&path) {
                Ok(m) => (m.kind, m.created_at, m.module),
                Err(_) => {
                    let created = meta
                        .modified()
                        .ok()
                        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                        .map(|d| d.as_millis() as i64)
                        .unwrap_or(0);
                    (BackupKind::Manual, created, module)
                }
            };
            items.push(BackupListItem {
                path: paths::display_path(&path),
                module: modu,
                kind,
                created_at,
                size_bytes: meta.len(),
            });
        }
    }
    items.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    Ok(items)
}

pub fn is_busy() -> bool {
    BACKUP_LOCK.try_lock().is_err()
}

pub fn most_recent_due_config_slot(now: chrono::DateTime<Local>) -> String {
    let today = now.date_naive();
    let mut candidates: Vec<(chrono::NaiveDateTime, String)> = Vec::new();
    for day_offset in 0..=1 {
        let day = today - chrono::Duration::days(day_offset);
        for t in CONFIG_SLOT_TIMES {
            let time = NaiveTime::parse_from_str(t, "%H:%M").unwrap_or(NaiveTime::MIN);
            let dt = day.and_time(time);
            if dt <= now.naive_local() {
                candidates.push((dt, format!("{}T{t}", day.format("%Y-%m-%d"))));
            }
        }
    }
    candidates.sort_by(|a, b| b.0.cmp(&a.0));
    candidates
        .into_iter()
        .next()
        .map(|(_, s)| s)
        .unwrap_or_else(|| {
            let day = today;
            format!("{}T00:00", day.format("%Y-%m-%d"))
        })
}

pub fn next_config_slot_after(now: chrono::DateTime<Local>) -> String {
    let today = now.date_naive();
    for day_offset in 0..=2 {
        let day = today + chrono::Duration::days(day_offset);
        let mut times: Vec<_> = CONFIG_SLOT_TIMES.to_vec();
        times.sort();
        for t in times {
            let time = NaiveTime::parse_from_str(t, "%H:%M").unwrap_or(NaiveTime::MIN);
            let dt = day.and_time(time);
            if dt > now.naive_local() {
                return format!("{}T{t}", day.format("%Y-%m-%d"));
            }
        }
    }
    format!("{}T06:00", (today + chrono::Duration::days(1)).format("%Y-%m-%d"))
}

pub fn get_status(app: &AppHandle, pool: &DbPool) -> AppResult<BackupStatus> {
    let conn = pool.get()?;
    let settings = settings::read(&conn)?;
    let backup_dir = resolve_backup_dir(app, &settings)?;
    let state = read_state(&backup_dir);
    let now = Local::now();
    let interval_ms = settings.auto_backup_chat_interval_minutes.max(1) * 60_000;
    let next_chat = state
        .last_chat_at_ms
        .map(|t| t + interval_ms)
        .or_else(|| Some(now_ms()));

    Ok(BackupStatus {
        enabled: settings.auto_backup_enabled,
        backup_dir: paths::display_path(&backup_dir),
        busy: is_busy(),
        config_times: CONFIG_SLOT_TIMES.iter().map(|s| (*s).to_string()).collect(),
        chat_interval_minutes: settings.auto_backup_chat_interval_minutes,
        config_keep: settings.auto_backup_config_keep,
        chat_keep: settings.auto_backup_chat_keep,
        last_config_slot: state.last_config_slot,
        last_chat_at_ms: state.last_chat_at_ms,
        last_usage_slot: state.last_usage_slot,
        last_full_at_ms: state.last_full_at_ms,
        last_success_at_ms: state.last_success_at_ms,
        last_error: state.last_error,
        next_config_slot: Some(next_config_slot_after(now)),
        next_chat_at_ms: next_chat,
    })
}

/// Run due auto backups. Returns how many modules were backed up.
pub fn run_scheduler_tick(app: &AppHandle, pool: &DbPool) -> AppResult<usize> {
    let conn = pool.get()?;
    let settings = settings::read(&conn)?;
    if !settings.auto_backup_enabled {
        return Ok(0);
    }
    drop(conn);

    let backup_dir = resolve_backup_dir(app, &settings)?;
    let mut state = read_state(&backup_dir);
    let now = Local::now();
    let now_ms_v = now_ms();
    let mut ran = 0usize;

    let due_slot = most_recent_due_config_slot(now);
    let need_config = state
        .last_config_slot
        .as_ref()
        .map(|s| s.as_str() < due_slot.as_str())
        .unwrap_or(true);

    if need_config {
        match create_backup(app, pool, BackupModule::Config, BackupKind::Auto, None) {
            Ok(_) => {
                state.last_config_slot = Some(due_slot.clone());
                state.last_success_at_ms = Some(now_ms_v);
                state.last_error = None;
                ran += 1;
                let keep = settings.auto_backup_config_keep.max(1) as usize;
                let _ = prune_module_backups(&backup_dir, BackupModule::Config, keep);
            }
            Err(e) => {
                state.last_error = Some(format!("config: {e}"));
                let _ = write_state(&backup_dir, &state);
                return Err(e);
            }
        }
        match create_backup(app, pool, BackupModule::Usage, BackupKind::Auto, None) {
            Ok(_) => {
                state.last_usage_slot = Some(due_slot);
                state.last_success_at_ms = Some(now_ms_v);
                ran += 1;
                let keep = settings.auto_backup_config_keep.max(1) as usize;
                let _ = prune_module_backups(&backup_dir, BackupModule::Usage, keep);
            }
            Err(e) => {
                state.last_error = Some(format!("usage: {e}"));
                let _ = write_state(&backup_dir, &state);
                return Err(e);
            }
        }
    }

    let interval_ms = settings.auto_backup_chat_interval_minutes.max(1) * 60_000;
    let need_chat = state
        .last_chat_at_ms
        .map(|t| now_ms_v - t >= interval_ms)
        .unwrap_or(true);
    if need_chat {
        match create_backup(app, pool, BackupModule::Chat, BackupKind::Auto, None) {
            Ok(result) => {
                // Advance watermark even when skipped (no dirty sessions).
                state.last_chat_at_ms = Some(now_ms_v);
                state.last_error = None;
                if !result.skipped {
                    state.last_success_at_ms = Some(now_ms_v);
                    ran += 1;
                    let keep = settings.auto_backup_chat_keep.max(1) as usize;
                    let _ = prune_module_backups(&backup_dir, BackupModule::Chat, keep);
                }
            }
            Err(e) => {
                state.last_error = Some(format!("chat: {e}"));
                let _ = write_state(&backup_dir, &state);
                return Err(e);
            }
        }
    }

    write_state(&backup_dir, &state)?;
    Ok(ran)
}

/// Record a successful manual full backup into state.
pub fn record_full_success(app: &AppHandle, pool: &DbPool, created_at: i64) -> AppResult<()> {
    let conn = pool.get()?;
    let settings = settings::read(&conn)?;
    let backup_dir = resolve_backup_dir(app, &settings)?;
    let mut state = read_state(&backup_dir);
    state.last_full_at_ms = Some(created_at);
    state.last_success_at_ms = Some(created_at);
    state.last_error = None;
    write_state(&backup_dir, &state)
}

#[allow(dead_code)]
pub fn with_conn_checkpoint(conn: &DbConn) -> AppResult<()> {
    let _ = conn.execute_batch("PRAGMA wal_checkpoint(PASSIVE);");
    Ok(())
}
