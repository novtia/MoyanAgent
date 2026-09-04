use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tauri::{AppHandle, Emitter};

use crate::ai::agent::tools::agent_tool::{
    ChatRequestFactory, ChildStreamHooks, SpawnedTempSession, SubagentSessionHost,
};
use crate::ai::agent::{self, RoleStateStore, RunAgentResult};
use crate::ai::{chat, parameters, session_log, token_log};
use crate::app::generation::params::{effective_agent_chain, resolve_session_generation};
use crate::app::project_rules;
use crate::app::reader_paths::session_project_cwd;
use crate::data::db::DbPool;
use crate::data::{session, settings};
use crate::error::{AppError, AppResult};

use super::generation::commands::finalize_generate_assistant_message;
use super::generation::streaming::{
    new_stream_blocks, persist_streamed_assistant_snapshot, snapshot_stream_blocks,
    stream_text_callback, tool_event_callback, StreamBlocks,
};

pub(crate) struct SettingsChatFactory {
    pub(crate) pool: DbPool,
}

impl SettingsChatFactory {
    pub(crate) fn new(pool: DbPool) -> Self {
        Self { pool }
    }
}

impl ChatRequestFactory for SettingsChatFactory {
    fn build(
        &self,
        prompt: &str,
        agent_type: &str,
        definition: &crate::ai::agent::AgentDefinition,
        session_id: Option<&str>,
    ) -> AppResult<(chat::ChatRequest, Vec<agent::Attachment>)> {
        let conn = self.pool.get()?;
        let settings = settings::read(&conn)?;

        // Prefer the parent session's model/provider so ConsultRoles / Agent
        // tool matches the conversation the user actually selected — not the
        // global default (which may be a different model entirely).
        let (mut provider, mut model) = if let Some(sid) = session_id {
            match session::get(&conn, sid) {
                Ok(sess) => match resolve_session_generation(&conn, &settings, &sess) {
                    Ok(resolved) => {
                        // Per-node agent-flow override for this agent_type wins
                        // over the bare session model when present.
                        let chain_model =
                            effective_agent_chain(&conn, &sess)
                                .as_ref()
                                .and_then(|chain| {
                                    chain.iter().find_map(|node| {
                                        if node.agent_type != agent_type {
                                            return None;
                                        }
                                        node.effective_overrides()
                                            .and_then(|ov| ov.model.as_deref())
                                            .map(str::trim)
                                            .filter(|m| !m.is_empty())
                                            .map(|m| m.to_string())
                                    })
                                });
                        match chain_model {
                            Some(m) => {
                                if let Some(p) = settings::find_provider_for_model(&settings, &m) {
                                    (p.clone(), m)
                                } else {
                                    (resolved.provider, resolved.model)
                                }
                            }
                            None => (resolved.provider, resolved.model),
                        }
                    }
                    Err(_) => (
                        settings::active_provider(&settings)
                            .cloned()
                            .ok_or_else(|| {
                                AppError::Config("no enabled model provider configured".into())
                            })?,
                        settings.model.clone(),
                    ),
                },
                Err(_) => (
                    settings::active_provider(&settings)
                        .cloned()
                        .ok_or_else(|| {
                            AppError::Config("no enabled model provider configured".into())
                        })?,
                    settings.model.clone(),
                ),
            }
        } else {
            (
                settings::active_provider(&settings)
                    .cloned()
                    .ok_or_else(|| {
                        AppError::Config("no enabled model provider configured".into())
                    })?,
                settings.model.clone(),
            )
        };

        // Agent definition model override (custom agents / built-in edits).
        if let Some(m) = definition
            .model
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            if let Some(p) = settings::find_provider_for_model(&settings, m) {
                provider = p.clone();
                model = m.to_string();
            }
        }

        // Runner overwrites `system_prompt` with `definition.system_prompt`
        // plus env-details + critical reminder, so leave it empty here.
        let mut chat = crate::ai::router::build_chat_request(
            &provider,
            &model,
            prompt.to_string(),
            Vec::new(),
            String::new(),
            Vec::new(),
            crate::ai::parameters::factory().build(
                String::new(),
                String::new(),
                Default::default(),
            ),
        )?;
        // The model may differ from the parent session's (definition override,
        // chain override), so the window is resolved for whichever model this
        // sub-agent actually ended up on. Resolution always lands on a concrete
        // value: a sub-agent running without a window enforces no budget at
        // all, which is exactly how one Read of a long manuscript used to walk
        // straight into a context-length rejection.
        chat.context_window = Some(crate::data::llm_catalog::resolve_context_window(
            &conn,
            &provider.id,
            &provider.sdk,
            &model,
        ));

        let cwd = session_id.and_then(|sid| session_project_cwd(&conn, sid));
        project_rules::prepend_project_rules(&mut chat.history, cwd.as_deref());

        Ok((chat, Vec::new()))
    }
}

/// Host bridge that materialises temporary child sessions for `Agent` tool
/// dispatches and streams the sub-agent's events into that child.
pub(crate) struct TauriSubagentHost {
    pub(crate) app: AppHandle,
    pub(crate) pool: DbPool,
    pub(crate) role_states: Arc<RoleStateStore>,
    pub(crate) token_stats: Arc<token_log::TokenStatsRecorder>,
    pub(crate) session_logger: Arc<session_log::SessionLogger>,
    /// Per-child stream block buffers drained at finalize.
    pub(crate) child_blocks: Mutex<HashMap<String, StreamBlocks>>,
}

impl TauriSubagentHost {
    pub(crate) fn new(
        app: AppHandle,
        pool: DbPool,
        role_states: Arc<RoleStateStore>,
        token_stats: Arc<token_log::TokenStatsRecorder>,
        session_logger: Arc<session_log::SessionLogger>,
    ) -> Self {
        Self {
            app,
            pool,
            role_states,
            token_stats,
            session_logger,
            child_blocks: Mutex::new(HashMap::new()),
        }
    }

    pub(crate) fn take_blocks(&self, child_session_id: &str) -> Vec<serde_json::Value> {
        let blocks = self
            .child_blocks
            .lock()
            .ok()
            .and_then(|mut g| g.remove(child_session_id));
        match blocks {
            Some(b) => snapshot_stream_blocks(&b),
            None => Vec::new(),
        }
    }
}

impl SubagentSessionHost for TauriSubagentHost {
    fn prepare_temp_session(
        &self,
        parent_session_id: &str,
        parent_request_message_id: Option<&str>,
        tool_call_id: &str,
        title: &str,
        prompt: &str,
    ) -> AppResult<SpawnedTempSession> {
        let conn = self.pool.get()?;
        let child = session::create_temp(&conn, parent_session_id, title, None)?;
        let params = serde_json::json!({ "spawned_prompt": true }).to_string();
        let user_msg = session::insert_message(
            &conn,
            &child.id,
            "user",
            Some(prompt),
            Some(params.as_str()),
        )?;

        // Mid-run draft onto the parent Agent tool_use so the card can jump
        // into the child before the sub-agent finishes.
        if let Some(req_id) = parent_request_message_id {
            let _ = self.app.emit(
                "gen://tool",
                serde_json::json!({
                    "session_id": parent_session_id,
                    "request_message_id": req_id,
                    "type": "tool_result",
                    "id": tool_call_id,
                    "tool": "Agent",
                    "output": {
                        "status": "running",
                        "child_session_id": &child.id,
                    },
                    "is_error": false,
                    "keep_pending": true,
                }),
            );
        }

        let _ = self.app.emit(
            "gen://status",
            serde_json::json!({
                "phase": "request",
                "session_id": &child.id,
                "message_id": &user_msg.id,
            }),
        );

        Ok(SpawnedTempSession {
            session_id: child.id,
            user_message_id: user_msg.id,
        })
    }

    fn begin_child_stream(
        &self,
        child_session_id: &str,
        request_message_id: &str,
    ) -> ChildStreamHooks {
        let blocks = new_stream_blocks();
        if let Ok(mut g) = self.child_blocks.lock() {
            g.insert(child_session_id.to_string(), blocks.clone());
        }
        ChildStreamHooks {
            on_text_delta: stream_text_callback(
                self.app.clone(),
                child_session_id.to_string(),
                request_message_id.to_string(),
                blocks.clone(),
            ),
            on_tool_event: tool_event_callback(
                self.app.clone(),
                child_session_id.to_string(),
                request_message_id.to_string(),
                blocks,
            ),
        }
    }

    fn finalize_temp_session(
        &self,
        child: &SpawnedTempSession,
        result: &RunAgentResult,
        model: &str,
        provider: &str,
    ) -> AppResult<()> {
        let blocks = self.take_blocks(&child.session_id);
        let conn = self.pool.get()?;
        let params = parameters::factory().build(String::new(), String::new(), Default::default());
        let resp = chat::GenerateResponse {
            images: result.images.clone(),
            videos: result.videos.clone(),
            text: result.final_text.clone(),
            thinking_content: result.thinking_content.clone(),
            usage: result.usage.clone(),
            tool_calls: Vec::new(),
            response_id: result.response_id.clone(),
        };
        let _ = finalize_generate_assistant_message(
            &self.app,
            &conn,
            &child.session_id,
            &child.user_message_id,
            &params,
            resp,
            blocks,
            &self.role_states,
            &self.token_stats,
            &self.session_logger,
            "subagent",
            model,
            provider,
        )?;
        let _ = session::set_spawn_task_id(&conn, &child.session_id, result.task_id.as_str());
        let _ = self.app.emit(
            "gen://status",
            serde_json::json!({
                "phase": "response",
                "session_id": &child.session_id,
            }),
        );
        Ok(())
    }

    fn abandon_temp_session(
        &self,
        child: &SpawnedTempSession,
        parent_session_id: &str,
        parent_request_message_id: Option<&str>,
        tool_call_id: Option<&str>,
        reason: &str,
    ) {
        // Always drain the buffer, even if nothing below succeeds: it is keyed
        // by child session id and would otherwise be held for the lifetime of
        // the app.
        let blocks = self.take_blocks(&child.session_id);

        match self.pool.get() {
            Ok(conn) => {
                // Keep what the sub-agent managed to produce, so a cancelled
                // research task still shows its partial notes instead of an
                // empty session.
                if let Err(e) = persist_streamed_assistant_snapshot(
                    &conn,
                    &child.session_id,
                    &blocks,
                    None,
                    None,
                    serde_json::json!({ "cancelled": true, "subagent_error": reason }),
                    Some(child.user_message_id.as_str()),
                ) {
                    eprintln!(
                        "abandon temp session {}: persist failed: {e}",
                        child.session_id
                    );
                }
            }
            Err(e) => eprintln!(
                "abandon temp session {}: db connection: {e}",
                child.session_id
            ),
        }

        // Resolve the parent's Agent card. `keep_pending` is deliberately absent
        // here: the earlier `running` update set it, and only a final event
        // without it lets the UI stop the spinner.
        if let (Some(req_id), Some(call_id)) = (parent_request_message_id, tool_call_id) {
            let _ = self.app.emit(
                "gen://tool",
                serde_json::json!({
                    "session_id": parent_session_id,
                    "request_message_id": req_id,
                    "type": "tool_result",
                    "id": call_id,
                    "tool": "Agent",
                    "output": {
                        "status": "failed",
                        "child_session_id": &child.session_id,
                        "error": reason,
                    },
                    "is_error": true,
                }),
            );
        }
        let _ = self.app.emit(
            "gen://status",
            serde_json::json!({
                "phase": "response",
                "session_id": &child.session_id,
            }),
        );
    }
}
