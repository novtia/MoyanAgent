//! Searchable text extraction + FTS5 trigram index maintenance.

use rusqlite::{params, Connection};
use serde::Serialize;
use serde_json::Value;

use crate::error::{AppError, AppResult};

const MAX_SEGMENT_BYTES: usize = 8 * 1024;
const SNIPPET_RADIUS: usize = 40;
const SNIPPET_MAX: usize = 120;

/// Escape user input for an FTS5 phrase query (`"…"`).
pub fn escape_fts_query(raw: &str) -> String {
    let escaped = raw.replace('"', "\"\"");
    format!("\"{escaped}\"")
}

pub fn escape_like(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for ch in raw.chars() {
        if matches!(ch, '\\' | '%' | '_') {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

/// Unicode char count — trigram MATCH needs ≥3; shorter queries use LIKE.
pub fn query_char_len(q: &str) -> usize {
    q.chars().count()
}

fn truncate_segment(s: &str) -> String {
    if s.len() <= MAX_SEGMENT_BYTES {
        return s.to_string();
    }
    let mut end = MAX_SEGMENT_BYTES;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &s[..end])
}

fn value_to_search_str(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        other => serde_json::to_string(other).unwrap_or_default(),
    }
}

/// Concatenate searchable fields: text → thinking_content → blocks thinking/text → tool.
pub fn build_searchable_text(text: Option<&str>, params: &Value) -> String {
    let mut parts: Vec<String> = Vec::new();

    if let Some(t) = text {
        let t = t.trim();
        if !t.is_empty() {
            parts.push(truncate_segment(t));
        }
    }

    if let Some(tc) = params
        .get("thinking_content")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        parts.push(truncate_segment(tc));
    }

    if let Some(blocks) = params.get("blocks").and_then(|v| v.as_array()) {
        for block in blocks {
            let ty = block.get("type").and_then(|v| v.as_str()).unwrap_or("");
            match ty {
                "thinking" | "text" => {
                    if let Some(c) = block
                        .get("content")
                        .and_then(|v| v.as_str())
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                    {
                        parts.push(truncate_segment(c));
                    }
                }
                "tool_use" => {
                    if let Some(tool) = block
                        .get("tool")
                        .and_then(|v| v.as_str())
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                    {
                        parts.push(tool.to_string());
                    }
                    for key in ["input", "output"] {
                        if let Some(v) = block.get(key) {
                            let s = value_to_search_str(v);
                            let s = s.trim();
                            if !s.is_empty() {
                                parts.push(truncate_segment(s));
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }

    parts.join("\n")
}

fn contains_ci(hay: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return false;
    }
    hay.to_lowercase().contains(&needle.to_lowercase())
}

/// Which logical fields matched `query` (for hit list tags).
pub fn match_fields_for(text: Option<&str>, params: &Value, query: &str) -> Vec<String> {
    let q = query.trim();
    if q.is_empty() {
        return Vec::new();
    }
    let mut fields = Vec::new();

    if text.map(|t| contains_ci(t, q)).unwrap_or(false) {
        fields.push("text".into());
    }

    let mut thinking_hit = params
        .get("thinking_content")
        .and_then(|v| v.as_str())
        .map(|s| contains_ci(s, q))
        .unwrap_or(false);

    let mut tool_hit = false;
    if let Some(blocks) = params.get("blocks").and_then(|v| v.as_array()) {
        for block in blocks {
            let ty = block.get("type").and_then(|v| v.as_str()).unwrap_or("");
            match ty {
                "thinking" => {
                    if !thinking_hit {
                        if let Some(c) = block.get("content").and_then(|v| v.as_str()) {
                            if contains_ci(c, q) {
                                thinking_hit = true;
                            }
                        }
                    }
                }
                "text" => {
                    if !fields.iter().any(|f| f == "text") {
                        if let Some(c) = block.get("content").and_then(|v| v.as_str()) {
                            if contains_ci(c, q) {
                                fields.push("text".into());
                            }
                        }
                    }
                }
                "tool_use" => {
                    if tool_hit {
                        continue;
                    }
                    if let Some(tool) = block.get("tool").and_then(|v| v.as_str()) {
                        if contains_ci(tool, q) {
                            tool_hit = true;
                            continue;
                        }
                    }
                    for key in ["input", "output"] {
                        if let Some(v) = block.get(key) {
                            let s = value_to_search_str(v);
                            if contains_ci(&s, q) {
                                tool_hit = true;
                                break;
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }

    if thinking_hit {
        fields.push("thinking".into());
    }
    if tool_hit {
        fields.push("tool".into());
    }
    fields
}

pub fn make_snippet(body: &str, query: &str) -> String {
    let compact: String = body.split_whitespace().collect::<Vec<_>>().join(" ");
    if compact.is_empty() {
        return String::new();
    }
    let needle = query.trim();
    if needle.is_empty() || compact.chars().count() <= SNIPPET_MAX {
        return if compact.chars().count() <= SNIPPET_MAX {
            compact
        } else {
            compact.chars().take(SNIPPET_MAX).collect::<String>() + "…"
        };
    }

    let lower = compact.to_lowercase();
    let needle_lower = needle.to_lowercase();
    let byte_idx = lower.find(&needle_lower);
    let start_char = match byte_idx {
        Some(bi) => compact[..bi].chars().count().saturating_sub(SNIPPET_RADIUS),
        None => 0,
    };
    let chars: Vec<char> = compact.chars().collect();
    let end = (start_char + SNIPPET_MAX).min(chars.len());
    let slice: String = chars[start_char..end].iter().collect();
    let prefix = if start_char > 0 { "…" } else { "" };
    let suffix = if end < chars.len() { "…" } else { "" };
    format!("{prefix}{slice}{suffix}")
}

fn sync_message_fts(conn: &Connection, rowid: i64, old_text: &str, new_text: &str) -> AppResult<()> {
    // External-content FTS: delete old row (ignore if absent), then insert.
    let _ = conn.execute(
        "INSERT INTO messages_fts(messages_fts, rowid, searchable_text) VALUES('delete', ?1, ?2)",
        params![rowid, old_text],
    );
    conn.execute(
        "INSERT INTO messages_fts(rowid, searchable_text) VALUES(?1, ?2)",
        params![rowid, new_text],
    )?;
    Ok(())
}

fn sync_session_fts(conn: &Connection, rowid: i64, old_title: &str, new_title: &str) -> AppResult<()> {
    let _ = conn.execute(
        "INSERT INTO sessions_fts(sessions_fts, rowid, title) VALUES('delete', ?1, ?2)",
        params![rowid, old_title],
    );
    conn.execute(
        "INSERT INTO sessions_fts(rowid, title) VALUES(?1, ?2)",
        params![rowid, new_title],
    )?;
    Ok(())
}

/// Recompute `searchable_text` + FTS row for one message.
pub fn reindex_message(conn: &Connection, message_id: &str) -> AppResult<()> {
    let (rowid, text, params_json, old_searchable): (i64, Option<String>, Option<String>, String) =
        conn.query_row(
            "SELECT rowid, text, params_json, searchable_text FROM messages WHERE id=?1",
            params![message_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .map_err(|_| AppError::NotFound(format!("message {message_id}")))?;

    let params_v: Value = params_json
        .as_deref()
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or(Value::Null);
    let searchable = build_searchable_text(text.as_deref(), &params_v);

    conn.execute(
        "UPDATE messages SET searchable_text=?1 WHERE id=?2",
        params![&searchable, message_id],
    )?;
    sync_message_fts(conn, rowid, &old_searchable, &searchable)?;
    Ok(())
}

/// Index a newly inserted session (call after INSERT into sessions).
pub fn index_new_session(conn: &Connection, session_id: &str) -> AppResult<()> {
    let (rowid, title): (i64, String) = conn.query_row(
        "SELECT rowid, title FROM sessions WHERE id=?1",
        params![session_id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    conn.execute(
        "INSERT INTO sessions_fts(rowid, title) VALUES(?1, ?2)",
        params![rowid, title],
    )?;
    Ok(())
}

/// Index a newly inserted message (call after INSERT with searchable_text set).
pub fn index_new_message(conn: &Connection, message_id: &str) -> AppResult<()> {
    let (rowid, searchable): (i64, String) = conn.query_row(
        "SELECT rowid, searchable_text FROM messages WHERE id=?1",
        params![message_id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    conn.execute(
        "INSERT INTO messages_fts(rowid, searchable_text) VALUES(?1, ?2)",
        params![rowid, searchable],
    )?;
    Ok(())
}

/// Full rebuild after schema migration (or repair).
pub fn backfill_search_index(conn: &Connection) -> AppResult<()> {
    let mut stmt = conn.prepare("SELECT id, text, params_json FROM messages")?;
    let rows = stmt.query_map(params![], |r| {
        let id: String = r.get(0)?;
        let text: Option<String> = r.get(1)?;
        let params_json: Option<String> = r.get(2)?;
        Ok((id, text, params_json))
    })?;

    let mut updates: Vec<(String, String)> = Vec::new();
    for row in rows {
        let (id, text, params_json) = row?;
        let params_v: Value = params_json
            .as_deref()
            .and_then(|s| serde_json::from_str(s).ok())
            .unwrap_or(Value::Null);
        let searchable = build_searchable_text(text.as_deref(), &params_v);
        updates.push((id, searchable));
    }
    drop(stmt);

    for (id, searchable) in updates {
        conn.execute(
            "UPDATE messages SET searchable_text=?1 WHERE id=?2",
            params![searchable, id],
        )?;
    }

    conn.execute_batch(
        "INSERT INTO messages_fts(messages_fts) VALUES('rebuild');
         INSERT INTO sessions_fts(sessions_fts) VALUES('rebuild');",
    )?;
    Ok(())
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionSearchHit {
    pub message_id: String,
    pub role: String,
    pub created_at: i64,
    pub snippet: String,
    pub match_fields: Vec<String>,
}

/// All message hits within one session, oldest-first (chat find navigation).
pub fn search_session_hits(
    conn: &Connection,
    session_id: &str,
    query: &str,
    limit: i64,
) -> AppResult<Vec<SessionSearchHit>> {
    let limit = if limit <= 0 { 200 } else { limit.min(500) };
    let query = query.trim();
    if query.is_empty() {
        return Ok(Vec::new());
    }

    let use_fts = query_char_len(query) >= 3;
    let mut hits = Vec::new();

    if use_fts {
        let fts_q = escape_fts_query(query);
        let mut stmt = conn.prepare(
            "SELECT m.id, m.role, m.created_at, m.searchable_text, m.text, m.params_json
             FROM messages m
             JOIN messages_fts ON messages_fts.rowid = m.rowid
             WHERE m.session_id = ?1 AND messages_fts MATCH ?2
             ORDER BY m.created_at ASC
             LIMIT ?3",
        )?;
        let rows = stmt.query_map(params![session_id, fts_q, limit], map_hit_row)?;
        for r in rows {
            hits.push(r?);
        }
    } else {
        let pattern = format!("%{}%", escape_like(query));
        let mut stmt = conn.prepare(
            "SELECT m.id, m.role, m.created_at, m.searchable_text, m.text, m.params_json
             FROM messages m
             WHERE m.session_id = ?1
               AND COALESCE(m.searchable_text, '') LIKE ?2 ESCAPE '\\'
             ORDER BY m.created_at ASC
             LIMIT ?3",
        )?;
        let rows = stmt.query_map(params![session_id, pattern, limit], map_hit_row)?;
        for r in rows {
            hits.push(r?);
        }
    }

    let mut out = Vec::with_capacity(hits.len());
    for (id, role, created_at, searchable, text, params_json) in hits {
        let params_v: Value = params_json
            .as_deref()
            .and_then(|s| serde_json::from_str(s).ok())
            .unwrap_or(Value::Null);
        let fields = match_fields_for(text.as_deref(), &params_v, query);
        let snippet = make_snippet(&searchable, query);
        out.push(SessionSearchHit {
            message_id: id,
            role,
            created_at,
            snippet,
            match_fields: fields,
        });
    }
    Ok(out)
}

type HitRow = (
    String,
    String,
    i64,
    String,
    Option<String>,
    Option<String>,
);

fn map_hit_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<HitRow> {
    Ok((
        r.get(0)?,
        r.get(1)?,
        r.get(2)?,
        r.get(3)?,
        r.get(4)?,
        r.get(5)?,
    ))
}

/// Rename helper: update FTS using known old/new titles.
pub fn update_session_title_fts(
    conn: &Connection,
    session_id: &str,
    old_title: &str,
    new_title: &str,
) -> AppResult<()> {
    let rowid: i64 = conn.query_row(
        "SELECT rowid FROM sessions WHERE id=?1",
        params![session_id],
        |r| r.get(0),
    )?;
    sync_session_fts(conn, rowid, old_title, new_title)
}
