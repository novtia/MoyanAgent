//! Session content log: one JSON file per session for development debugging.
//!
//! This module is intentionally separate from [`crate::ai::token_log`]:
//!
//! - **Token stats** ([`crate::ai::token_log`]) records *numbers* into SQLite
//!   for analytics/billing.
//! - **Session log** (this module) records the *content* a session produced
//!   — system settings, every message, every tool call/result, and errors —
//!   into a local JSON file per session, so a developer can replay exactly
//!   what happened while debugging.
//!
//! Each session gets `Documents/MoYanAgent/logs/{session_id}.json`: a single
//! pretty-printed JSON array of [`SessionLogEntry`]. Entries are kept lean —
//! only the *new* content each step produced, never a replay of the whole
//! prompt/history (that lives once in the `session_settings` entry and in the
//! per-message / per-tool entries).

use std::fs::{self};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use ulid::Ulid;

use crate::ai::agent::tools::ToolResult;
use crate::ai::chat::GenerateResponse;
use crate::ai::token_log::LogContext;
use crate::ai::tokens::TokenUsage;
use crate::data::db::now_ms;
use crate::data::token_log::{rollback_scope_for_message, TokenLogRollbackScope};

pub const KIND_SETTINGS: &str = "session_settings";
pub const KIND_CONTEXT: &str = "context";
pub const KIND_USER_MESSAGE: &str = "user_message";
pub const KIND_ASSISTANT_TURN: &str = "assistant_turn";
pub const KIND_TOOL_CALL: &str = "tool_call";
pub const KIND_TURN_SUMMARY: &str = "turn_summary";
pub const KIND_ERROR: &str = "error";

const MAX_LOG_CHARS: usize = 16_384;

/// One element in a session log file. `data` carries the kind-specific payload;
/// the flat fields exist so rollback can filter entries without parsing `data`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionLogEntry {
    pub id: String,
    /// Creation time in ms (mirrors `created_at` used by rollback scope).
    pub ts: i64,
    pub kind: String,
    pub session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub correlation_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_index: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<i64>,
    pub data: Value,
}

/// Writes human-debuggable session content to a per-session JSON file.
pub struct SessionLogger {
    logs_dir: PathBuf,
    write_lock: Mutex<()>,
}

impl SessionLogger {
    pub fn new(logs_dir: PathBuf) -> Self {
        Self {
            logs_dir,
            write_lock: Mutex::new(()),
        }
    }

    /// Snapshot of the effective session/system settings at generation start.
    pub fn log_settings(&self, ctx: &LogContext, settings: Value) {
        let mut entry = base_entry(KIND_SETTINGS, ctx);
        entry.data = settings;
        self.append(entry);
    }

    /// Injected context for a turn (project rules, context summary, role board, etc.).
    pub fn log_context(&self, ctx: &LogContext, source: &str, label: &str, content: &str) {
        if content.trim().is_empty() {
            return;
        }
        let mut entry = base_entry(KIND_CONTEXT, ctx);
        let (preview, truncated) = truncate_str_chars(content, MAX_LOG_CHARS);
        let mut data = json!({
            "source": source,
            "label": label,
            "content": preview,
        });
        if truncated {
            data["truncated"] = json!(true);
        }
        entry.data = data;
        self.append(entry);
    }

    /// A user message (prompt + attachment placeholders).
    pub fn log_user_message(&self, ctx: &LogContext, text: &str, attachments: Vec<Value>) {
        let mut entry = base_entry(KIND_USER_MESSAGE, ctx);
        let mut data = json!({ "text": text });
        if !attachments.is_empty() {
            data["attachments"] = Value::Array(attachments);
        }
        entry.data = data;
        self.append(entry);
    }

    /// One model turn — records visible text, reasoning, generated media,
    /// tool call names, token usage, and timing. Pure tool-call turns are
    /// included so the execution trace captures every model round-trip.
    #[allow(clippy::too_many_arguments)]
    pub fn log_assistant_turn(
        &self,
        ctx: &LogContext,
        turn_index: u32,
        model: &str,
        provider: &str,
        response: &GenerateResponse,
        started_at: i64,
        duration_ms: i64,
    ) {
        let text = response.text.as_deref().unwrap_or("");
        let thinking = response.thinking_content.as_deref().unwrap_or("");
        let mut entry = base_entry(KIND_ASSISTANT_TURN, ctx);
        entry.turn_index = Some(turn_index as i64);
        entry.started_at = Some(started_at);
        entry.duration_ms = Some(duration_ms.max(0));
        let mut data = json!({
            "model": model,
            "provider": provider,
            "usage": {
                "prompt_tokens": response.usage.prompt_tokens,
                "completion_tokens": response.usage.completion_tokens,
                "total_tokens": response.usage.total_tokens,
                "cache_read_tokens": response.usage.cache_read_tokens,
                "cache_write_tokens": response.usage.cache_write_tokens,
            },
        });
        if !text.is_empty() {
            data["text"] = json!(text);
        }
        if !thinking.is_empty() {
            data["thinking"] = json!(thinking);
        }
        if !response.tool_calls.is_empty() {
            let names: Vec<&str> = response.tool_calls.iter().map(|c| c.name.as_str()).collect();
            data["tool_calls"] = json!(names);
        }
        if !response.images.is_empty() {
            data["images"] = json!(response
                .images
                .iter()
                .map(|i| format!("<image {} bytes, {}>", i.bytes.len(), i.mime))
                .collect::<Vec<_>>());
        }
        if !response.videos.is_empty() {
            data["videos"] = json!(response
                .videos
                .iter()
                .map(|v| format!("<video {} bytes, {}>", v.bytes.len(), v.mime))
                .collect::<Vec<_>>());
        }
        entry.data = data;
        self.append(entry);
    }

    /// One tool invocation. Logs the input, timing, and output (truncated when
    /// overly large) for both success and error outcomes.
    pub fn log_tool_call(
        &self,
        ctx: &LogContext,
        tool_name: &str,
        input: &Value,
        result: &ToolResult,
        started_at: i64,
        duration_ms: i64,
    ) {
        let mut entry = base_entry(KIND_TOOL_CALL, ctx);
        entry.tool_name = Some(tool_name.to_string());
        entry.started_at = Some(started_at);
        entry.duration_ms = Some(duration_ms.max(0));
        let (output_val, truncated) = truncate_value_for_log(&result.content, MAX_LOG_CHARS);
        let mut data = json!({
            "input": input,
            "output": output_val,
        });
        if truncated {
            data["truncated"] = json!(true);
        }
        if result.is_error {
            data["is_error"] = json!(true);
            data["error"] = data["output"].clone();
        }
        entry.data = data;
        self.append(entry);
    }

    /// End-of-turn marker for a persisted assistant message.
    pub fn log_turn_summary(
        &self,
        ctx: &LogContext,
        message_id: &str,
        model: &str,
        provider: &str,
        usage: &TokenUsage,
    ) {
        let mut entry = base_entry(KIND_TURN_SUMMARY, ctx);
        entry.message_id = Some(message_id.to_string());
        entry.data = json!({
            "model": model,
            "provider": provider,
            "usage": {
                "prompt_tokens": usage.prompt_tokens,
                "completion_tokens": usage.completion_tokens,
                "total_tokens": usage.total_tokens,
            },
        });
        self.append(entry);
    }

    /// A generation failure. Captured so the debug log shows the error too.
    pub fn log_error(&self, ctx: &LogContext, message: &str) {
        let mut entry = base_entry(KIND_ERROR, ctx);
        entry.data = json!({ "message": message });
        self.append(entry);
    }

    /// Remove the on-disk log when a session is deleted.
    pub fn delete_session_log(&self, session_id: &str) {
        let _guard = match self.write_lock.lock() {
            Ok(g) => g,
            Err(_) => return,
        };
        let path = session_log_path(&self.logs_dir, session_id);
        if path.exists() {
            if let Err(e) = fs::remove_file(&path) {
                eprintln!("[session_log] delete session log failed: {e}");
            }
        }
    }

    /// Trim the session log from `message_id` onward (resend / delete branch).
    pub fn rollback_from_message(
        &self,
        conn: &crate::data::db::DbConn,
        session_id: &str,
        message_id: &str,
    ) {
        let Ok(Some(scope)) = rollback_scope_for_message(conn, session_id, message_id) else {
            return;
        };
        let _guard = match self.write_lock.lock() {
            Ok(g) => g,
            Err(_) => return,
        };
        if let Err(e) = rewrite_session_json(&self.logs_dir, session_id, |entry| {
            !should_drop_entry(entry, &scope)
        }) {
            eprintln!("[session_log] rollback failed: {e}");
        }
    }

    /// Read every trace entry recorded for `session_id` in chronological order.
    pub fn read_session(&self, session_id: &str) -> Vec<SessionLogEntry> {
        if session_id.trim().is_empty() {
            return Vec::new();
        }
        let _guard = match self.write_lock.lock() {
            Ok(g) => g,
            Err(_) => return Vec::new(),
        };
        read_entries(&session_log_path(&self.logs_dir, session_id))
    }

    fn append(&self, entry: SessionLogEntry) {
        if entry.session_id.is_empty() {
            eprintln!("[session_log] skipped write: missing session_id");
            return;
        }
        let _guard = match self.write_lock.lock() {
            Ok(g) => g,
            Err(_) => return,
        };
        if let Err(e) = append_session_json(&self.logs_dir, entry) {
            eprintln!("[session_log] write failed: {e}");
        }
    }
}

fn base_entry(kind: &str, ctx: &LogContext) -> SessionLogEntry {
    SessionLogEntry {
        id: Ulid::new().to_string(),
        ts: now_ms(),
        kind: kind.to_string(),
        session_id: ctx.session_id.clone().unwrap_or_default(),
        correlation_id: ctx.correlation_id.clone(),
        message_id: None,
        agent_id: ctx.agent_id.clone(),
        agent_type: ctx.agent_type.clone(),
        turn_index: None,
        tool_name: None,
        started_at: None,
        duration_ms: None,
        data: Value::Null,
    }
}

fn truncate_str_chars(s: &str, max_chars: usize) -> (String, bool) {
    if s.chars().count() <= max_chars {
        return (s.to_string(), false);
    }
    let truncated: String = s.chars().take(max_chars).collect();
    (format!("{truncated}\n...[truncated]"), true)
}

fn truncate_value_for_log(val: &Value, max_chars: usize) -> (Value, bool) {
    let serialized = serde_json::to_string_pretty(val).unwrap_or_default();
    if serialized.chars().count() <= max_chars {
        return (val.clone(), false);
    }
    let (preview, _) = truncate_str_chars(&serialized, max_chars);
    (Value::String(preview), true)
}

fn session_log_path(logs_dir: &Path, session_id: &str) -> PathBuf {
    logs_dir.join(format!("{session_id}.json"))
}

fn read_entries(path: &Path) -> Vec<SessionLogEntry> {
    let Ok(raw) = fs::read_to_string(path) else {
        return Vec::new();
    };
    serde_json::from_str::<Vec<SessionLogEntry>>(&raw).unwrap_or_default()
}

fn write_entries(path: &Path, entries: &[SessionLogEntry]) -> std::io::Result<()> {
    let data = serde_json::to_string_pretty(entries).unwrap_or_else(|_| "[]".into());
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, data)?;
    fs::rename(tmp, path)?;
    Ok(())
}

fn append_session_json(logs_dir: &Path, entry: SessionLogEntry) -> std::io::Result<()> {
    fs::create_dir_all(logs_dir)?;
    let path = session_log_path(logs_dir, &entry.session_id);
    let mut entries = read_entries(&path);
    entries.push(entry);
    write_entries(&path, &entries)
}

fn rewrite_session_json(
    logs_dir: &Path,
    session_id: &str,
    keep: impl Fn(&SessionLogEntry) -> bool,
) -> std::io::Result<()> {
    let path = session_log_path(logs_dir, session_id);
    if !path.exists() {
        return Ok(());
    }
    let kept: Vec<SessionLogEntry> = read_entries(&path)
        .into_iter()
        .filter(|e| keep(e))
        .collect();
    if kept.is_empty() {
        let _ = fs::remove_file(&path);
        return Ok(());
    }
    write_entries(&path, &kept)
}

/// Mirror of the token-log rollback: drop this message and everything
/// chronologically after it.
pub fn should_drop_entry(entry: &SessionLogEntry, scope: &TokenLogRollbackScope) -> bool {
    if entry.ts >= scope.pivot_created_at {
        return true;
    }
    if let Some(ref cid) = entry.correlation_id {
        if scope.user_message_ids.contains(cid) {
            return true;
        }
    }
    if let Some(ref mid) = entry.message_id {
        if scope.assistant_message_ids.contains(mid) {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn drop_entry_by_pivot_timestamp() {
        let scope = TokenLogRollbackScope {
            pivot_created_at: 100,
            user_message_ids: HashSet::new(),
            assistant_message_ids: HashSet::new(),
        };
        let mut entry = base_entry(
            KIND_ASSISTANT_TURN,
            &LogContext {
                session_id: Some("s".into()),
                correlation_id: None,
                agent_id: None,
                agent_type: None,
            },
        );
        entry.ts = 100;
        assert!(should_drop_entry(&entry, &scope));
        entry.ts = 99;
        assert!(!should_drop_entry(&entry, &scope));
    }

    #[test]
    fn drop_entry_by_correlation_id() {
        let scope = TokenLogRollbackScope {
            pivot_created_at: 1000,
            user_message_ids: HashSet::from(["u1".into()]),
            assistant_message_ids: HashSet::new(),
        };
        let mut entry = base_entry(
            KIND_TOOL_CALL,
            &LogContext {
                session_id: Some("s".into()),
                correlation_id: Some("u1".into()),
                agent_id: None,
                agent_type: None,
            },
        );
        entry.ts = 50;
        assert!(should_drop_entry(&entry, &scope));
    }

    #[test]
    fn legacy_json_entry_without_timing_deserializes() {
        let legacy = r#"[{
            "id": "e1",
            "ts": 1700000000000,
            "kind": "assistant_turn",
            "session_id": "s1",
            "turn_index": 1,
            "data": { "model": "gpt-4o", "provider": "openai", "text": "hello" }
        }]"#;
        let parsed: Vec<SessionLogEntry> = serde_json::from_str(legacy).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].started_at, None);
        assert_eq!(parsed[0].duration_ms, None);
    }

    #[test]
    fn logger_records_and_reads_trace_with_timing_and_context() {
        let dir = std::env::temp_dir().join(format!("moyan_trace_test_{}", Ulid::new()));
        let logger = SessionLogger::new(dir.clone());
        let ctx = LogContext {
            session_id: Some("sess_1".into()),
            correlation_id: Some("msg_u1".into()),
            agent_id: Some("agent_1".into()),
            agent_type: Some("main".into()),
        };

        logger.log_context(&ctx, "project_rules", "项目规则", "rule body");
        logger.log_user_message(&ctx, "你好", vec![]);

        let mut resp = GenerateResponse::default();
        resp.tool_calls.push(crate::ai::chat::ProviderToolCall::new(
            "tc1",
            "Read",
            json!({ "path": "a.md" }),
        ));
        resp.usage = TokenUsage {
            prompt_tokens: Some(100),
            completion_tokens: Some(20),
            total_tokens: Some(120),
            ..Default::default()
        };
        // Pure tool-call turn (empty text & thinking) must still be recorded.
        logger.log_assistant_turn(&ctx, 1, "claude-3-7", "anthropic", &resp, 1000, 250);

        let tool_res = ToolResult::ok(json!({ "content": "ok" }));
        logger.log_tool_call(&ctx, "Read", &json!({ "path": "a.md" }), &tool_res, 1250, 40);

        let entries = logger.read_session("sess_1");
        assert_eq!(entries.len(), 4);
        assert_eq!(entries[0].kind, KIND_CONTEXT);
        assert_eq!(entries[2].kind, KIND_ASSISTANT_TURN);
        assert_eq!(entries[2].started_at, Some(1000));
        assert_eq!(entries[2].duration_ms, Some(250));
        assert_eq!(entries[2].data["usage"]["total_tokens"], json!(120));
        assert_eq!(entries[2].data["tool_calls"], json!(["Read"]));
        assert_eq!(entries[3].kind, KIND_TOOL_CALL);
        assert_eq!(entries[3].duration_ms, Some(40));
        assert_eq!(entries[3].data["output"]["content"], json!("ok"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn rollback_by_user_correlation_id_clears_previous_retry_attempts() {
        let dir = std::env::temp_dir().join(format!("moyan_trace_rollback_{}", Ulid::new()));
        let logger = SessionLogger::new(dir.clone());
        let turn1 = LogContext {
            session_id: Some("sess_1".into()),
            correlation_id: Some("u1".into()),
            agent_id: None,
            agent_type: Some("general-purpose".into()),
        };
        let turn2 = LogContext {
            session_id: Some("sess_1".into()),
            correlation_id: Some("u2".into()),
            agent_id: None,
            agent_type: Some("general-purpose".into()),
        };

        logger.log_settings(&turn1, json!({ "model": "m1" }));
        logger.log_user_message(&turn1, "turn 1", vec![]);
        logger.log_settings(&turn2, json!({ "model": "m1" }));
        logger.log_user_message(&turn2, "turn 2 attempt 1", vec![]);
        logger.log_error(&turn2, "HTTP 429");

        let scope = TokenLogRollbackScope {
            pivot_created_at: i64::MAX,
            user_message_ids: HashSet::from(["u2".into()]),
            assistant_message_ids: HashSet::new(),
        };
        rewrite_session_json(&dir, "sess_1", |entry| !should_drop_entry(entry, &scope)).unwrap();

        logger.log_settings(&turn2, json!({ "model": "m1", "regenerate": true }));
        logger.log_user_message(&turn2, "turn 2 attempt 2", vec![]);

        let entries = logger.read_session("sess_1");
        assert_eq!(entries.len(), 4);
        assert_eq!(entries[0].correlation_id.as_deref(), Some("u1"));
        assert_eq!(entries[1].correlation_id.as_deref(), Some("u1"));
        assert_eq!(entries[2].correlation_id.as_deref(), Some("u2"));
        assert_eq!(entries[3].correlation_id.as_deref(), Some("u2"));
        assert_eq!(entries[3].data["text"], json!("turn 2 attempt 2"));

        let _ = fs::remove_dir_all(&dir);
    }
}
