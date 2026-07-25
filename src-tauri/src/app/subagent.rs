use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tauri::{AppHandle, Emitter};

use crate::ai::agent::tools::agent_tool::{
    ChatRequestFactory, ChildStreamHooks, SpawnedTempSession, SubagentSessionHost,
};
use crate::ai::agent::memory::UserContextLoader;
use crate::ai::agent::{
    self, FileSnapshotStore, FsUserContextLoader, RoleStateStore, RunAgentResult,
};
use crate::ai::{chat, parameters, session_log, token_log};
use crate::data::db::DbPool;
use crate::data::{session, settings};
use crate::error::{AppError, AppResult};

use super::generation::commands::finalize_generate_assistant_message;
use super::generation::streaming::{
    new_stream_blocks, snapshot_stream_blocks, stream_text_callback, tool_event_callback,
    StreamBlocks,
};

pub(crate) struct SettingsChatFactory {
    pub(crate) pool: DbPool,
    pub(crate) user_context: Arc<FsUserContextLoader>,
}

impl SettingsChatFactory {
    pub(crate) fn new(pool: DbPool, user_context: Arc<FsUserContextLoader>) -> Self {
        Self { pool, user_context }
    }
}

impl ChatRequestFactory for SettingsChatFactory {
    fn build(
        &self,
        prompt: &str,
        _agent_type: &str,
        definition: &crate::ai::agent::AgentDefinition,
    ) -> AppResult<(chat::ChatRequest, Vec<agent::Attachment>)> {
        let conn = self.pool.get()?;
        let settings = settings::read(&conn)?;

        // Spawned sub-agents follow the global default model/provider.
        let provider = settings::active_provider(&settings)
            .cloned()
            .ok_or_else(|| AppError::Config("no enabled model provider configured".into()))?;

        // Runner overwrites `system_prompt` with `definition.system_prompt`
        // plus env-details + critical reminder, so leave it empty here.
        let chat = crate::ai::router::build_chat_request(
            &provider,
            &settings.model,
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

        // Honour `omit_claude_md`: only inject user-context (CLAUDE.md +
        // rules) when the agent definition opts in. Rendered as a
        // `Delta { topic = "user_context" }` attachment so the engine
        // turns it into a `<system-reminder>` block on entry.
        let attachments = if definition.omit_claude_md {
            Vec::new()
        } else {
            self.user_context
                .load()
                .ok()
                .map(|uc| {
                    let rendered = uc.rendered.trim();
                    if rendered.is_empty() {
                        Vec::new()
                    } else {
                        vec![agent::Attachment::for_main(agent::AttachmentKind::Delta {
                            topic: "user_context".into(),
                            body: rendered.to_string(),
                        })]
                    }
                })
                .unwrap_or_default()
        };

        Ok((chat, attachments))
    }
}

/// Host bridge that materialises temporary child sessions for `Agent` tool
/// dispatches and streams the sub-agent's events into that child.
pub(crate) struct TauriSubagentHost {
    pub(crate) app: AppHandle,
    pub(crate) pool: DbPool,
    pub(crate) role_states: Arc<RoleStateStore>,
    pub(crate) file_snapshots: Arc<FileSnapshotStore>,
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
        file_snapshots: Arc<FileSnapshotStore>,
        token_stats: Arc<token_log::TokenStatsRecorder>,
        session_logger: Arc<session_log::SessionLogger>,
    ) -> Self {
        Self {
            app,
            pool,
            role_states,
            file_snapshots,
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
        let params = parameters::factory().build(
            String::new(),
            String::new(),
            Default::default(),
        );
        let resp = chat::GenerateResponse {
            images: result.images.clone(),
            videos: result.videos.clone(),
            text: result.final_text.clone(),
            thinking_content: result.thinking_content.clone(),
            usage: result.usage.clone(),
            tool_calls: Vec::new(),
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
            &self.file_snapshots,
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
}
