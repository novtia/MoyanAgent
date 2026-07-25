use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::ai::agent::core::context::AbortHandle;
use crate::ai::agent::{
    AgentRegistry, FileSnapshotStore, FsSessionMemoryExtractor, FsUserContextLoader,
    NotificationQueue, ProviderEngine, QueryEngine, RoleStateStore, StaticMcpRegistry, TaskStore,
    ToolPool,
};
use crate::ai::{session_log, token_log};
use crate::data::db::{self, DbPool};
use crate::error::AppResult;

pub struct AppState {
    pub pool: DbPool,
    /// Per-session abort controllers for in-flight `generate_image` runs.
    pub(crate) generation_abort: Mutex<HashMap<String, AbortHandle>>,

    /// Agent subsystem services. Kept on `AppState` so that any Tauri
    /// command can register tasks, fan out notifications, or pick up a
    /// definition without walking the global registry every call.
    pub agent_registry: Arc<AgentRegistry>,
    pub task_store: Arc<TaskStore>,
    pub notifications: Arc<NotificationQueue>,
    #[allow(dead_code)]
    pub engine: Arc<ProviderEngine>,
    /// Full agent query loop (tool turns). Shared with [`AgentTool`].
    pub query_engine: Arc<dyn QueryEngine>,
    /// CLAUDE.md / user-context loader; cached, invalidate on compact.
    pub user_context: Arc<FsUserContextLoader>,
    /// MCP registry snapshot used by `AgentTool` to gate sub-agents.
    pub mcp: Arc<StaticMcpRegistry>,
    /// Shared tool pool. Currently holds [`FileReadTool`]; further
    /// host-implemented tools register on top.
    pub tools: Arc<ToolPool>,
    /// Per-session-memory extractor. Stateless aside from the last
    /// observed [`SessionMemory`] snapshot.
    pub session_memory: Arc<FsSessionMemoryExtractor>,
    /// Shared, project/session-scoped character state board mutated by the
    /// `RoleState` tool and snapshotted per assistant message.
    pub role_states: Arc<RoleStateStore>,
    /// AskUser human-in-the-loop wait table (tool + `answer_ask_user` command).
    pub prompt_registry: Arc<crate::ai::agent::tools::prompt_registry::PromptRegistry>,
    /// Shared, session-scoped buffer of pending agent file mutations. Drained
    /// per assistant message into `file_snapshots` for delete/regenerate
    /// rollback of created / updated / deleted files.
    pub file_snapshots: Arc<FileSnapshotStore>,
    /// Token usage statistics recorder (SQLite only, for analytics/billing).
    pub token_stats: Arc<token_log::TokenStatsRecorder>,
    /// Session content logger (per-session JSON files, for debugging).
    pub session_logger: Arc<session_log::SessionLogger>,
}

impl AppState {
    pub(crate) fn conn(&self) -> AppResult<db::DbConn> {
        Ok(self.pool.get()?)
    }
}
