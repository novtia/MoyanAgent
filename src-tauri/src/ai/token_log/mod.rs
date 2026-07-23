//! Token usage statistics: numeric events recorded into SQLite.
//!
//! This module is deliberately narrow — it only records *numbers* (token
//! counts, call counts, per-model breakdowns) for analytics/billing, into
//! the `token_usage_events` table. It does **not** write any files and does
//! **not** store request/response content.
//!
//! The session *content* (system settings, messages, tool I/O, errors) is
//! the responsibility of the separate [`crate::ai::session_log`] module,
//! which writes per-session JSON files for debugging.

use serde_json::{json, Map, Value};

use crate::ai::agent::tools::ToolResult;
use crate::ai::tokens::TokenUsage;
use crate::data::db::DbPool;
use crate::data::token_log::{
    self, TokenUsageEvent, EVENT_API_CALL, EVENT_TOOL_CALL, EVENT_TURN_SUMMARY,
};

/// Shared metadata identifying which session/turn/agent a log line belongs to.
/// Used by both [`TokenStatsRecorder`] and [`crate::ai::session_log`].
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

/// Records token usage statistics into SQLite. Cheap to share via `Arc`.
pub struct TokenStatsRecorder {
    pool: DbPool,
}

impl TokenStatsRecorder {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
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
        self.persist(event);
    }

    pub fn log_tool_call(&self, entry: ToolCallLog) {
        let (output_chars, output_bytes) = measure_content(&entry.result.content);
        let metadata = extract_tool_metadata(&entry.tool_name, &entry.input);
        let mut event = TokenUsageEvent::new(EVENT_TOOL_CALL);
        apply_context(&mut event, &entry.ctx);
        event.tool_name = Some(entry.tool_name);
        event.output_chars = Some(output_chars);
        event.output_bytes = Some(output_bytes);
        event.is_error = entry.result.is_error;
        event.metadata_json = metadata.map(|v| v.to_string());
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

    fn persist(&self, event: TokenUsageEvent) {
        match self.pool.get() {
            Ok(conn) => {
                if let Err(e) = token_log::insert_event(&conn, &event) {
                    eprintln!("[token_stats] SQLite insert failed: {e}");
                }
            }
            Err(e) => eprintln!("[token_stats] DB pool unavailable: {e}"),
        }
    }
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
