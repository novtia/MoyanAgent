use std::collections::HashSet;

use rusqlite::params;
use serde::{Deserialize, Serialize};
use ulid::Ulid;

use crate::data::db::{now_ms, DbConn};
use crate::error::{AppError, AppResult};

pub const EVENT_API_CALL: &str = "api_call";
pub const EVENT_TOOL_CALL: &str = "tool_call";
pub const EVENT_TURN_SUMMARY: &str = "turn_summary";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenUsageEvent {
    pub id: String,
    pub created_at: i64,
    pub event_kind: String,
    pub session_id: Option<String>,
    pub correlation_id: Option<String>,
    pub message_id: Option<String>,
    pub agent_id: Option<String>,
    pub agent_type: Option<String>,
    pub model: Option<String>,
    pub provider: Option<String>,
    pub turn_index: Option<i64>,
    pub tool_name: Option<String>,
    pub prompt_tokens: Option<i64>,
    pub completion_tokens: Option<i64>,
    pub total_tokens: Option<i64>,
    pub output_chars: Option<i64>,
    pub output_bytes: Option<i64>,
    pub is_error: bool,
    pub metadata_json: Option<String>,
    /// Full debug payload (tool input/output, request/response content).
    /// `#[serde(default)]` keeps older JSONL lines (written before this
    /// column existed) deserialisable during rollback rewrites.
    #[serde(default)]
    pub content_json: Option<String>,
}

impl TokenUsageEvent {
    pub fn new(event_kind: impl Into<String>) -> Self {
        Self {
            id: Ulid::new().to_string(),
            created_at: now_ms(),
            event_kind: event_kind.into(),
            session_id: None,
            correlation_id: None,
            message_id: None,
            agent_id: None,
            agent_type: None,
            model: None,
            provider: None,
            turn_index: None,
            tool_name: None,
            prompt_tokens: None,
            completion_tokens: None,
            total_tokens: None,
            output_chars: None,
            output_bytes: None,
            is_error: false,
            metadata_json: None,
            content_json: None,
        }
    }
}

pub fn insert_event(conn: &DbConn, event: &TokenUsageEvent) -> AppResult<()> {
    conn.execute(
        "INSERT INTO token_usage_events(
            id, created_at, event_kind, session_id, correlation_id, message_id,
            agent_id, agent_type, model, provider, turn_index, tool_name,
            prompt_tokens, completion_tokens, total_tokens, output_chars,
            output_bytes, is_error, metadata_json, content_json
        ) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20)",
        params![
            event.id,
            event.created_at,
            event.event_kind,
            event.session_id,
            event.correlation_id,
            event.message_id,
            event.agent_id,
            event.agent_type,
            event.model,
            event.provider,
            event.turn_index,
            event.tool_name,
            event.prompt_tokens,
            event.completion_tokens,
            event.total_tokens,
            event.output_chars,
            event.output_bytes,
            event.is_error as i64,
            event.metadata_json,
            event.content_json,
        ],
    )?;
    Ok(())
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ModelUsageRow {
    pub model: String,
    pub provider: Option<String>,
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub total_tokens: i64,
    pub event_count: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TokenUsageSummary {
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub total_tokens: i64,
    pub api_call_count: i64,
    pub tool_call_count: i64,
    pub turn_summary_count: i64,
    pub by_model: Vec<ModelUsageRow>,
}

#[derive(Debug, Clone, Default)]
pub struct TokenUsageListFilter {
    pub session_id: Option<String>,
    pub model: Option<String>,
    pub event_kind: Option<String>,
    pub from_ms: Option<i64>,
    pub to_ms: Option<i64>,
    pub limit: i64,
    pub offset: i64,
}

fn map_event(row: &rusqlite::Row<'_>) -> rusqlite::Result<TokenUsageEvent> {
    Ok(TokenUsageEvent {
        id: row.get(0)?,
        created_at: row.get(1)?,
        event_kind: row.get(2)?,
        session_id: row.get(3)?,
        correlation_id: row.get(4)?,
        message_id: row.get(5)?,
        agent_id: row.get(6)?,
        agent_type: row.get(7)?,
        model: row.get(8)?,
        provider: row.get(9)?,
        turn_index: row.get(10)?,
        tool_name: row.get(11)?,
        prompt_tokens: row.get(12)?,
        completion_tokens: row.get(13)?,
        total_tokens: row.get(14)?,
        output_chars: row.get(15)?,
        output_bytes: row.get(16)?,
        is_error: row.get::<_, i64>(17)? != 0,
        metadata_json: row.get(18)?,
        content_json: row.get(19)?,
    })
}

pub fn query_summary(
    conn: &DbConn,
    from_ms: Option<i64>,
    to_ms: Option<i64>,
) -> AppResult<TokenUsageSummary> {
    let mut sql = String::from(
        "SELECT
            COALESCE(SUM(prompt_tokens), 0),
            COALESCE(SUM(completion_tokens), 0),
            COALESCE(SUM(total_tokens), 0),
            SUM(CASE WHEN event_kind = 'api_call' THEN 1 ELSE 0 END),
            SUM(CASE WHEN event_kind = 'tool_call' THEN 1 ELSE 0 END),
            SUM(CASE WHEN event_kind = 'turn_summary' THEN 1 ELSE 0 END)
         FROM token_usage_events WHERE 1=1",
    );
    let mut args: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
    if let Some(from) = from_ms {
        sql.push_str(" AND created_at >= ?");
        args.push(Box::new(from));
    }
    if let Some(to) = to_ms {
        sql.push_str(" AND created_at <= ?");
        args.push(Box::new(to));
    }
    let params_ref: Vec<&dyn rusqlite::ToSql> = args.iter().map(|b| b.as_ref()).collect();
    let (prompt, completion, total, api, tool, turn): (i64, i64, i64, i64, i64, i64) = conn
        .query_row(&sql, params_ref.as_slice(), |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?))
        })?;

    let mut model_sql = String::from(
        "SELECT COALESCE(model, ''), provider,
                COALESCE(SUM(prompt_tokens), 0),
                COALESCE(SUM(completion_tokens), 0),
                COALESCE(SUM(total_tokens), 0),
                COUNT(*)
         FROM token_usage_events
         WHERE event_kind IN ('api_call', 'turn_summary')",
    );
    let mut model_args: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
    if let Some(from) = from_ms {
        model_sql.push_str(" AND created_at >= ?");
        model_args.push(Box::new(from));
    }
    if let Some(to) = to_ms {
        model_sql.push_str(" AND created_at <= ?");
        model_args.push(Box::new(to));
    }
    model_sql.push_str(" GROUP BY COALESCE(model, ''), provider ORDER BY SUM(total_tokens) DESC");
    let model_params: Vec<&dyn rusqlite::ToSql> = model_args.iter().map(|b| b.as_ref()).collect();
    let mut stmt = conn.prepare(&model_sql)?;
    let by_model = stmt
        .query_map(model_params.as_slice(), |r| {
            Ok(ModelUsageRow {
                model: r.get(0)?,
                provider: r.get(1)?,
                prompt_tokens: r.get(2)?,
                completion_tokens: r.get(3)?,
                total_tokens: r.get(4)?,
                event_count: r.get(5)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(TokenUsageSummary {
        prompt_tokens: prompt,
        completion_tokens: completion,
        total_tokens: total,
        api_call_count: api,
        tool_call_count: tool,
        turn_summary_count: turn,
        by_model,
    })
}

/// Messages from the rolled-back branch — used to trim the per-session JSONL file.
#[derive(Debug, Clone)]
pub struct TokenLogRollbackScope {
    pub pivot_created_at: i64,
    pub user_message_ids: HashSet<String>,
    pub assistant_message_ids: HashSet<String>,
}

/// Compute which token-log events should be removed when rolling back from `message_id`.
/// Mirrors file/role snapshot rollback: this message and everything chronologically after it.
pub fn rollback_scope_for_message(
    conn: &DbConn,
    session_id: &str,
    message_id: &str,
) -> AppResult<Option<TokenLogRollbackScope>> {
    let (target_created_at, target_role): (i64, String) = conn
        .query_row(
            "SELECT created_at, role FROM messages WHERE id = ?1 AND session_id = ?2",
            params![message_id, session_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(|_| AppError::NotFound(format!("message {message_id}")))?;

    let pivot_created_at = if target_role == "user" {
        target_created_at
    } else {
        conn.query_row(
            "SELECT created_at FROM messages
             WHERE session_id = ?1 AND role = 'user' AND created_at < ?2
             ORDER BY created_at DESC LIMIT 1",
            params![session_id, target_created_at],
            |r| r.get(0),
        )
        .unwrap_or(target_created_at)
    };

    let mut user_message_ids = HashSet::new();
    let mut assistant_message_ids = HashSet::new();
    let mut stmt = conn.prepare(
        "SELECT id, role FROM messages
         WHERE session_id = ?1 AND created_at >= ?2",
    )?;
    let rows = stmt.query_map(params![session_id, pivot_created_at], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
    })?;
    for row in rows {
        let (id, role) = row?;
        if role == "user" {
            user_message_ids.insert(id);
        } else if role == "assistant" {
            assistant_message_ids.insert(id);
        }
    }

    Ok(Some(TokenLogRollbackScope {
        pivot_created_at,
        user_message_ids,
        assistant_message_ids,
    }))
}

pub fn list_events(conn: &DbConn, filter: &TokenUsageListFilter) -> AppResult<Vec<TokenUsageEvent>> {
    let limit = filter.limit.clamp(1, 500);
    let offset = filter.offset.max(0);
    let mut sql = String::from(
        "SELECT id, created_at, event_kind, session_id, correlation_id, message_id,
                agent_id, agent_type, model, provider, turn_index, tool_name,
                prompt_tokens, completion_tokens, total_tokens, output_chars,
                output_bytes, is_error, metadata_json, content_json
         FROM token_usage_events WHERE 1=1",
    );
    let mut args: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
    if let Some(ref sid) = filter.session_id {
        sql.push_str(" AND session_id = ?");
        args.push(Box::new(sid.clone()));
    }
    if let Some(ref model) = filter.model {
        sql.push_str(" AND model = ?");
        args.push(Box::new(model.clone()));
    }
    if let Some(ref kind) = filter.event_kind {
        sql.push_str(" AND event_kind = ?");
        args.push(Box::new(kind.clone()));
    }
    if let Some(from) = filter.from_ms {
        sql.push_str(" AND created_at >= ?");
        args.push(Box::new(from));
    }
    if let Some(to) = filter.to_ms {
        sql.push_str(" AND created_at <= ?");
        args.push(Box::new(to));
    }
    sql.push_str(" ORDER BY created_at DESC LIMIT ? OFFSET ?");
    args.push(Box::new(limit));
    args.push(Box::new(offset));
    let params_ref: Vec<&dyn rusqlite::ToSql> = args.iter().map(|b| b.as_ref()).collect();
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params_ref.as_slice(), map_event)?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}
