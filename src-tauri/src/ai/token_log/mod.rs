//! Structured token usage logging: per-session JSONL files + SQLite events.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde_json::{json, Map, Value};

use crate::ai::agent::tools::ToolResult;
use crate::ai::chat::{ChatRequest, GenerateResponse};
use crate::ai::tokens::TokenUsage;
use crate::data::db::DbPool;
use crate::data::token_log::{
    self, should_drop_event, TokenUsageEvent, EVENT_API_CALL, EVENT_TOOL_CALL, EVENT_TURN_SUMMARY,
};

#[derive(Debug, Clone)]
pub struct LogContext {
    pub session_id: Option<String>,
    pub correlation_id: Option<String>,
    pub agent_id: Option<String>,
    pub agent_type: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ApiCallLog {
    pub ctx: LogContext,
    pub model: String,
    pub provider: String,
    pub turn_index: u32,
    pub usage: TokenUsage,
    /// Full request payload sent to the provider this turn (system prompt,
    /// user prompt, history, tool chain, tool results). Image bytes are
    /// replaced with placeholders to keep the log textual.
    pub request: Value,
    /// Full response from the provider this turn (text, thinking, tool calls).
    pub response: Value,
}

#[derive(Debug, Clone)]
pub struct ToolCallLog {
    pub ctx: LogContext,
    pub tool_name: String,
    pub result: ToolResult,
    pub input: Value,
}

#[derive(Debug, Clone)]
pub struct TurnSummaryLog {
    pub ctx: LogContext,
    pub message_id: String,
    pub model: String,
    pub provider: String,
    pub usage: TokenUsage,
}

pub struct TokenUsageLogger {
    pool: DbPool,
    logs_dir: PathBuf,
    write_lock: Mutex<()>,
}

impl TokenUsageLogger {
    pub fn new(pool: DbPool, logs_dir: PathBuf) -> Self {
        Self {
            pool,
            logs_dir,
            write_lock: Mutex::new(()),
        }
    }

    pub fn log_api_call(&self, entry: ApiCallLog) {
        let mut event = TokenUsageEvent::new(EVENT_API_CALL);
        apply_context(&mut event, &entry.ctx);
        event.model = Some(entry.model);
        event.provider = Some(entry.provider);
        event.turn_index = Some(entry.turn_index as i64);
        event.prompt_tokens = entry.usage.prompt_tokens;
        event.completion_tokens = entry.usage.completion_tokens;
        event.total_tokens = entry.usage.total_tokens;
        event.content_json = Some(
            json!({
                "request": entry.request,
                "response": entry.response,
            })
            .to_string(),
        );
        self.persist(event);
    }

    pub fn log_tool_call(&self, entry: ToolCallLog) {
        let (output_chars, output_bytes) = measure_content(&entry.result.content);
        let metadata = extract_tool_metadata(&entry.tool_name, &entry.input);
        let content = json!({
            "input": entry.input,
            "output": entry.result.content,
            "is_error": entry.result.is_error,
        });
        let mut event = TokenUsageEvent::new(EVENT_TOOL_CALL);
        apply_context(&mut event, &entry.ctx);
        event.tool_name = Some(entry.tool_name);
        event.output_chars = Some(output_chars);
        event.output_bytes = Some(output_bytes);
        event.is_error = entry.result.is_error;
        event.metadata_json = metadata.map(|v| v.to_string());
        event.content_json = Some(content.to_string());
        self.persist(event);
    }

    pub fn log_turn_summary(&self, entry: TurnSummaryLog) {
        let mut event = TokenUsageEvent::new(EVENT_TURN_SUMMARY);
        apply_context(&mut event, &entry.ctx);
        event.message_id = Some(entry.message_id);
        event.model = Some(entry.model);
        event.provider = Some(entry.provider);
        event.prompt_tokens = entry.usage.prompt_tokens;
        event.completion_tokens = entry.usage.completion_tokens;
        event.total_tokens = entry.usage.total_tokens;
        self.persist(event);
    }

    /// Trim the session JSONL file from `message_id` onward (resend / delete branch).
    /// SQLite rows are intentionally kept for historical analytics.
    pub fn rollback_jsonl_from_message(
        &self,
        conn: &crate::data::db::DbConn,
        session_id: &str,
        message_id: &str,
    ) {
        let Ok(Some(scope)) = token_log::rollback_scope_for_message(conn, session_id, message_id)
        else {
            return;
        };
        let _guard = match self.write_lock.lock() {
            Ok(g) => g,
            Err(_) => return,
        };
        if let Err(e) = rewrite_session_jsonl(&self.logs_dir, session_id, |event| {
            !should_drop_event(event, &scope)
        }) {
            eprintln!("[token_log] JSONL rollback failed: {e}");
        }
    }

    /// Remove the on-disk JSONL file when a session is deleted.
    pub fn delete_session_log(&self, session_id: &str) {
        let _guard = match self.write_lock.lock() {
            Ok(g) => g,
            Err(_) => return,
        };
        let path = session_log_path(&self.logs_dir, session_id);
        if path.exists() {
            if let Err(e) = fs::remove_file(&path) {
                eprintln!("[token_log] delete session log failed: {e}");
            }
        }
    }

    fn persist(&self, event: TokenUsageEvent) {
        let _guard = match self.write_lock.lock() {
            Ok(g) => g,
            Err(_) => return,
        };

        if let Some(ref sid) = event.session_id {
            if let Err(e) = append_session_jsonl(&self.logs_dir, sid, &event) {
                eprintln!("[token_log] JSONL write failed: {e}");
            }
        } else {
            eprintln!("[token_log] skipped JSONL write: missing session_id");
        }

        match self.pool.get() {
            Ok(conn) => {
                if let Err(e) = token_log::insert_event(&conn, &event) {
                    eprintln!("[token_log] SQLite insert failed: {e}");
                }
            }
            Err(e) => eprintln!("[token_log] DB pool unavailable: {e}"),
        }
    }
}

fn session_log_path(logs_dir: &Path, session_id: &str) -> PathBuf {
    logs_dir.join(format!("{session_id}.jsonl"))
}

fn append_session_jsonl(
    logs_dir: &Path,
    session_id: &str,
    event: &TokenUsageEvent,
) -> std::io::Result<()> {
    fs::create_dir_all(logs_dir)?;
    let path = session_log_path(logs_dir, session_id);
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    let line = serde_json::to_string(event).unwrap_or_else(|_| "{}".into());
    file.write_all(line.as_bytes())?;
    file.write_all(b"\n")?;
    Ok(())
}

fn rewrite_session_jsonl(
    logs_dir: &Path,
    session_id: &str,
    keep: impl Fn(&TokenUsageEvent) -> bool,
) -> std::io::Result<()> {
    let path = session_log_path(logs_dir, session_id);
    if !path.exists() {
        return Ok(());
    }

    let raw = fs::read_to_string(&path)?;
    let mut kept = Vec::new();
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(event) = serde_json::from_str::<TokenUsageEvent>(trimmed) else {
            continue;
        };
        if keep(&event) {
            kept.push(trimmed.to_string());
        }
    }

    if kept.is_empty() {
        let _ = fs::remove_file(&path);
        return Ok(());
    }

    let tmp = path.with_extension("jsonl.tmp");
    {
        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&tmp)?;
        for line in &kept {
            file.write_all(line.as_bytes())?;
            file.write_all(b"\n")?;
        }
    }
    fs::rename(tmp, path)?;
    Ok(())
}

fn apply_context(event: &mut TokenUsageEvent, ctx: &LogContext) {
    event.session_id = ctx.session_id.clone();
    event.correlation_id = ctx.correlation_id.clone();
    event.agent_id = ctx.agent_id.clone();
    event.agent_type = ctx.agent_type.clone();
}

fn measure_content(content: &Value) -> (i64, i64) {
    let s = serde_json::to_string(content).unwrap_or_default();
    (s.chars().count() as i64, s.len() as i64)
}

fn extract_tool_metadata(tool_name: &str, input: &Value) -> Option<Value> {
    match tool_name {
        "Read" | "Edit" | "Write" => {
            let mut m = Map::new();
            if let Some(p) = input.get("path").and_then(Value::as_str) {
                m.insert("path".into(), json!(p));
            }
            if let Some(n) = input.get("paragraph_number").and_then(Value::as_i64) {
                m.insert("paragraph_number".into(), json!(n));
            }
            if let Some(n) = input.get("paragraph_from").and_then(Value::as_i64) {
                m.insert("paragraph_from".into(), json!(n));
            }
            if let Some(n) = input.get("paragraph_to").and_then(Value::as_i64) {
                m.insert("paragraph_to".into(), json!(n));
            }
            match input.get("from") {
                Some(Value::Number(n)) => {
                    m.insert("from".into(), json!(n));
                }
                Some(Value::String(s)) => {
                    m.insert("from".into(), json!(s));
                }
                _ => {}
            }
            if m.is_empty() {
                None
            } else {
                Some(Value::Object(m))
            }
        }
        _ => None,
    }
}

/// Serialise the outbound provider request into a textual JSON payload.
/// Raw image bytes are collapsed into `<image N bytes, mime>` placeholders
/// so the debug log stays human-readable and avoids huge base64 blobs.
pub fn request_content(chat: &ChatRequest) -> Value {
    json!({
        "model": chat.model,
        "provider": chat.provider.id,
        "system_prompt": chat.system_prompt,
        "prompt": chat.prompt,
        "attachments": chat.attachments.iter().map(image_placeholder).collect::<Vec<_>>(),
        "history": chat
            .history
            .iter()
            .map(|h| {
                json!({
                    "role": h.role,
                    "text": h.text,
                    "thinking_content": h.thinking_content,
                    "images": h.images.iter().map(image_placeholder).collect::<Vec<_>>(),
                })
            })
            .collect::<Vec<_>>(),
        "tools": chat.tools.iter().map(|t| t.name.clone()).collect::<Vec<_>>(),
        "tool_chain": chat
            .tool_chain
            .iter()
            .map(|round| {
                json!({
                    "assistant": pending_turn_json(&round.assistant),
                    "results": round.results.iter().map(tool_result_json).collect::<Vec<_>>(),
                })
            })
            .collect::<Vec<_>>(),
        "pending_assistant_turn": chat.pending_assistant_turn.as_ref().map(pending_turn_json),
        "tool_results": chat.tool_results.iter().map(tool_result_json).collect::<Vec<_>>(),
    })
}

/// Serialise the provider response (text, reasoning, tool calls, images).
pub fn response_content(resp: &GenerateResponse) -> Value {
    json!({
        "text": resp.text,
        "thinking_content": resp.thinking_content,
        "tool_calls": resp
            .tool_calls
            .iter()
            .map(|c| json!({ "id": c.id, "name": c.name, "arguments": c.arguments }))
            .collect::<Vec<_>>(),
        "images": resp
            .images
            .iter()
            .map(|i| format!("<image {} bytes, {}>", i.bytes.len(), i.mime))
            .collect::<Vec<_>>(),
        "videos": resp
            .videos
            .iter()
            .map(|i| format!("<video {} bytes, {}>", i.bytes.len(), i.mime))
            .collect::<Vec<_>>(),
    })
}

fn image_placeholder(att: &crate::ai::chat::AttachmentBytes) -> Value {
    json!(format!("<image {} bytes, {}>", att.bytes.len(), att.mime))
}

fn tool_result_json(r: &crate::ai::chat::ToolResultMessage) -> Value {
    json!({
        "tool_call_id": r.tool_call_id,
        "content": r.content,
        "is_error": r.is_error,
    })
}

fn pending_turn_json(turn: &crate::ai::chat::PendingAssistantTurn) -> Value {
    json!({
        "text": turn.text,
        "thinking_content": turn.thinking_content,
        "tool_calls": turn
            .tool_calls
            .iter()
            .map(|c| json!({ "id": c.id, "name": c.name, "arguments": c.arguments }))
            .collect::<Vec<_>>(),
    })
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;
    use crate::data::token_log::TokenLogRollbackScope;

    #[test]
    fn should_drop_by_pivot_timestamp() {
        let scope = TokenLogRollbackScope {
            pivot_created_at: 100,
            user_message_ids: HashSet::new(),
            assistant_message_ids: HashSet::new(),
        };
        let mut event = TokenUsageEvent::new(EVENT_API_CALL);
        event.created_at = 100;
        assert!(should_drop_event(&event, &scope));
        event.created_at = 99;
        assert!(!should_drop_event(&event, &scope));
    }

    #[test]
    fn should_drop_by_correlation_id() {
        let scope = TokenLogRollbackScope {
            pivot_created_at: 1000,
            user_message_ids: HashSet::from(["u1".into()]),
            assistant_message_ids: HashSet::new(),
        };
        let mut event = TokenUsageEvent::new(EVENT_TOOL_CALL);
        event.created_at = 50;
        event.correlation_id = Some("u1".into());
        assert!(should_drop_event(&event, &scope));
    }
}
