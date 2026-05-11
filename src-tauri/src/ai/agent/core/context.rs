//! Per-agent execution context.
//!
//! `ToolUseContext` is the central isolation boundary referenced throughout
//! `agent-architecture.md` §10 and §20. Every tool call carries an immutable
//! reference to one; sub-agents get a *clone with overrides* so they don't
//! leak file caches, memory deltas or abort signals back to the parent.
//!
//! Cheap to share via `Arc`; mutate via inner `Mutex` only where the TS
//! side does so (file caches, nested memory paths).

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use tokio::sync::watch;

use crate::ai::agent::memory::UserContext;
use crate::ai::agent::core::permission::PermissionMode;
use crate::ai::agent::types::{AgentId, MessageRole, QuerySource};

/// Snapshot of the runtime context attached to every tool invocation.
pub struct ToolUseContext {
    pub agent_id: AgentId,
    pub query_source: QuerySource,
    pub permission_mode: PermissionMode,
    pub cwd: PathBuf,

    /// Cancellation. Sub-agents typically have *child* signals so that
    /// killing a parent agent also tears down its workers, but background
    /// tasks keep an isolated controller.
    pub abort: AbortSignal,

    /// Files the model has already Read this turn. Used by FileReadTool
    /// to short-circuit unchanged reads, and by attachment rendering to
    /// avoid duplicate `nested_memory` injections.
    pub read_file_state: Arc<Mutex<HashSet<PathBuf>>>,

    /// Paths that triggered a nested-memory attachment on this turn.
    pub nested_memory_attachment_triggers: Arc<Mutex<HashSet<PathBuf>>>,

    /// `loadedNestedMemoryPaths` — long-lived dedup set, cleared on compact.
    pub loaded_nested_memory_paths: Arc<Mutex<HashSet<PathBuf>>>,

    /// User context cache pointer. `None` ⇒ context disabled (bare/simple mode).
    pub user_context: Option<Arc<UserContext>>,

    /// The *parent agent's* fully-rendered `system_prompt`. Populated
    /// when the engine enters a query loop so that any `Agent(...)`
    /// invocation from inside that loop can fork-inherit it via
    /// [`Self::parent_system_prompt`]. `None` at the top of the main
    /// loop or when fork inheritance is irrelevant.
    pub parent_system_prompt: Option<String>,

    /// Role attribution for the *current* model turn. Used by injectors that
    /// need to know whether they are writing to a user-role meta message.
    pub current_turn_role: MessageRole,
}

impl ToolUseContext {
    pub fn builder(agent_id: AgentId, cwd: PathBuf) -> ToolUseContextBuilder {
        ToolUseContextBuilder::new(agent_id, cwd)
    }

    /// Fork a sub-agent context. Mirrors `createSubagentContext()`:
    ///
    /// - file caches are cloned (not shared);
    /// - memory dedup sets become *nested* (child changes don't affect parent);
    /// - abort signal becomes a child of the parent's signal.
    pub fn fork_subagent(self: &Arc<Self>, agent_id: AgentId) -> Arc<ToolUseContext> {
        let read_clone = self
            .read_file_state
            .lock()
            .ok()
            .map(|s| s.clone())
            .unwrap_or_default();
        let loaded_clone = self
            .loaded_nested_memory_paths
            .lock()
            .ok()
            .map(|s| s.clone())
            .unwrap_or_default();
        Arc::new(ToolUseContext {
            agent_id,
            query_source: QuerySource::Subagent,
            permission_mode: self.permission_mode,
            cwd: self.cwd.clone(),
            abort: self.abort.child(),
            read_file_state: Arc::new(Mutex::new(read_clone)),
            nested_memory_attachment_triggers: Arc::new(Mutex::new(HashSet::new())),
            loaded_nested_memory_paths: Arc::new(Mutex::new(loaded_clone)),
            user_context: self.user_context.clone(),
            parent_system_prompt: self.parent_system_prompt.clone(),
            current_turn_role: MessageRole::User,
        })
    }
}

/// Lightweight cancellation flag backed by `tokio::sync::watch`. Cheap to
/// clone, supports child-of relationship for sub-agents.
#[derive(Clone)]
pub struct AbortSignal {
    rx: watch::Receiver<bool>,
    /// Kept for the lifetime of the original signal so that `aborted()`
    /// remains stable even after spawning children.
    _tx: Arc<watch::Sender<bool>>,
}

impl AbortSignal {
    pub fn new() -> (Self, AbortHandle) {
        let (tx, rx) = watch::channel(false);
        let tx = Arc::new(tx);
        (
            Self {
                rx,
                _tx: tx.clone(),
            },
            AbortHandle { tx },
        )
    }

    pub fn aborted(&self) -> bool {
        *self.rx.borrow()
    }

    pub fn child(&self) -> AbortSignal {
        self.clone()
    }
}

pub struct AbortHandle {
    tx: Arc<watch::Sender<bool>>,
}

impl AbortHandle {
    pub fn abort(&self) {
        let _ = self.tx.send(true);
    }
}

/// Builder for [`ToolUseContext`].
pub struct ToolUseContextBuilder {
    agent_id: AgentId,
    cwd: PathBuf,
    query_source: QuerySource,
    permission_mode: PermissionMode,
    user_context: Option<Arc<UserContext>>,
    parent_system_prompt: Option<String>,
}

impl ToolUseContextBuilder {
    pub fn new(agent_id: AgentId, cwd: PathBuf) -> Self {
        Self {
            agent_id,
            cwd,
            query_source: QuerySource::ReplMainThread,
            permission_mode: PermissionMode::Default,
            user_context: None,
            parent_system_prompt: None,
        }
    }

    pub fn query_source(mut self, source: QuerySource) -> Self {
        self.query_source = source;
        self
    }

    pub fn permission_mode(mut self, mode: PermissionMode) -> Self {
        self.permission_mode = mode;
        self
    }

    pub fn user_context(mut self, ctx: Arc<UserContext>) -> Self {
        self.user_context = Some(ctx);
        self
    }

    pub fn parent_system_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.parent_system_prompt = Some(prompt.into());
        self
    }

    pub fn build(self) -> (Arc<ToolUseContext>, AbortHandle) {
        let (signal, handle) = AbortSignal::new();
        let ctx = ToolUseContext {
            agent_id: self.agent_id,
            query_source: self.query_source,
            permission_mode: self.permission_mode,
            cwd: self.cwd,
            abort: signal,
            read_file_state: Arc::new(Mutex::new(HashSet::new())),
            nested_memory_attachment_triggers: Arc::new(Mutex::new(HashSet::new())),
            loaded_nested_memory_paths: Arc::new(Mutex::new(HashSet::new())),
            user_context: self.user_context,
            parent_system_prompt: self.parent_system_prompt,
            current_turn_role: MessageRole::User,
        };
        (Arc::new(ctx), handle)
    }
}
