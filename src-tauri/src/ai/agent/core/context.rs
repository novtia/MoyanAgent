//! Per-agent execution context.
//!
//! `ToolUseContext` is the central isolation boundary referenced throughout
//! `agent-architecture.md` §10 and §20. Every tool call carries an immutable
//! reference to one; sub-agents get a *clone with overrides* so they don't
//! leak file caches, memory deltas or abort signals back to the parent.
//!
//! Cheap to share via `Arc`; mutate via inner `Mutex` only where the TS
//! side does so (file caches, nested memory paths).

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use tokio::sync::watch;

use crate::ai::agent::memory::UserContext;
use crate::ai::agent::core::permission::PermissionMode;
use crate::ai::agent::types::{AgentId, MessageRole, QuerySource};
use crate::ai::session_log::SessionLogger;
use crate::ai::token_log::TokenStatsRecorder;

/// Snapshot of the runtime context attached to every tool invocation.
pub struct ToolUseContext {
    pub agent_id: AgentId,
    pub query_source: QuerySource,
    pub permission_mode: PermissionMode,
    pub cwd: PathBuf,

    /// Database session id this run belongs to, when known. Tools that
    /// persist per-conversation state (e.g. `RoleState`) key off this.
    /// `None` for runs not tied to a chat session.
    pub session_id: Option<String>,

    /// Role-state scope id: `project_id` for project sessions, otherwise
    /// the session id. Set alongside `session_id` for main/sub-agent runs.
    pub role_state_scope_id: Option<String>,

    /// Correlates all token events within one user send / assistant reply turn.
    pub correlation_id: Option<String>,

    /// Agent definition type driving this run (e.g. `general-purpose`).
    pub agent_type: Option<String>,

    /// Token usage statistics recorder (SQLite); `None` disables telemetry.
    pub token_stats: Option<Arc<TokenStatsRecorder>>,

    /// Session content logger (per-session JSON files for debugging);
    /// `None` disables content logging for this run.
    pub session_logger: Option<Arc<SessionLogger>>,

    /// Cancellation. Sub-agents typically have *child* signals so that
    /// killing a parent agent also tears down its workers, but background
    /// tasks keep an isolated controller.
    pub abort: AbortSignal,

    /// Files the model has already Read this session, mapped to the content
    /// hash observed at that read/write. Used by FileReadTool to short-circuit
    /// unchanged reads.
    pub read_file_state: Arc<Mutex<HashMap<PathBuf, u64>>>,

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
            session_id: self.session_id.clone(),
            role_state_scope_id: self.role_state_scope_id.clone(),
            correlation_id: self.correlation_id.clone(),
            agent_type: self.agent_type.clone(),
            token_stats: self.token_stats.clone(),
            session_logger: self.session_logger.clone(),
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
///
/// A [`child`](Self::child) owns its own channel and additionally observes
/// every ancestor's: cancelling a parent tears the child down, while cancelling
/// the child leaves the parent (and its siblings) running. Ancestors are held
/// as a flat list so checking the flag never recurses.
#[derive(Clone)]
pub struct AbortSignal {
    rx: watch::Receiver<bool>,
    /// Kept for the lifetime of the original signal so that `aborted()`
    /// remains stable even after spawning children.
    _tx: Arc<watch::Sender<bool>>,
    /// Receivers of this signal's ancestors, outermost first.
    ancestors: Arc<Vec<watch::Receiver<bool>>>,
}

impl AbortSignal {
    pub fn new() -> (Self, AbortHandle) {
        let (tx, rx) = watch::channel(false);
        let tx = Arc::new(tx);
        (
            Self {
                rx,
                _tx: tx.clone(),
                ancestors: Arc::new(Vec::new()),
            },
            AbortHandle { tx },
        )
    }

    pub fn aborted(&self) -> bool {
        *self.rx.borrow() || self.ancestors.iter().any(|rx| *rx.borrow())
    }

    /// Block until this signal or any ancestor is aborted.
    pub async fn wait_aborted(&self) {
        if self.aborted() {
            return;
        }
        let waits: Vec<_> = std::iter::once(self.rx.clone())
            .chain(self.ancestors.iter().cloned())
            .map(|mut rx| {
                Box::pin(async move {
                    // A closed channel means the owner is gone; treat it as
                    // "will never abort" and let the other arms decide.
                    if rx.changed().await.is_err() {
                        std::future::pending::<()>().await;
                    }
                })
            })
            .collect();
        futures_util::future::select_all(waits).await;
    }

    /// Derive a signal that this one can cancel but which cannot cancel this
    /// one. Used for sub-agents so a worker failing does not kill its parent.
    pub fn child(&self) -> AbortSignal {
        let (tx, rx) = watch::channel(false);
        let mut ancestors = Vec::with_capacity(self.ancestors.len() + 1);
        ancestors.extend(self.ancestors.iter().cloned());
        ancestors.push(self.rx.clone());
        AbortSignal {
            rx,
            _tx: Arc::new(tx),
            ancestors: Arc::new(ancestors),
        }
    }

    /// Handle that aborts this signal and everything derived from it via
    /// [`child`](Self::child) — but not its parents.
    pub fn controller(&self) -> AbortHandle {
        AbortHandle {
            tx: self._tx.clone(),
        }
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

#[cfg(test)]
mod abort_tests {
    use super::*;

    #[test]
    fn a_parent_abort_reaches_every_descendant() {
        let (parent, handle) = AbortSignal::new();
        let child = parent.child();
        let grandchild = child.child();

        handle.abort();

        assert!(parent.aborted());
        assert!(child.aborted(), "a child follows its parent");
        assert!(grandchild.aborted(), "and so does a grandchild");
    }

    /// The whole point of a child signal: one sub-agent giving up must not
    /// cancel the run that spawned it, nor its siblings.
    #[test]
    fn aborting_a_child_leaves_the_parent_and_siblings_running() {
        let (parent, _handle) = AbortSignal::new();
        let child = parent.child();
        let sibling = parent.child();

        child.controller().abort();

        assert!(child.aborted());
        assert!(!parent.aborted(), "parent keeps running");
        assert!(!sibling.aborted(), "sibling keeps running");
    }

    #[tokio::test]
    async fn waiting_wakes_on_a_parent_abort() {
        let (parent, handle) = AbortSignal::new();
        let child = parent.child();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            handle.abort();
        });

        tokio::time::timeout(std::time::Duration::from_secs(5), child.wait_aborted())
            .await
            .expect("the child must wake when its parent is cancelled");
        assert!(child.aborted());
    }

    #[tokio::test]
    async fn waiting_wakes_on_its_own_abort() {
        let (parent, _handle) = AbortSignal::new();
        let child = parent.child();
        let own = child.controller();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            own.abort();
        });

        tokio::time::timeout(std::time::Duration::from_secs(5), child.wait_aborted())
            .await
            .expect("its own controller must still wake it");
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
    session_id: Option<String>,
    role_state_scope_id: Option<String>,
    correlation_id: Option<String>,
    agent_type: Option<String>,
    token_stats: Option<Arc<TokenStatsRecorder>>,
    session_logger: Option<Arc<SessionLogger>>,
    /// When set, the built context shares this cancellation controller
    /// (used by the main session so `cancel_generation` can stop in-flight work).
    abort_signal: Option<AbortSignal>,
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
            session_id: None,
            role_state_scope_id: None,
            correlation_id: None,
            agent_type: None,
            token_stats: None,
            session_logger: None,
            abort_signal: None,
        }
    }

    pub fn abort_signal(mut self, signal: AbortSignal) -> Self {
        self.abort_signal = Some(signal);
        self
    }

    pub fn session_id(mut self, session_id: impl Into<String>) -> Self {
        self.session_id = Some(session_id.into());
        self
    }

    pub fn role_state_scope_id(mut self, scope_id: impl Into<String>) -> Self {
        self.role_state_scope_id = Some(scope_id.into());
        self
    }

    pub fn correlation_id(mut self, correlation_id: impl Into<String>) -> Self {
        self.correlation_id = Some(correlation_id.into());
        self
    }

    pub fn agent_type(mut self, agent_type: impl Into<String>) -> Self {
        self.agent_type = Some(agent_type.into());
        self
    }

    pub fn token_stats(mut self, recorder: Arc<TokenStatsRecorder>) -> Self {
        self.token_stats = Some(recorder);
        self
    }

    pub fn session_logger(mut self, logger: Arc<SessionLogger>) -> Self {
        self.session_logger = Some(logger);
        self
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
        let (signal, handle) = match self.abort_signal {
            Some(signal) => {
                let handle = signal.controller();
                (signal, handle)
            }
            None => AbortSignal::new(),
        };
        let ctx = ToolUseContext {
            agent_id: self.agent_id,
            query_source: self.query_source,
            permission_mode: self.permission_mode,
            cwd: self.cwd,
            session_id: self.session_id,
            role_state_scope_id: self.role_state_scope_id,
            correlation_id: self.correlation_id,
            agent_type: self.agent_type,
            token_stats: self.token_stats,
            session_logger: self.session_logger,
            abort: signal,
            read_file_state: Arc::new(Mutex::new(HashMap::new())),
            nested_memory_attachment_triggers: Arc::new(Mutex::new(HashSet::new())),
            loaded_nested_memory_paths: Arc::new(Mutex::new(HashSet::new())),
            user_context: self.user_context,
            parent_system_prompt: self.parent_system_prompt,
            current_turn_role: MessageRole::User,
        };
        (Arc::new(ctx), handle)
    }
}
