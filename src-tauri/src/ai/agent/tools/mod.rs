//! Tool abstractions.
//!
//! Maps the TS side:
//!
//! - `Tool` interface → [`Tool`] trait
//! - `tools.ts` registry + `assembleToolPool()` → [`ToolPool`]
//! - `ToolUseContext` access → injected via [`ToolInvocation`]
//! - `tool_result` shape → [`ToolResult`]
//!
//! Tools are deliberately object-safe; the executor only sees `dyn Tool`.
//! Concurrency safety mirrors the docs: read-only / concurrency-safe tools
//! can run in parallel inside [`crate::ai::agent::exec::query::QueryEngine`].
//!
//! Submodules:
//! - [`fs`]          filesystem read (FileRead)
//! - [`edit`]        filesystem mutation (Write, Edit)
//! - [`bash`]        shell execution (Bash)
//! - [`agent_tool`]  the `Agent` meta-tool that spawns sub-agents

pub mod agent_tool;
pub mod ask_user;
pub mod bash;
pub mod consult_roles;
pub mod create_doc;
pub mod delete;
pub mod edit;
pub mod fs;
pub mod grep;
pub mod list_files;
pub mod paragraph;
pub mod pe_docs;
pub mod project_path;
pub mod prompt_registry;
pub mod read_receipt;
pub mod role_state;
pub mod text_decode;
pub mod todo;
pub mod web_fetch;
pub mod web_search;

use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

use crate::ai::agent::core::context::ToolUseContext;
use crate::ai::agent::core::permission::{PermissionDecision, PermissionRequest};
use crate::ai::agent::types::MessageId;
use crate::error::{AppError, AppResult};

/// Static description of a tool. The model-facing schema lives in `schema`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    /// JSON schema of the `input` argument as exposed to the model.
    pub schema: serde_json::Value,
    /// `true` if the tool performs no side-effects and is safe to execute
    /// concurrently with other read-only tools.
    pub read_only: bool,
    /// `true` if it is safe to execute this tool concurrently with siblings
    /// even when it performs writes — typically false.
    pub concurrency_safe: bool,
}

/// Single invocation passed to [`Tool::execute`].
pub struct ToolInvocation<'a> {
    pub id: MessageId,
    pub input: serde_json::Value,
    pub context: &'a ToolUseContext,
}

/// Result returned by a tool. Mirrors `{ content, is_error, metadata }`
/// shapes used in the TS executor.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    /// Free-form text / JSON content shown to the model as `tool_result`.
    pub content: serde_json::Value,
    /// If true, the executor will surface this as a denied / errored result.
    pub is_error: bool,
    /// Side-channel metadata used by the UI and telemetry, not sent to the model.
    pub metadata: Option<serde_json::Value>,
}

impl ToolResult {
    pub fn ok(content: serde_json::Value) -> Self {
        Self {
            content,
            is_error: false,
            metadata: None,
        }
    }

    pub fn error(message: impl Into<String>) -> Self {
        Self {
            content: serde_json::json!({ "error": message.into() }),
            is_error: true,
            metadata: None,
        }
    }
}

/// Async return type used by tools without `async-trait`.
pub type ToolFuture<'a> = Pin<Box<dyn Future<Output = AppResult<ToolResult>> + Send + 'a>>;

/// Core tool trait. Implementations must be `Send + Sync` to fit the executor.
pub trait Tool: Send + Sync {
    fn spec(&self) -> &ToolSpec;

    /// Validate the raw input *before* permission resolution. Cheap, sync.
    fn validate(&self, _input: &serde_json::Value) -> AppResult<()> {
        Ok(())
    }

    fn execute<'a>(&'a self, invocation: ToolInvocation<'a>) -> ToolFuture<'a>;
}

/// A registry of installed tools, plus deny-lists and per-agent filters.
///
/// Mirrors the layered filtering described in `agent-architecture.md` §9:
///
/// 1. [`ToolPool::all`] — built-ins + MCP-injected tools.
/// 2. [`ToolPool::deny_global`] — `ALL_AGENT_DISALLOWED_TOOLS`.
/// 3. [`ToolPool::filter_for_agent`] — applies agent's `tools` / `disallowedTools`.
#[derive(Default)]
pub struct ToolPool {
    /// Interior-mutable so that consumers can register additional tools
    /// (e.g. `AgentTool` registered after the pool is wrapped in `Arc`)
    /// without `&mut self`.
    ///
    /// Ordered by name rather than hashed: the serialized `tools` array is part
    /// of the prompt prefix that providers hash for their context cache, and a
    /// per-request reshuffle would void every hit past the system prompt.
    tools: Mutex<BTreeMap<String, Arc<dyn Tool>>>,
    /// Tools that are denied for *every* sub-agent (e.g. `AgentTool`,
    /// `TaskOutput`, plan-mode primitives).
    global_deny: Mutex<Vec<String>>,
    /// Direct handle for todo-state inspection by the query engine.
    todo_list: Mutex<Option<Arc<todo::TodoListTool>>>,
}

impl ToolPool {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register<T: Tool + 'static>(&self, tool: T) {
        let name = tool.spec().name.clone();
        if let Ok(mut g) = self.tools.lock() {
            g.insert(name, Arc::new(tool));
        }
    }

    /// Register a [`todo::TodoListTool`] and wire it for engine-level
    /// incomplete-task detection.
    pub fn register_todo_list(&self, tool: todo::TodoListTool) {
        let arc = Arc::new(tool);
        if let Ok(mut g) = self.tools.lock() {
            g.insert(todo::TOOL_NAME.to_string(), arc.clone());
        }
        if let Ok(mut t) = self.todo_list.lock() {
            *t = Some(arc);
        }
    }

    pub fn register_arc(&self, tool: Arc<dyn Tool>) {
        let name = tool.spec().name.clone();
        if let Ok(mut g) = self.tools.lock() {
            g.insert(name, tool);
        }
    }

    pub fn deny_global(&self, name: impl Into<String>) {
        if let Ok(mut g) = self.global_deny.lock() {
            g.push(name.into());
        }
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        self.tools.lock().ok()?.get(name).cloned()
    }

    /// Snapshot of all currently-registered tools, ordered by name. Returns
    /// owned `Arc`s so callers don't hold the inner lock during long-running
    /// inspections.
    pub fn all(&self) -> Vec<Arc<dyn Tool>> {
        self.tools
            .lock()
            .ok()
            .map(|g| g.values().cloned().collect())
            .unwrap_or_default()
    }

    /// Produce the subset of tools available to a particular agent
    /// definition. Honors:
    ///
    /// - global deny list,
    /// - agent's `disallowedTools`,
    /// - agent's `tools` whitelist (`["*"]` = all non-denied).
    pub fn filter_for_agent(
        &self,
        allow: &[String],
        deny: &[String],
    ) -> BTreeMap<String, Arc<dyn Tool>> {
        let Ok(tools_guard) = self.tools.lock() else {
            return BTreeMap::new();
        };
        let global_deny = self.global_deny.lock().map(|g| g.clone()).unwrap_or_default();
        let wildcard = allow.iter().any(|t| t == "*");
        let allow_set: std::collections::HashSet<&String> = allow.iter().collect();
        let deny_set: std::collections::HashSet<&String> =
            global_deny.iter().chain(deny.iter()).collect();
        tools_guard
            .iter()
            .filter(|(name, _)| !deny_set.contains(*name))
            .filter(|(name, _)| wildcard || allow_set.contains(*name))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }

    /// Copy the typed TodoList handle from another pool. Worker pools are
    /// assembled via [`Self::register_arc`] which only sees `dyn Tool`, so
    /// the engine's nudge / clear path needs this extra wiring.
    pub fn share_todo_list_from(&self, other: &ToolPool) {
        let handle = other.todo_list.lock().ok().and_then(|g| g.clone());
        if let Some(arc) = handle {
            if let Ok(mut t) = self.todo_list.lock() {
                *t = Some(arc);
            }
        }
    }

    /// When the TodoList tool has unfinished items in this run's scope,
    /// return a nudge the query engine can inject so the model keeps working.
    pub fn incomplete_todo_nudge(&self, ctx: &ToolUseContext) -> Option<String> {
        let guard = self.todo_list.lock().ok()?;
        guard.as_ref()?.incomplete_nudge_message(ctx)
    }

    /// Compact live ✔/☐ checklist for the current run, or `None` if no list.
    pub fn todo_prompt_snapshot(&self, ctx: &ToolUseContext, premature_stop: bool) -> Option<String> {
        let guard = self.todo_list.lock().ok()?;
        guard.as_ref()?.prompt_snapshot(ctx, premature_stop)
    }

    /// Drop this run's TodoList so the next generation starts empty.
    pub fn clear_todo_scope(&self, ctx: &ToolUseContext) {
        if let Ok(guard) = self.todo_list.lock() {
            if let Some(tool) = guard.as_ref() {
                tool.clear_scope(ctx);
            }
        }
    }

    /// Permission + validation + execute pipeline. The hooks stages from the
    /// TS executor (`PreToolUse`, `PostToolUse`) are intentionally elided
    /// here; the runner inserts them around this call.
    pub async fn execute(
        &self,
        name: &str,
        invocation: ToolInvocation<'_>,
        request: PermissionRequest<'_>,
        resolver: &dyn crate::ai::agent::core::permission::PermissionResolver,
    ) -> AppResult<ToolResult> {
        let tool = self
            .get(name)
            .ok_or_else(|| AppError::Invalid(format!("unknown tool: {name}")))?;

        tool.validate(&invocation.input)?;

        match resolver.resolve(request)? {
            PermissionDecision::Allow { .. } => {}
            PermissionDecision::Deny { reason } => {
                return Ok(ToolResult::error(format!("denied: {reason}")));
            }
            // No interactive approval channel is wired into the executor, so
            // `Ask` cannot be satisfied here. Report it as a *refusal pending
            // approval* rather than a generic failure: a model that reads
            // "error" retries the same call, whereas this tells it the call
            // was understood but withheld, and that only the user can unblock
            // it. Resolvers that must not stall a run should return an
            // explicit `Allow` / `Deny` instead.
            PermissionDecision::Ask { reason } => {
                let detail = reason
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .unwrap_or("this tool requires explicit user approval");
                return Ok(ToolResult::error(format!(
                    "awaiting user approval: {detail}. Do not retry — ask the user to \
                     approve this action (or raise the permission mode) before continuing."
                )));
            }
        }

        tool.execute(invocation).await
    }
}

#[cfg(test)]
mod pool_tests {
    use super::*;
    use serde_json::json;
    use crate::ai::agent::core::context::ToolUseContextBuilder;
    use crate::ai::agent::core::permission::{AllowAllResolver, PermissionRequest};
    use crate::ai::agent::types::{AgentId, MessageId};
    use std::path::PathBuf;

    fn test_context() -> Arc<ToolUseContext> {
        ToolUseContextBuilder::new(AgentId::new(), PathBuf::from("."))
            .build()
            .0
    }

    async fn run_todo(
        pool: &ToolPool,
        ctx: &Arc<ToolUseContext>,
        input: serde_json::Value,
    ) -> ToolResult {
        pool.execute(
            todo::TOOL_NAME,
            ToolInvocation {
                id: MessageId("todo".into()),
                input: input.clone(),
                context: ctx.as_ref(),
            },
            PermissionRequest {
                agent_id: &ctx.agent_id,
                tool_name: todo::TOOL_NAME,
                input: &input,
                mode: ctx.permission_mode,
                is_async: false,
                is_coordinator_worker: false,
            },
            &AllowAllResolver,
        )
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn todo_create_then_update_via_pool() {
        let pool = ToolPool::new();
        pool.register_todo_list(todo::TodoListTool::new());
        let ctx = test_context();

        let created = run_todo(
            &pool,
            &ctx,
            json!({
                "action": "create",
                "tasks": [
                    { "title": "task one", "detail": "d1" },
                    { "title": "task two" }
                ]
            }),
        )
        .await;
        assert!(!created.is_error);
        assert_eq!(created.content["items"][0]["id"], 1);
        assert_eq!(created.content["items"][1]["id"], 2);

        let snap = pool
            .todo_prompt_snapshot(ctx.as_ref(), false)
            .expect("live snapshot");
        assert!(snap.contains("☐"));
        assert!(snap.contains("task one"));
        assert!(!snap.contains("You tried to stop"));

        let nudge = pool.incomplete_todo_nudge(ctx.as_ref());
        assert!(nudge.is_some());
        assert!(nudge.unwrap().contains("You tried to stop"));

        let updated = run_todo(
            &pool,
            &ctx,
            json!({
                "action": "update",
                "tasks": [
                    { "id": 1, "status": "done" },
                    { "id": 2, "status": "done" }
                ]
            }),
        )
        .await;
        assert!(!updated.is_error);
        assert_eq!(updated.content["items"][0]["status"], "done");
        assert!(pool.incomplete_todo_nudge(ctx.as_ref()).is_none());
    }

    #[tokio::test]
    async fn todo_second_create_is_rejected() {
        let pool = ToolPool::new();
        pool.register_todo_list(todo::TodoListTool::new());
        let ctx = test_context();

        run_todo(
            &pool,
            &ctx,
            json!({ "action": "create", "tasks": [{ "title": "a" }] }),
        )
        .await;
        let second = run_todo(
            &pool,
            &ctx,
            json!({ "action": "create", "tasks": [{ "title": "b" }] }),
        )
        .await;
        assert!(second.is_error);
    }

    #[tokio::test]
    async fn todo_lists_are_isolated_per_session_and_agent() {
        let pool = ToolPool::new();
        pool.register_todo_list(todo::TodoListTool::new());
        let ctx_a = ToolUseContextBuilder::new(AgentId::new(), PathBuf::from("."))
            .session_id("sess-a")
            .build()
            .0;
        let ctx_b = ToolUseContextBuilder::new(AgentId::new(), PathBuf::from("."))
            .session_id("sess-b")
            .build()
            .0;

        run_todo(
            &pool,
            &ctx_a,
            json!({
                "action": "create",
                "tasks": [{ "title": "voxel planet" }]
            }),
        )
        .await;
        let created_b = run_todo(
            &pool,
            &ctx_b,
            json!({
                "action": "create",
                "tasks": [{ "title": "write chapter one" }]
            }),
        )
        .await;
        assert!(!created_b.is_error);
        assert_eq!(created_b.content["items"][0]["title"], "write chapter one");
        assert!(pool
            .incomplete_todo_nudge(ctx_a.as_ref())
            .unwrap()
            .contains("voxel planet"));
        assert!(pool
            .incomplete_todo_nudge(ctx_b.as_ref())
            .unwrap()
            .contains("write chapter one"));

        pool.clear_todo_scope(ctx_a.as_ref());
        assert!(pool.incomplete_todo_nudge(ctx_a.as_ref()).is_none());
        let recreate_a = run_todo(
            &pool,
            &ctx_a,
            json!({
                "action": "create",
                "tasks": [{ "title": "fresh list" }]
            }),
        )
        .await;
        assert!(!recreate_a.is_error);
        assert_eq!(recreate_a.content["items"][0]["title"], "fresh list");
    }

    struct DummyTool(ToolSpec);

    impl Tool for DummyTool {
        fn spec(&self) -> &ToolSpec {
            &self.0
        }
        fn execute<'a>(&'a self, _invocation: ToolInvocation<'a>) -> ToolFuture<'a> {
            Box::pin(async { Ok(ToolResult::ok(json!({}))) })
        }
    }

    fn dummy(name: &str) -> DummyTool {
        DummyTool(ToolSpec {
            name: name.into(),
            description: String::new(),
            schema: json!({}),
            read_only: true,
            concurrency_safe: true,
        })
    }

    /// Mirror how `run_cancellable_generation` assembles the per-request pool:
    /// filter the global registry, then register the survivors into a brand-new
    /// `ToolPool`.
    fn worker_tool_names(global: &ToolPool) -> Vec<String> {
        let worker = ToolPool::new();
        for (_, tool) in global.filter_for_agent(&["*".to_string()], &[]) {
            worker.register_arc(tool);
        }
        worker
            .all()
            .into_iter()
            .map(|t| t.spec().name.clone())
            .collect()
    }

    /// The tools array is part of the prompt prefix every provider hashes for
    /// its context cache. A per-request reshuffle keeps the token count
    /// identical while destroying every cache hit past the system prompt.
    #[test]
    fn tool_definition_order_is_stable_across_pools() {
        let names = [
            "Read", "Edit", "Write", "Bash", "Grep", "ListFiles", "Delete", "WebFetch",
            "WebSearch", "TodoList", "AskUser", "CreateDoc", "RoleState", "ConsultRoles",
        ];

        let baseline = {
            let global = ToolPool::new();
            for n in names {
                global.register(dummy(n));
            }
            worker_tool_names(&global)
        };

        for attempt in 0..32 {
            let global = ToolPool::new();
            for n in names {
                global.register(dummy(n));
            }
            assert_eq!(
                baseline,
                worker_tool_names(&global),
                "tool order changed on attempt {attempt}"
            );
        }
    }

    #[tokio::test]
    async fn todo_update_unknown_id_returns_error() {
        let pool = ToolPool::new();
        pool.register_todo_list(todo::TodoListTool::new());
        let ctx = test_context();

        run_todo(
            &pool,
            &ctx,
            json!({ "action": "create", "tasks": [{ "title": "a" }] }),
        )
        .await;
        let bad = run_todo(
            &pool,
            &ctx,
            json!({ "action": "update", "tasks": [{ "id": 999, "status": "done" }] }),
        )
        .await;
        assert!(bad.is_error);
    }
}
