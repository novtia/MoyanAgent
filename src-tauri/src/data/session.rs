use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use ulid::Ulid;

use crate::ai::tokens;
use crate::data::db::{now_ms, DbConn};
use crate::data::message_search;
use crate::data::settings::{
    validate_model_param_settings, ModelParamSettings, DEFAULT_HISTORY_TURNS,
};
use crate::error::{AppError, AppResult};

fn decode_llm_params(raw: Option<String>) -> ModelParamSettings {
    raw.and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// Persisted `sessions.agent_type` values the UI may set for main-chat generation.
pub const SESSION_AGENT_GENERAL: &str = "general-purpose";
pub const SESSION_AGENT_PLAN: &str = "Plan";
/// Default main-session mode: normal chat with AskUser + web tools only.
pub const SESSION_AGENT_CHAT: &str = "chat";
/// TRPG director mode (project sessions only).
pub const SESSION_AGENT_DIRECTOR: &str = "trpg-director";

/// Sentinel used inside `agent_chain` to mark the session's default main agent.
/// Resolved at generation time to [`generation_agent_definition_key`] of the
/// session's `agent_type`. The main agent is always present and cannot be
/// removed; other agents are arranged before/after it.
pub const AGENT_CHAIN_MAIN: &str = "__main__";

/// Maps DB `sessions.agent_type` → agent registry key for the primary-session
/// agent run ([`crate::ai::agent::run_agent`]).
pub fn generation_agent_definition_key(stored: &str) -> &'static str {
    match stored.trim() {
        SESSION_AGENT_CHAT => SESSION_AGENT_CHAT,
        SESSION_AGENT_PLAN => SESSION_AGENT_PLAN,
        SESSION_AGENT_DIRECTOR => SESSION_AGENT_DIRECTOR,
        _ => SESSION_AGENT_GENERAL,
    }
}

/// Per-node configuration overrides applied to a single position in an agent
/// flow chain. Each field is optional; `None` keeps the resolved agent
/// definition's value. These overrides live only inside the chain
/// (session/project `agent_chain`) and never mutate the global built-in or
/// custom agent definition.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct NodeOverrides {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<String>>,
}

impl NodeOverrides {
    pub fn is_empty(&self) -> bool {
        self.system_prompt.is_none() && self.model.is_none() && self.tools.is_none()
    }
}

/// One node in an agent flow chain: an `agent_type` plus optional per-node
/// overrides. Serialises to a bare string when there are no overrides (so the
/// legacy `["Explore", "__main__"]` wire format is preserved), otherwise to
/// `{ "agent_type": ..., "overrides": {...} }`.
#[derive(Debug, Clone)]
pub struct ChainNode {
    pub agent_type: String,
    pub overrides: Option<NodeOverrides>,
}

impl ChainNode {
    pub fn bare(agent_type: impl Into<String>) -> Self {
        Self {
            agent_type: agent_type.into(),
            overrides: None,
        }
    }

    /// The override set, but only when it actually carries at least one value.
    pub fn effective_overrides(&self) -> Option<&NodeOverrides> {
        self.overrides.as_ref().filter(|o| !o.is_empty())
    }
}

impl Serialize for ChainNode {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self.effective_overrides() {
            None => self.agent_type.serialize(s),
            Some(ov) => {
                #[derive(Serialize)]
                struct Full<'a> {
                    agent_type: &'a str,
                    overrides: &'a NodeOverrides,
                }
                Full {
                    agent_type: &self.agent_type,
                    overrides: ov,
                }
                .serialize(s)
            }
        }
    }
}

impl<'de> Deserialize<'de> for ChainNode {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Bare(String),
            Full {
                agent_type: String,
                #[serde(default)]
                overrides: Option<NodeOverrides>,
            },
        }
        Ok(match Raw::deserialize(d)? {
            Raw::Bare(s) => ChainNode {
                agent_type: s,
                overrides: None,
            },
            Raw::Full {
                agent_type,
                overrides,
            } => ChainNode {
                agent_type,
                overrides,
            },
        })
    }
}

/// Normalise a chain: trim agent types, drop empty entries, and collapse empty
/// override objects back to `None` so they re-serialise as bare strings.
pub(crate) fn normalize_chain(chain: &[ChainNode]) -> Vec<ChainNode> {
    chain
        .iter()
        .filter_map(|n| {
            let agent_type = n.agent_type.trim().to_string();
            if agent_type.is_empty() {
                return None;
            }
            let overrides = n.overrides.clone().filter(|o| !o.is_empty());
            Some(ChainNode {
                agent_type,
                overrides,
            })
        })
        .collect()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub title: String,
    pub model: Option<String>,
    /// Which model provider (service) this session uses. `None` means unset /
    /// follow the global default until resolved.
    pub provider_id: Option<String>,
    pub system_prompt: String,
    pub history_turns: i64,
    pub llm_params: ModelParamSettings,
    /// Context window limit for this session (tokens). `None` means unset / follow model.
    pub context_window: Option<i64>,
    /// Cumulative context usage tracked for this session (tokens).
    pub context_window_used: i64,
    /// Which built-in agent definition drives turns (`general-purpose` | `Plan`, …).
    pub agent_type: String,
    /// Ordered agent flow chain. `None`/empty means a single agent run driven
    /// by `agent_type`. Each node is an agent type plus optional per-node
    /// config overrides (see [`ChainNode`]).
    pub agent_chain: Option<Vec<ChainNode>>,
    /// Project this session belongs to, if any.
    pub project_id: Option<String>,
    /// Parent session when this is a temporary subagent child session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_session_id: Option<String>,
    /// Hidden from the sidebar when true (subagent temp sessions).
    #[serde(default)]
    pub is_temporary: bool,
    /// Task id of the Agent tool run that spawned this temp session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spawn_task_id: Option<String>,
    /// Latest Volcengine Responses API `response.id` for Session cache chaining.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_response_id: Option<String>,
    /// Thinking type (`enabled`/`disabled`) used when the cache chain was created.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_thinking_key: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

/// Resolve the main agent for a generation turn.
/// Standalone (no project) sessions always run in ask/`chat` mode.
/// Project sessions honour the session's stored `agent_type` (ask / plan / agent / director).
pub fn session_generation_agent(sess: &Session) -> &'static str {
    if sess.project_id.is_none() {
        SESSION_AGENT_CHAT
    } else {
        generation_agent_definition_key(&sess.agent_type)
    }
}

fn decode_agent_chain(raw: Option<String>) -> Option<Vec<ChainNode>> {
    let raw = raw?;
    let parsed: Vec<ChainNode> = serde_json::from_str(&raw).ok()?;
    let cleaned = normalize_chain(&parsed);
    if cleaned.is_empty() {
        None
    } else {
        Some(cleaned)
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionSummary {
    pub id: String,
    pub title: String,
    pub model: Option<String>,
    pub provider_id: Option<String>,
    pub system_prompt: String,
    pub history_turns: i64,
    pub llm_params: ModelParamSettings,
    pub context_window: Option<i64>,
    pub context_window_used: i64,
    pub agent_type: String,
    pub updated_at: i64,
    pub message_count: i64,
    pub project_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionSearchResult {
    pub id: String,
    pub title: String,
    pub model: Option<String>,
    pub provider_id: Option<String>,
    pub system_prompt: String,
    pub history_turns: i64,
    pub llm_params: ModelParamSettings,
    pub context_window: Option<i64>,
    pub context_window_used: i64,
    pub agent_type: String,
    pub updated_at: i64,
    pub message_count: i64,
    pub project_id: Option<String>,
    pub match_message_id: Option<String>,
    pub match_role: Option<String>,
    pub match_text: Option<String>,
    pub match_created_at: Option<i64>,
    pub match_count: i64,
    pub title_match: bool,
}

/// Lightweight message index row for in-session timeline / virtual list.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageOutlineItem {
    pub id: String,
    pub role: String,
    pub preview: Option<String>,
    pub created_at: i64,
}

const OUTLINE_PREVIEW_CHARS: usize = 100;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageRef {
    pub id: String,
    pub role: String, // input | output | edited | draft
    pub rel_path: String,
    pub thumb_rel_path: Option<String>,
    pub mime: String,
    pub media_role: Option<String>,
    pub source_url: Option<String>,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub bytes: Option<i64>,
    pub ord: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub id: String,
    pub session_id: String,
    pub role: String, // user | assistant | error
    pub text: Option<String>,
    pub params: Option<serde_json::Value>,
    pub created_at: i64,
    pub images: Vec<ImageRef>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionWithMessages {
    pub session: Session,
    pub messages: Vec<Message>,
    /// Parent session title when `session.parent_session_id` is set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_title: Option<String>,
}

pub fn create(conn: &DbConn, title: Option<String>, model: Option<String>) -> AppResult<Session> {
    let id = Ulid::new().to_string();
    let now = now_ms();
    let title = title.unwrap_or_else(|| "New session".into());
    conn.execute(
        "INSERT INTO sessions(id, title, model, system_prompt, agent_type, created_at, updated_at) VALUES(?1, ?2, ?3, '', ?4, ?5, ?5)",
        params![id, title, model, SESSION_AGENT_CHAT, now],
    )?;
    let _ = message_search::index_new_session(conn, &id);
    Ok(Session {
        id,
        title,
        model,
        provider_id: None,
        system_prompt: String::new(),
        history_turns: DEFAULT_HISTORY_TURNS,
        llm_params: ModelParamSettings::default(),
        context_window: None,
        context_window_used: 0,
        agent_type: SESSION_AGENT_CHAT.into(),
        agent_chain: None,
        project_id: None,
        parent_session_id: None,
        is_temporary: false,
        spawn_task_id: None,
        last_response_id: None,
        cache_thinking_key: None,
        created_at: now,
        updated_at: now,
    })
}

/// Create a hidden temporary child session for an Agent tool dispatch.
/// Copies model / provider / project / llm settings from the parent.
pub fn create_temp(
    conn: &DbConn,
    parent_id: &str,
    title: &str,
    spawn_task_id: Option<&str>,
) -> AppResult<Session> {
    let parent = get(conn, parent_id)?;
    let id = Ulid::new().to_string();
    let now = now_ms();
    let title = {
        let t = title.trim();
        if t.is_empty() {
            "Temporary session".to_string()
        } else {
            t.chars().take(120).collect()
        }
    };
    let llm_json = serde_json::to_string(&parent.llm_params)
        .map_err(|e| AppError::Invalid(format!("failed to serialize llm_params: {e}")))?;
    conn.execute(
        "INSERT INTO sessions(
            id, title, model, provider_id, system_prompt, history_turns, llm_params,
            context_window, context_window_used, agent_type, project_id,
            parent_session_id, is_temporary, spawn_task_id,
            created_at, updated_at
         ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,0,?9,?10,?11,1,?12,?13,?13)",
        params![
            id,
            title,
            parent.model,
            parent.provider_id,
            parent.system_prompt,
            parent.history_turns,
            llm_json,
            parent.context_window,
            parent.agent_type,
            parent.project_id,
            parent_id,
            spawn_task_id,
            now,
        ],
    )?;
    let _ = message_search::index_new_session(conn, &id);
    Ok(Session {
        id,
        title,
        model: parent.model,
        provider_id: parent.provider_id,
        system_prompt: parent.system_prompt,
        history_turns: parent.history_turns,
        llm_params: parent.llm_params,
        context_window: parent.context_window,
        context_window_used: 0,
        agent_type: parent.agent_type,
        agent_chain: None,
        project_id: parent.project_id,
        parent_session_id: Some(parent_id.to_string()),
        is_temporary: true,
        spawn_task_id: spawn_task_id.map(|s| s.to_string()),
        last_response_id: None,
        cache_thinking_key: None,
        created_at: now,
        updated_at: now,
    })
}

/// Ids of temporary child sessions spawned from `parent_id`.
pub fn list_temp_child_ids(conn: &DbConn, parent_id: &str) -> AppResult<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT id FROM sessions WHERE parent_session_id=?1 AND is_temporary=1",
    )?;
    let rows = stmt.query_map(params![parent_id], |r| r.get::<_, String>(0))?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

pub fn set_spawn_task_id(conn: &DbConn, id: &str, spawn_task_id: &str) -> AppResult<()> {
    let n = conn.execute(
        "UPDATE sessions SET spawn_task_id=?1, updated_at=?2 WHERE id=?3",
        params![spawn_task_id, now_ms(), id],
    )?;
    if n == 0 {
        return Err(AppError::NotFound(format!("session {id}")));
    }
    Ok(())
}

/// Persist the ordered agent flow chain for a session. An empty list clears
/// the chain (the session falls back to single-agent generation).
pub fn set_agent_chain(conn: &DbConn, id: &str, chain: &[ChainNode]) -> AppResult<()> {
    let cleaned = normalize_chain(chain);
    let stored: Option<String> = if cleaned.is_empty() {
        None
    } else {
        Some(
            serde_json::to_string(&cleaned)
                .map_err(|e| AppError::Invalid(format!("failed to serialize agent_chain: {e}")))?,
        )
    };
    let updated = now_ms();
    let n = conn.execute(
        "UPDATE sessions SET agent_chain=?1, updated_at=?2 WHERE id=?3",
        params![stored, updated, id],
    )?;
    if n == 0 {
        return Err(AppError::NotFound(format!("session {id}")));
    }
    Ok(())
}

pub fn set_agent_type(conn: &DbConn, id: &str, agent_type: &str) -> AppResult<()> {
    let t = agent_type.trim();
    if t != SESSION_AGENT_GENERAL
        && t != SESSION_AGENT_PLAN
        && t != SESSION_AGENT_CHAT
        && t != SESSION_AGENT_DIRECTOR
    {
        return Err(AppError::Invalid(format!(
            "agent_type must be \"{SESSION_AGENT_GENERAL}\", \"{SESSION_AGENT_PLAN}\", \"{SESSION_AGENT_CHAT}\", or \"{SESSION_AGENT_DIRECTOR}\""
        )));
    }
    // Agent / Plan / Director are project-session modes only. Standalone stays on ask/chat.
    if t != SESSION_AGENT_CHAT {
        let project_id: Option<String> = conn
            .query_row(
                "SELECT project_id FROM sessions WHERE id=?1",
                params![id],
                |r| r.get(0),
            )
            .map_err(|_| AppError::NotFound(format!("session {id}")))?;
        if project_id.is_none() {
            return Err(AppError::Invalid(
                "agent/plan/director mode is only available for project sessions".into(),
            ));
        }
    }
    let updated = now_ms();
    let n = conn.execute(
        "UPDATE sessions SET agent_type=?1, updated_at=?2 WHERE id=?3",
        params![t, updated, id],
    )?;
    if n == 0 {
        return Err(AppError::NotFound(format!("session {id}")));
    }
    Ok(())
}

pub fn rename(conn: &DbConn, id: &str, title: &str) -> AppResult<()> {
    let (old_title,): (String,) = conn
        .query_row(
            "SELECT title FROM sessions WHERE id=?1",
            params![id],
            |r| Ok((r.get(0)?,)),
        )
        .map_err(|_| AppError::NotFound(format!("session {id}")))?;
    let updated = now_ms();
    let n = conn.execute(
        "UPDATE sessions SET title=?1, updated_at=?2 WHERE id=?3",
        params![title, updated, id],
    )?;
    if n == 0 {
        return Err(AppError::NotFound(format!("session {id}")));
    }
    let _ = message_search::update_session_title_fts(conn, id, &old_title, title);
    Ok(())
}

pub fn delete(conn: &DbConn, id: &str) -> AppResult<()> {
    conn.execute("DELETE FROM sessions WHERE id=?1", params![id])?;
    Ok(())
}

pub fn update_config(
    conn: &DbConn,
    id: &str,
    system_prompt: &str,
    history_turns: i64,
    llm_params: &ModelParamSettings,
) -> AppResult<()> {
    if history_turns < 0 {
        return Err(AppError::Invalid(
            "history_turns must be non-negative".into(),
        ));
    }
    validate_model_param_settings(llm_params)?;
    let prev = get(conn, id)?;
    let params_json = serde_json::to_string(llm_params)
        .map_err(|e| AppError::Invalid(format!("failed to serialize llm_params: {e}")))?;
    let updated = now_ms();
    let n = conn.execute(
        "UPDATE sessions SET system_prompt=?1, history_turns=?2, llm_params=?3, updated_at=?4 WHERE id=?5",
        params![system_prompt, history_turns, params_json, updated, id],
    )?;
    if n == 0 {
        return Err(AppError::NotFound(format!("session {id}")));
    }
    // System prompt or thinking changes break the Volcengine cache chain.
    let thinking_changed = prev.llm_params.thinking_enabled != llm_params.thinking_enabled
        || prev.llm_params.thinking_effort != llm_params.thinking_effort;
    let prompt_changed = prev.system_prompt != system_prompt;
    if thinking_changed || prompt_changed {
        let _ = clear_response_cache(conn, id);
    }
    Ok(())
}

/// Persist the full model identity (provider + model) and context-window for a
/// session. Used so each session fully owns which service and model it targets,
/// independent of the global default.
pub fn set_provider_model_and_context(
    conn: &DbConn,
    id: &str,
    provider_id: Option<&str>,
    model: Option<&str>,
    context_window: Option<i64>,
) -> AppResult<()> {
    let trimmed_provider = provider_id.map(str::trim).filter(|s| !s.is_empty());
    let trimmed_model = model.map(str::trim).filter(|s| !s.is_empty());
    let updated = now_ms();
    // Provider/model changes invalidate any in-flight Responses cache chain.
    let n = conn.execute(
        "UPDATE sessions SET provider_id=?1, model=?2, context_window=?3,
                last_response_id=NULL, cache_thinking_key=NULL, updated_at=?4 WHERE id=?5",
        params![trimmed_provider, trimmed_model, context_window, updated, id],
    )?;
    if n == 0 {
        return Err(AppError::NotFound(format!("session {id}")));
    }
    Ok(())
}

/// Persist the latest Responses API cache chain tip for this session.
pub fn set_response_cache(
    conn: &DbConn,
    id: &str,
    last_response_id: Option<&str>,
    cache_thinking_key: Option<&str>,
) -> AppResult<()> {
    let updated = now_ms();
    let n = conn.execute(
        "UPDATE sessions SET last_response_id=?1, cache_thinking_key=?2, updated_at=?3 WHERE id=?4",
        params![last_response_id, cache_thinking_key, updated, id],
    )?;
    if n == 0 {
        return Err(AppError::NotFound(format!("session {id}")));
    }
    Ok(())
}

/// Drop the local Session-cache chain tip (does not call the remote DELETE API).
pub fn clear_response_cache(conn: &DbConn, id: &str) -> AppResult<()> {
    set_response_cache(conn, id, None, None)
}

/// Recompute `sessions.context_window_used` from stored messages.
///
/// Uses the **most recent assistant message's** prompt size as the context
/// window usage indicator. The prompt sent to the API already includes
/// everything in that request: system prompt, tool definitions, injected
/// context (CLAUDE.md / env block), full conversation history, and the current
/// user message. It therefore accurately represents how much of the context
/// window is filled and how much remains for future turns.
///
/// Preference order:
/// 1. `last_prompt_tokens` — the prompt of the *final* API call in the turn.
///    For turns that fan out into multiple tool-call rounds, `prompt_tokens`
///    (and hence `total_tokens`) is the *sum* of every round's prompt, which
///    massively over-counts the real occupancy. `last_prompt_tokens` is the
///    single most-recent request's prompt, so it reflects true occupancy.
/// 2. `total_tokens` (prompt + completion) — legacy fallback for messages
///    stored before `last_prompt_tokens` existed, and single-call turns.
/// 3. `prompt_tokens` — for providers that only expose the input side.
/// Falls back to 0 when none are available (e.g. image-generation APIs that
/// don't report usage).
pub fn recompute_context_window_used(conn: &DbConn, session_id: &str) -> AppResult<()> {
    let loaded = load_with_messages(conn, session_id)?;
    let mut used: i64 = 0;
    for msg in loaded.messages.iter().rev() {
        if msg.role != "assistant" {
            continue;
        }
        if let Some(ref p) = msg.params {
            let u = tokens::extract_usage(p);
            let t = u
                .last_prompt_tokens
                .filter(|x| *x > 0)
                .or_else(|| u.total_tokens.filter(|x| *x > 0))
                .or_else(|| u.prompt_tokens.filter(|x| *x > 0));
            if let Some(t) = t {
                used = t;
                break;
            }
        }
    }
    let updated = now_ms();
    conn.execute(
        "UPDATE sessions SET context_window_used=?1, updated_at=?2 WHERE id=?3",
        params![used, updated, session_id],
    )?;
    Ok(())
}

/// Fold a manual edit back into a message's block list.
///
/// `text` is the concatenation of every prose block (see
/// `finalize_generate_assistant_message`), which is also what the edit box is
/// seeded with — so the edit replaces all of them at once. The replacement goes
/// where the *last* prose block was, not the first: prose that trailed a tool
/// call has to stay behind it, both to read correctly and so timeline replay
/// still finds a final `Text` segment. Tool and AskUser blocks are untouched.
fn collapse_text_blocks(blocks: &[serde_json::Value], text: &str) -> Vec<serde_json::Value> {
    let is_text = |b: &serde_json::Value| b.get("type").and_then(|t| t.as_str()) == Some("text");
    let anchor = blocks.iter().rposition(is_text);
    let mut next: Vec<serde_json::Value> = blocks.iter().filter(|b| !is_text(b)).cloned().collect();
    if !text.is_empty() {
        let at = match anchor {
            // Non-text blocks ahead of the anchor keep their slots.
            Some(i) => blocks.iter().take(i).filter(|b| !is_text(b)).count(),
            None => next.len(),
        };
        next.insert(at, serde_json::json!({ "type": "text", "content": text }));
    }
    next
}

pub fn update_message_text(conn: &DbConn, id: &str, text: &str) -> AppResult<()> {
    let n = conn.execute("UPDATE messages SET text=?1 WHERE id=?2", params![text, id])?;
    if n == 0 {
        return Err(AppError::NotFound(format!("message {id}")));
    }
    // Keep interleaved AskUser/tool history, but replace prose text blocks so
    // manual edits stay visible after we prefer `blocks` rendering in the UI.
    if let Ok(Some(raw)) = conn.query_row(
        "SELECT params_json FROM messages WHERE id=?1",
        params![id],
        |r| r.get::<_, Option<String>>(0),
    ) {
        if let Ok(mut params_v) = serde_json::from_str::<serde_json::Value>(&raw) {
            if let Some(blocks) = params_v.get_mut("blocks").and_then(|b| b.as_array_mut()) {
                *blocks = collapse_text_blocks(blocks, text);
                if let Ok(s) = serde_json::to_string(&params_v) {
                    let _ = conn.execute(
                        "UPDATE messages SET params_json=?1 WHERE id=?2",
                        params![s, id],
                    );
                }
            }
        }
    }
    message_search::reindex_message(conn, id)?;
    Ok(())
}

pub fn update_message_params(conn: &DbConn, id: &str, params_json: &str) -> AppResult<()> {
    let n = conn.execute(
        "UPDATE messages SET params_json=?1 WHERE id=?2",
        params![params_json, id],
    )?;
    if n == 0 {
        return Err(AppError::NotFound(format!("message {id}")));
    }
    message_search::reindex_message(conn, id)?;
    Ok(())
}

/// Returns image rel_paths (and thumb rel_paths) that should be cleaned from disk.
pub fn delete_message(conn: &DbConn, id: &str) -> AppResult<Vec<(String, Option<String>)>> {
    let session_id: String = conn
        .query_row(
            "SELECT session_id FROM messages WHERE id=?1",
            params![id],
            |r| r.get(0),
        )
        .map_err(|_| AppError::NotFound(format!("message {id}")))?;
    let mut stmt =
        conn.prepare("SELECT rel_path, thumb_path FROM message_images WHERE message_id=?1")?;
    let rows = stmt.query_map(params![id], |r| {
        let rel: String = r.get(0)?;
        let thumb: Option<String> = r.get(1)?;
        Ok((rel, thumb))
    })?;
    let mut paths = Vec::new();
    for r in rows {
        paths.push(r?);
    }
    conn.execute(
        "DELETE FROM message_images WHERE message_id=?1",
        params![id],
    )?;
    let n = conn.execute("DELETE FROM messages WHERE id=?1", params![id])?;
    if n == 0 {
        return Err(AppError::NotFound(format!("message {id}")));
    }
    recompute_context_window_used(conn, &session_id)?;
    Ok(paths)
}

pub fn list(conn: &DbConn) -> AppResult<Vec<SessionSummary>> {
    let mut stmt = conn.prepare(
        "SELECT s.id, s.title, s.model, s.system_prompt, s.history_turns, s.llm_params, s.context_window, s.context_window_used, s.agent_type, s.updated_at,
            (SELECT COUNT(*) FROM messages m WHERE m.session_id = s.id) AS cnt,
            s.project_id, s.provider_id
         FROM sessions s
         WHERE COALESCE(s.is_temporary, 0) = 0
         ORDER BY s.updated_at DESC",
    )?;
    let rows = stmt.query_map(params![], |r| {
        let raw: Option<String> = r.get(5)?;
        Ok(SessionSummary {
            id: r.get(0)?,
            title: r.get(1)?,
            model: r.get(2)?,
            provider_id: r.get(12)?,
            system_prompt: r.get(3)?,
            history_turns: r.get(4)?,
            llm_params: decode_llm_params(raw),
            context_window: r.get(6)?,
            context_window_used: r.get(7)?,
            agent_type: r.get(8)?,
            updated_at: r.get(9)?,
            message_count: r.get(10)?,
            project_id: r.get(11)?,
        })
    })?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

fn escape_like(raw: &str) -> String {
    message_search::escape_like(raw)
}

pub fn search(conn: &DbConn, query: &str, limit: i64) -> AppResult<Vec<SessionSearchResult>> {
    let limit = if limit <= 0 { 20 } else { limit.min(50) };
    let query = query.trim();

    if query.is_empty() {
        let mut stmt = conn.prepare(
            "SELECT s.id, s.title, s.model, s.system_prompt, s.history_turns, s.llm_params, s.context_window, s.context_window_used, s.agent_type, s.updated_at,
                (SELECT COUNT(*) FROM messages m WHERE m.session_id = s.id) AS cnt,
                s.project_id,
                NULL AS match_message_id, NULL AS match_role, NULL AS match_text,
                NULL AS match_created_at, 0 AS match_count, 0 AS title_match,
                s.provider_id
             FROM sessions s
             WHERE COALESCE(s.is_temporary, 0) = 0
             ORDER BY s.updated_at DESC
             LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit], map_search_result)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        return Ok(out);
    }

    let use_fts = message_search::query_char_len(query) >= 3;

    if use_fts {
        let fts_q = message_search::escape_fts_query(query);
        let mut stmt = conn.prepare(
            "SELECT s.id, s.title, s.model, s.system_prompt, s.history_turns, s.llm_params, s.context_window, s.context_window_used, s.agent_type, s.updated_at,
                (SELECT COUNT(*) FROM messages m WHERE m.session_id = s.id) AS cnt,
                s.project_id,
                (SELECT mm.id FROM messages mm
                 JOIN messages_fts ON messages_fts.rowid = mm.rowid
                 WHERE mm.session_id = s.id AND messages_fts MATCH ?1
                 ORDER BY mm.created_at DESC LIMIT 1) AS match_message_id,
                (SELECT mm.role FROM messages mm
                 JOIN messages_fts ON messages_fts.rowid = mm.rowid
                 WHERE mm.session_id = s.id AND messages_fts MATCH ?1
                 ORDER BY mm.created_at DESC LIMIT 1) AS match_role,
                (SELECT mm.searchable_text FROM messages mm
                 JOIN messages_fts ON messages_fts.rowid = mm.rowid
                 WHERE mm.session_id = s.id AND messages_fts MATCH ?1
                 ORDER BY mm.created_at DESC LIMIT 1) AS match_text,
                (SELECT mm.created_at FROM messages mm
                 JOIN messages_fts ON messages_fts.rowid = mm.rowid
                 WHERE mm.session_id = s.id AND messages_fts MATCH ?1
                 ORDER BY mm.created_at DESC LIMIT 1) AS match_created_at,
                (SELECT COUNT(*) FROM messages mc
                 JOIN messages_fts ON messages_fts.rowid = mc.rowid
                 WHERE mc.session_id = s.id AND messages_fts MATCH ?1) AS match_count,
                CASE WHEN EXISTS (
                    SELECT 1 FROM sessions_fts WHERE sessions_fts.rowid = s.rowid AND sessions_fts MATCH ?1
                ) THEN 1 ELSE 0 END AS title_match,
                s.provider_id
             FROM sessions s
             WHERE COALESCE(s.is_temporary, 0) = 0
               AND (
                 EXISTS (
                    SELECT 1 FROM sessions_fts
                    WHERE sessions_fts.rowid = s.rowid AND sessions_fts MATCH ?1
                 )
                 OR EXISTS (
                    SELECT 1 FROM messages mx
                    JOIN messages_fts ON messages_fts.rowid = mx.rowid
                    WHERE mx.session_id = s.id AND messages_fts MATCH ?1
                 )
               )
             ORDER BY
                CASE WHEN EXISTS (
                    SELECT 1 FROM sessions_fts
                    WHERE sessions_fts.rowid = s.rowid AND sessions_fts MATCH ?1
                ) THEN 0 ELSE 1 END,
                s.updated_at DESC
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![fts_q, limit], map_search_result)?;
        let mut out = Vec::new();
        for r in rows {
            let mut item = r?;
            if let Some(ref body) = item.match_text {
                item.match_text = Some(message_search::make_snippet(body, query));
            }
            out.push(item);
        }
        return Ok(out);
    }

    let pattern = format!("%{}%", escape_like(query));
    let mut stmt = conn.prepare(
        "SELECT s.id, s.title, s.model, s.system_prompt, s.history_turns, s.llm_params, s.context_window, s.context_window_used, s.agent_type, s.updated_at,
            (SELECT COUNT(*) FROM messages m WHERE m.session_id = s.id) AS cnt,
            s.project_id,
            (SELECT mm.id FROM messages mm
             WHERE mm.session_id = s.id AND COALESCE(mm.searchable_text, '') LIKE ?1 ESCAPE '\\'
             ORDER BY mm.created_at DESC LIMIT 1) AS match_message_id,
            (SELECT mm.role FROM messages mm
             WHERE mm.session_id = s.id AND COALESCE(mm.searchable_text, '') LIKE ?1 ESCAPE '\\'
             ORDER BY mm.created_at DESC LIMIT 1) AS match_role,
            (SELECT mm.searchable_text FROM messages mm
             WHERE mm.session_id = s.id AND COALESCE(mm.searchable_text, '') LIKE ?1 ESCAPE '\\'
             ORDER BY mm.created_at DESC LIMIT 1) AS match_text,
            (SELECT mm.created_at FROM messages mm
             WHERE mm.session_id = s.id AND COALESCE(mm.searchable_text, '') LIKE ?1 ESCAPE '\\'
             ORDER BY mm.created_at DESC LIMIT 1) AS match_created_at,
            (SELECT COUNT(*) FROM messages mc
             WHERE mc.session_id = s.id AND COALESCE(mc.searchable_text, '') LIKE ?1 ESCAPE '\\') AS match_count,
            CASE WHEN s.title LIKE ?1 ESCAPE '\\' THEN 1 ELSE 0 END AS title_match,
            s.provider_id
         FROM sessions s
         WHERE COALESCE(s.is_temporary, 0) = 0
           AND (
             s.title LIKE ?1 ESCAPE '\\'
             OR EXISTS (
                 SELECT 1 FROM messages mx
                 WHERE mx.session_id = s.id AND COALESCE(mx.searchable_text, '') LIKE ?1 ESCAPE '\\'
             )
           )
         ORDER BY
            CASE WHEN s.title LIKE ?1 ESCAPE '\\' THEN 0 ELSE 1 END,
            s.updated_at DESC
         LIMIT ?2",
    )?;
    let rows = stmt.query_map(params![pattern, limit], map_search_result)?;
    let mut out = Vec::new();
    for r in rows {
        let mut item = r?;
        if let Some(ref body) = item.match_text {
            item.match_text = Some(message_search::make_snippet(body, query));
        }
        out.push(item);
    }
    Ok(out)
}

fn map_search_result(row: &rusqlite::Row<'_>) -> rusqlite::Result<SessionSearchResult> {
    let title_match: i64 = row.get(17)?;
    let raw: Option<String> = row.get(5)?;
    Ok(SessionSearchResult {
        id: row.get(0)?,
        title: row.get(1)?,
        model: row.get(2)?,
        provider_id: row.get(18)?,
        system_prompt: row.get(3)?,
        history_turns: row.get(4)?,
        llm_params: decode_llm_params(raw),
        context_window: row.get(6)?,
        context_window_used: row.get(7)?,
        agent_type: row.get(8)?,
        updated_at: row.get(9)?,
        message_count: row.get(10)?,
        project_id: row.get(11)?,
        match_message_id: row.get(12)?,
        match_role: row.get(13)?,
        match_text: row.get(14)?,
        match_created_at: row.get(15)?,
        match_count: row.get(16)?,
        title_match: title_match != 0,
    })
}

pub fn get(conn: &DbConn, id: &str) -> AppResult<Session> {
    let mut stmt = conn.prepare(
        "SELECT id, title, model, system_prompt, history_turns, llm_params, context_window, context_window_used, agent_type, project_id, created_at, updated_at, agent_chain, provider_id,
                parent_session_id, is_temporary, spawn_task_id, last_response_id, cache_thinking_key
         FROM sessions WHERE id=?1",
    )?;
    let mut rows = stmt.query(params![id])?;
    if let Some(row) = rows.next()? {
        let raw: Option<String> = row.get(5)?;
        let chain_raw: Option<String> = row.get(12)?;
        let is_temporary: i64 = row.get(15).unwrap_or(0);
        Ok(Session {
            id: row.get(0)?,
            title: row.get(1)?,
            model: row.get(2)?,
            provider_id: row.get(13)?,
            system_prompt: row.get(3)?,
            history_turns: row.get(4)?,
            llm_params: decode_llm_params(raw),
            context_window: row.get(6)?,
            context_window_used: row.get(7)?,
            agent_type: row.get(8)?,
            agent_chain: decode_agent_chain(chain_raw),
            project_id: row.get(9)?,
            parent_session_id: row.get(14)?,
            is_temporary: is_temporary != 0,
            spawn_task_id: row.get(16)?,
            last_response_id: row.get(17)?,
            cache_thinking_key: row.get(18)?,
            created_at: row.get(10)?,
            updated_at: row.get(11)?,
        })
    } else {
        Err(AppError::NotFound(format!("session {id}")))
    }
}

pub fn touch(conn: &DbConn, id: &str) -> AppResult<()> {
    conn.execute(
        "UPDATE sessions SET updated_at=?1 WHERE id=?2",
        params![now_ms(), id],
    )?;
    Ok(())
}

pub fn insert_message(
    conn: &DbConn,
    session_id: &str,
    role: &str,
    text: Option<&str>,
    params_json: Option<&str>,
) -> AppResult<Message> {
    let id = Ulid::new().to_string();
    let now = now_ms();
    let params_v: Option<serde_json::Value> = match params_json {
        Some(s) => serde_json::from_str(s).ok(),
        None => None,
    };
    let searchable = message_search::build_searchable_text(
        text,
        params_v.as_ref().unwrap_or(&serde_json::Value::Null),
    );
    conn.execute(
        "INSERT INTO messages(id, session_id, role, text, params_json, created_at, searchable_text) VALUES(?1,?2,?3,?4,?5,?6,?7)",
        params![id, session_id, role, text, params_json, now, searchable],
    )?;
    let _ = message_search::index_new_message(conn, &id);
    touch(conn, session_id)?;
    Ok(Message {
        id,
        session_id: session_id.into(),
        role: role.into(),
        text: text.map(|s| s.to_string()),
        params: params_v,
        created_at: now,
        images: vec![],
    })
}

pub fn insert_image(
    conn: &DbConn,
    session_id: &str,
    message_id: Option<&str>,
    role: &str,
    rel_path: &str,
    thumb_rel_path: Option<&str>,
    mime: &str,
    width: Option<u32>,
    height: Option<u32>,
    bytes: Option<u64>,
    ord: i64,
) -> AppResult<ImageRef> {
    insert_media(
        conn,
        session_id,
        message_id,
        role,
        rel_path,
        thumb_rel_path,
        mime,
        None,
        None,
        width,
        height,
        bytes,
        ord,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn insert_media(
    conn: &DbConn,
    session_id: &str,
    message_id: Option<&str>,
    role: &str,
    rel_path: &str,
    thumb_rel_path: Option<&str>,
    mime: &str,
    media_role: Option<&str>,
    source_url: Option<&str>,
    width: Option<u32>,
    height: Option<u32>,
    bytes: Option<u64>,
    ord: i64,
) -> AppResult<ImageRef> {
    let id = Ulid::new().to_string();
    let now = now_ms();
    conn.execute(
        "INSERT INTO message_images(id, message_id, session_id, role, rel_path, thumb_path, mime, media_role, source_url, width, height, bytes, ord, created_at)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)",
        params![
            id,
            message_id,
            session_id,
            role,
            rel_path,
            thumb_rel_path,
            mime,
            media_role,
            source_url,
            width.map(|v| v as i64),
            height.map(|v| v as i64),
            bytes.map(|v| v as i64),
            ord,
            now
        ],
    )?;
    Ok(ImageRef {
        id,
        role: role.into(),
        rel_path: rel_path.into(),
        thumb_rel_path: thumb_rel_path.map(|s| s.to_string()),
        mime: mime.into(),
        media_role: media_role.map(str::to_string),
        source_url: source_url.map(str::to_string),
        width: width.map(|v| v as i64),
        height: height.map(|v| v as i64),
        bytes: bytes.map(|v| v as i64),
        ord,
    })
}

pub fn set_image_media_role(conn: &DbConn, id: &str, media_role: Option<&str>) -> AppResult<()> {
    conn.execute(
        "UPDATE message_images SET media_role=?1 WHERE id=?2",
        params![media_role, id],
    )?;
    Ok(())
}

pub fn bind_images_to_message(
    conn: &DbConn,
    message_id: &str,
    image_ids: &[String],
) -> AppResult<()> {
    for id in image_ids {
        conn.execute(
            "UPDATE message_images SET message_id=?1 WHERE id=?2",
            params![message_id, id],
        )?;
    }
    Ok(())
}

/// Replace the set of `input` images on a message with the given ordered list of image ids.
/// Each id must already exist in `message_images` and must either be unbound (a draft)
/// or already bound to this message; all must be in the same session.
/// Returns (rel_path, thumb_path) pairs for images that were removed and should be cleaned from disk.
pub fn update_message_input_images(
    conn: &DbConn,
    message_id: &str,
    new_image_ids: &[String],
) -> AppResult<Vec<(String, Option<String>)>> {
    let session_id: String = match conn.query_row(
        "SELECT session_id FROM messages WHERE id=?1",
        params![message_id],
        |r| r.get(0),
    ) {
        Ok(s) => s,
        Err(rusqlite::Error::QueryReturnedNoRows) => {
            return Err(AppError::NotFound(format!("message {message_id}")));
        }
        Err(e) => return Err(e.into()),
    };

    let mut current: Vec<(String, String, Option<String>)> = Vec::new();
    {
        let mut stmt = conn.prepare(
            "SELECT id, rel_path, thumb_path FROM message_images
             WHERE message_id=?1 AND role='input'",
        )?;
        let rows = stmt.query_map(params![message_id], |r| {
            let id: String = r.get(0)?;
            let rel: String = r.get(1)?;
            let thumb: Option<String> = r.get(2)?;
            Ok((id, rel, thumb))
        })?;
        for r in rows {
            current.push(r?);
        }
    }

    let new_set: std::collections::HashSet<&str> =
        new_image_ids.iter().map(|s| s.as_str()).collect();

    for image_id in new_image_ids {
        let row: Result<(String, Option<String>, String), rusqlite::Error> = conn.query_row(
            "SELECT session_id, message_id, role FROM message_images WHERE id=?1",
            params![image_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        );
        match row {
            Ok((sid, mid, role)) => {
                if sid != session_id {
                    return Err(AppError::Invalid(format!(
                        "image {image_id} not in session"
                    )));
                }
                match mid {
                    None => {}
                    Some(ref m) if m == message_id => {}
                    Some(_) => {
                        return Err(AppError::Invalid(format!(
                            "image {image_id} bound to another message"
                        )));
                    }
                }
                if role != "input" {
                    return Err(AppError::Invalid(format!(
                        "image {image_id} is not an input image"
                    )));
                }
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => {
                return Err(AppError::NotFound(format!("image {image_id}")));
            }
            Err(e) => return Err(e.into()),
        }
    }

    let mut removed: Vec<(String, Option<String>)> = Vec::new();
    for (id, rel, thumb) in &current {
        if !new_set.contains(id.as_str()) {
            conn.execute("DELETE FROM message_images WHERE id=?1", params![id])?;
            removed.push((rel.clone(), thumb.clone()));
        }
    }

    for (i, image_id) in new_image_ids.iter().enumerate() {
        conn.execute(
            "UPDATE message_images SET message_id=?1, ord=?2, role='input' WHERE id=?3",
            params![message_id, i as i64, image_id],
        )?;
    }

    touch(conn, &session_id)?;
    Ok(removed)
}

pub fn get_image(conn: &DbConn, id: &str) -> AppResult<ImageRef> {
    let mut stmt = conn.prepare(
        "SELECT id, role, rel_path, thumb_path, mime, media_role, source_url, width, height, bytes, ord
         FROM message_images WHERE id=?1",
    )?;
    let mut rows = stmt.query(params![id])?;
    if let Some(r) = rows.next()? {
        Ok(ImageRef {
            id: r.get(0)?,
            role: r.get(1)?,
            rel_path: r.get(2)?,
            thumb_rel_path: r.get(3)?,
            mime: r.get(4)?,
            media_role: r.get(5)?,
            source_url: r.get(6)?,
            width: r.get(7)?,
            height: r.get(8)?,
            bytes: r.get(9)?,
            ord: r.get(10)?,
        })
    } else {
        Err(AppError::NotFound(format!("image {id}")))
    }
}

/// Delete message_images rows by id. Does not delete the parent message.
/// Returns (rel_path, thumb_path) pairs that should be cleaned from disk.
pub fn delete_images(
    conn: &DbConn,
    ids: &[String],
) -> AppResult<Vec<(String, Option<String>)>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }

    let mut removed: Vec<(String, Option<String>)> = Vec::new();
    let mut session_ids: std::collections::HashSet<String> = std::collections::HashSet::new();

    for id in ids {
        let row: Result<(String, String, Option<String>), rusqlite::Error> = conn.query_row(
            "SELECT session_id, rel_path, thumb_path FROM message_images WHERE id=?1",
            params![id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        );
        match row {
            Ok((sid, rel, thumb)) => {
                session_ids.insert(sid);
                conn.execute("DELETE FROM message_images WHERE id=?1", params![id])?;
                removed.push((rel, thumb));
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => {
                // Already gone — skip.
            }
            Err(e) => return Err(e.into()),
        }
    }

    for sid in session_ids {
        touch(conn, &sid)?;
    }
    Ok(removed)
}

pub fn image_session_id(conn: &DbConn, id: &str) -> AppResult<String> {
    let mut stmt = conn.prepare("SELECT session_id FROM message_images WHERE id=?1")?;
    let mut rows = stmt.query(params![id])?;
    if let Some(r) = rows.next()? {
        Ok(r.get(0)?)
    } else {
        Err(AppError::NotFound(format!("image {id}")))
    }
}

fn load_message_images(conn: &DbConn, message_id: &str) -> AppResult<Vec<ImageRef>> {
    let mut stmt = conn.prepare(
        "SELECT id, role, rel_path, thumb_path, mime, media_role, source_url, width, height, bytes, ord
         FROM message_images WHERE message_id=?1 ORDER BY ord ASC",
    )?;
    let rows = stmt.query_map(params![message_id], |r| {
        Ok(ImageRef {
            id: r.get(0)?,
            role: r.get(1)?,
            rel_path: r.get(2)?,
            thumb_rel_path: r.get(3)?,
            mime: r.get(4)?,
            media_role: r.get(5)?,
            source_url: r.get(6)?,
            width: r.get(7)?,
            height: r.get(8)?,
            bytes: r.get(9)?,
            ord: r.get(10)?,
        })
    })?;
    let mut v = Vec::new();
    for r in rows {
        v.push(r?);
    }
    Ok(v)
}

pub fn load_with_messages(conn: &DbConn, session_id: &str) -> AppResult<SessionWithMessages> {
    let session = get(conn, session_id)?;
    let parent_title = parent_title_of(conn, &session)?;
    let messages = load_all_messages(conn, session_id)?;
    Ok(SessionWithMessages {
        session,
        messages,
        parent_title,
    })
}

fn parent_title_of(conn: &DbConn, session: &Session) -> AppResult<Option<String>> {
    Ok(match session.parent_session_id.as_deref() {
        Some(pid) => conn
            .query_row(
                "SELECT title FROM sessions WHERE id=?1",
                params![pid],
                |r| r.get::<_, String>(0),
            )
            .ok(),
        None => None,
    })
}

fn collapse_ws(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev_space = false;
    for ch in s.chars() {
        if ch.is_whitespace() {
            if !prev_space && !out.is_empty() {
                out.push(' ');
                prev_space = true;
            }
        } else {
            out.push(ch);
            prev_space = false;
        }
    }
    out.trim().to_string()
}

fn outline_preview(role: &str, text: Option<&str>) -> Option<String> {
    let raw = text.map(str::trim).filter(|s| !s.is_empty())?;
    let mut collapsed = collapse_ws(raw);
    // Drop fenced code block openers so previews stay readable.
    if collapsed.starts_with("```") {
        if let Some(rest) = collapsed.split_once('\n').map(|(_, r)| r.trim()) {
            collapsed = collapse_ws(rest);
        }
    }
    if collapsed.is_empty() {
        return fallback_preview(role);
    }
    let mut chars = collapsed.chars();
    let taken: String = chars.by_ref().take(OUTLINE_PREVIEW_CHARS).collect();
    if chars.next().is_some() {
        Some(format!("{taken}…"))
    } else {
        Some(taken)
    }
}

fn fallback_preview(role: &str) -> Option<String> {
    match role {
        "assistant" => Some("(tools/media)".into()),
        "user" => Some("(attachment)".into()),
        "error" => Some("(error)".into()),
        _ => None,
    }
}

/// Full lightweight outline for a session (id/role/preview/created_at only).
pub fn list_message_outline(conn: &DbConn, session_id: &str) -> AppResult<Vec<MessageOutlineItem>> {
    let _ = get(conn, session_id)?;
    let mut stmt = conn.prepare(
        "SELECT id, role, text, created_at FROM messages
         WHERE session_id=?1 ORDER BY created_at ASC",
    )?;
    let rows = stmt.query_map(params![session_id], |r| {
        let id: String = r.get(0)?;
        let role: String = r.get(1)?;
        let text: Option<String> = r.get(2)?;
        let created_at: i64 = r.get(3)?;
        Ok(MessageOutlineItem {
            preview: outline_preview(&role, text.as_deref()).or_else(|| {
                // Empty text: still surface a stub for non-empty roles that often
                // only carry images / tool blocks.
                if matches!(role.as_str(), "user" | "assistant" | "error") {
                    fallback_preview(&role)
                } else {
                    None
                }
            }),
            id,
            role,
            created_at,
        })
    })?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

fn map_message_row(session_id: &str, r: &rusqlite::Row<'_>) -> rusqlite::Result<Message> {
    let params_str: Option<String> = r.get(3)?;
    Ok(Message {
        id: r.get(0)?,
        session_id: session_id.into(),
        role: r.get(1)?,
        text: r.get(2)?,
        params: params_str.and_then(|s| serde_json::from_str(&s).ok()),
        created_at: r.get(4)?,
        images: vec![],
    })
}

fn attach_images(conn: &DbConn, mut messages: Vec<Message>) -> AppResult<Vec<Message>> {
    for m in &mut messages {
        m.images = load_message_images(conn, &m.id)?;
    }
    Ok(messages)
}

fn load_all_messages(conn: &DbConn, session_id: &str) -> AppResult<Vec<Message>> {
    let mut stmt = conn.prepare(
        "SELECT id, role, text, params_json, created_at FROM messages
         WHERE session_id=?1 ORDER BY created_at ASC",
    )?;
    let rows = stmt.query_map(params![session_id], |r| map_message_row(session_id, r))?;
    let mut v = Vec::new();
    for r in rows {
        v.push(r?);
    }
    attach_images(conn, v)
}

/// Load messages in created_at order with optional window bounds.
///
/// - `before_created_at`: strictly older than this timestamp (exclusive)
/// - `after_created_at`: strictly newer than this timestamp (exclusive)
/// - `around_message_id`: center a bidirectional window on this message
/// - `limit`: max rows (clamped 1..500)
pub fn load_messages_ordered(
    conn: &DbConn,
    session_id: &str,
    around_message_id: Option<&str>,
    before_created_at: Option<i64>,
    after_created_at: Option<i64>,
    limit: i64,
) -> AppResult<Vec<Message>> {
    let limit = if limit <= 0 {
        60
    } else {
        limit.min(500)
    };

    // A missing anchor is expected: the caller's anchor may have just been
    // deleted (message delete / resend truncation). Fall back to the tail
    // window instead of failing the whole load.
    let anchor_created = match around_message_id {
        Some(mid) => conn
            .query_row(
                "SELECT created_at FROM messages WHERE id=?1 AND session_id=?2",
                params![mid, session_id],
                |r| r.get::<_, i64>(0),
            )
            .optional()?,
        None => None,
    };

    if let Some(anchor_created) = anchor_created {
        let before_n = limit / 2;
        let after_n = limit.saturating_sub(before_n);

        let mut before_stmt = conn.prepare(
            "SELECT id, role, text, params_json, created_at FROM messages
             WHERE session_id=?1 AND created_at < ?2
             ORDER BY created_at DESC LIMIT ?3",
        )?;
        let before_rows = before_stmt.query_map(params![session_id, anchor_created, before_n], |r| {
            map_message_row(session_id, r)
        })?;
        let mut before = Vec::new();
        for r in before_rows {
            before.push(r?);
        }
        before.reverse();

        let mut mid_stmt = conn.prepare(
            "SELECT id, role, text, params_json, created_at FROM messages
             WHERE session_id=?1 AND created_at >= ?2
             ORDER BY created_at ASC LIMIT ?3",
        )?;
        let mid_rows =
            mid_stmt.query_map(params![session_id, anchor_created, after_n.max(1)], |r| {
                map_message_row(session_id, r)
            })?;
        let mut after = Vec::new();
        for r in mid_rows {
            after.push(r?);
        }

        let mut merged = before;
        merged.extend(after);
        return attach_images(conn, merged);
    }

    // Tail / before / after window
    if let Some(before) = before_created_at {
        let mut stmt = conn.prepare(
            "SELECT id, role, text, params_json, created_at FROM messages
             WHERE session_id=?1 AND created_at < ?2
             ORDER BY created_at DESC LIMIT ?3",
        )?;
        let rows = stmt.query_map(params![session_id, before, limit], |r| {
            map_message_row(session_id, r)
        })?;
        let mut v = Vec::new();
        for r in rows {
            v.push(r?);
        }
        v.reverse();
        return attach_images(conn, v);
    }

    if let Some(after) = after_created_at {
        let mut stmt = conn.prepare(
            "SELECT id, role, text, params_json, created_at FROM messages
             WHERE session_id=?1 AND created_at > ?2
             ORDER BY created_at ASC LIMIT ?3",
        )?;
        let rows = stmt.query_map(params![session_id, after, limit], |r| {
            map_message_row(session_id, r)
        })?;
        let mut v = Vec::new();
        for r in rows {
            v.push(r?);
        }
        return attach_images(conn, v);
    }

    // Default: last `limit` messages.
    let mut stmt = conn.prepare(
        "SELECT id, role, text, params_json, created_at FROM messages
         WHERE session_id=?1
         ORDER BY created_at DESC LIMIT ?2",
    )?;
    let rows = stmt.query_map(params![session_id, limit], |r| map_message_row(session_id, r))?;
    let mut v = Vec::new();
    for r in rows {
        v.push(r?);
    }
    v.reverse();
    attach_images(conn, v)
}

/// Session shell + a window of messages (default: last `limit`).
pub fn load_with_message_window(
    conn: &DbConn,
    session_id: &str,
    around_message_id: Option<&str>,
    limit: i64,
) -> AppResult<SessionWithMessages> {
    let session = get(conn, session_id)?;
    let parent_title = parent_title_of(conn, &session)?;
    let messages = load_messages_ordered(
        conn,
        session_id,
        around_message_id,
        None,
        None,
        limit,
    )?;
    Ok(SessionWithMessages {
        session,
        messages,
        parent_title,
    })
}

/// All media rows for a session (gallery), ordered by message time then ord.
pub fn list_session_media(conn: &DbConn, session_id: &str) -> AppResult<Vec<ImageRef>> {
    let _ = get(conn, session_id)?;
    let mut stmt = conn.prepare(
        "SELECT mi.id, mi.role, mi.rel_path, mi.thumb_path, mi.mime, mi.media_role,
                mi.source_url, mi.width, mi.height, mi.bytes, mi.ord
         FROM message_images mi
         INNER JOIN messages m ON m.id = mi.message_id
         WHERE mi.session_id=?1 AND mi.message_id IS NOT NULL
         ORDER BY m.created_at ASC, mi.ord ASC",
    )?;
    let rows = stmt.query_map(params![session_id], |r| {
        Ok(ImageRef {
            id: r.get(0)?,
            role: r.get(1)?,
            rel_path: r.get(2)?,
            thumb_rel_path: r.get(3)?,
            mime: r.get(4)?,
            media_role: r.get(5)?,
            source_url: r.get(6)?,
            width: r.get(7)?,
            height: r.get(8)?,
            bytes: r.get(9)?,
            ord: r.get(10)?,
        })
    })?;
    let mut v = Vec::new();
    for r in rows {
        v.push(r?);
    }
    Ok(v)
}

#[cfg(test)]
mod collapse_text_blocks_tests {
    use super::collapse_text_blocks;
    use serde_json::json;

    fn types(blocks: &[serde_json::Value]) -> Vec<&str> {
        blocks
            .iter()
            .map(|b| b["type"].as_str().unwrap_or(""))
            .collect()
    }

    #[test]
    fn edited_prose_stays_behind_the_tool_call_it_describes() {
        let blocks = vec![
            json!({"type":"text","content":"先看一下"}),
            json!({"type":"tool_use","id":"c1","tool":"Read"}),
            json!({"type":"text","content":"看完了"}),
        ];
        let next = collapse_text_blocks(&blocks, "先看一下\n看完了（改）");
        assert_eq!(types(&next), vec!["tool_use", "text"]);
        assert_eq!(next[1]["content"], "先看一下\n看完了（改）");
    }

    #[test]
    fn tool_blocks_survive_the_edit() {
        let blocks = vec![
            json!({"type":"tool_use","id":"c1","tool":"Read"}),
            json!({"type":"tool_use","id":"c2","tool":"Edit"}),
            json!({"type":"text","content":"done"}),
        ];
        let next = collapse_text_blocks(&blocks, "改好了");
        assert_eq!(types(&next), vec!["tool_use", "tool_use", "text"]);
        assert_eq!(next[0]["id"], "c1");
        assert_eq!(next[1]["id"], "c2");
    }

    #[test]
    fn a_message_with_no_prose_gains_a_trailing_block() {
        let blocks = vec![json!({"type":"tool_use","id":"c1","tool":"Read"})];
        let next = collapse_text_blocks(&blocks, "补一句");
        assert_eq!(types(&next), vec!["tool_use", "text"]);
    }

    #[test]
    fn clearing_the_text_drops_prose_without_touching_tools() {
        let blocks = vec![
            json!({"type":"text","content":"a"}),
            json!({"type":"tool_use","id":"c1","tool":"Read"}),
            json!({"type":"text","content":"b"}),
        ];
        let next = collapse_text_blocks(&blocks, "");
        assert_eq!(types(&next), vec!["tool_use"]);
    }
}
