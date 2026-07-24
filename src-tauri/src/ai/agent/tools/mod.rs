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
pub mod create_doc;
pub mod delete;
pub mod edit;
pub mod fs;
pub mod grep;
pub mod list_files;
pub mod paragraph;
pub mod project_path;
pub mod prompt_registry;
pub mod read_receipt;
pub mod role_state;
pub mod text_decode;
pub mod todo;
pub mod web_fetch;
pub mod web_search;

use std::collections::HashMap;
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
    tools: Mutex<HashMap<String, Arc<dyn Tool>>>,
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

    /// Snapshot of all currently-registered tools. Returns owned `Arc`s
    /// so callers don't hold the inner lock during long-running
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
    ) -> HashMap<String, Arc<dyn Tool>> {
        let Ok(tools_guard) = self.tools.lock() else {
            return HashMap::new();
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

    /// Permission + validation + execute pipeline. The hooks stages from the
    /// TS executor (`PreToolUse`, `PostToolUse`) are intentionally elided
    /// here; the runner inserts them around this call.
    /// When the TodoList tool has unfinished items, return a nudge the
    /// query engine can inject so the model keeps working.
    pub fn incomplete_todo_nudge(&self) -> Option<String> {
        let guard = self.todo_list.lock().ok()?;
        guard.as_ref()?.incomplete_nudge_message()
    }

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
            PermissionDecision::Ask { reason } => {
                return Ok(ToolResult::error(format!(
                    "permission requires user prompt: {}",
                    reason.unwrap_or_default()
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

        let nudge = pool.incomplete_todo_nudge();
        assert!(nudge.is_some());

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
        assert!(pool.incomplete_todo_nudge().is_none());
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
