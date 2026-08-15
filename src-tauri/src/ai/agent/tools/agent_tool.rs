//! The `Agent` tool — entry point that lets the parent loop spawn a
//! sub-agent. Maps `tools/AgentTool/AgentTool.tsx`.
//!
//! Responsibilities (compressed from §7 of the architecture doc):
//!
//! 1. Pick an agent definition (`subagent_type` → explicit, missing →
//!    `FORK_AGENT` when the fork gate is on, else `general-purpose`).
//! 2. Check `requiredMcpServers`.
//! 3. Resolve isolation: none / worktree / remote.
//! 4. Build prompt messages (normal vs fork).
//! 5. Build a worker `ToolPool` (filter by allow/deny).
//! 6. Register a task (foreground / background).
//! 7. Delegate to [`crate::ai::agent::exec::runner::run_agent`].
//! 8. Collect final text + usage + duration, clean up.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::ai::agent::config::builtin::{AGENT_FORK, AGENT_GENERAL_PURPOSE};
use crate::ai::agent::config::definition::AgentDefinition;
use crate::ai::agent::config::mcp::McpRegistry;
use crate::ai::agent::config::registry::AgentRegistry;
use crate::ai::agent::core::attachment::Attachment;
use crate::ai::agent::exec::query::QueryEngine;
use crate::ai::agent::exec::runner::{RunAgentParams, RunAgentResult, run_agent};
use crate::ai::agent::core::task::TaskStore;
use crate::ai::agent::tools::{Tool, ToolFuture, ToolInvocation, ToolPool, ToolResult, ToolSpec};
use crate::ai::agent::types::AgentRunMode;
use crate::ai::chat::{ChatRequest, TextDeltaCallback};
use crate::ai::agent::exec::query::ToolEventCallback;
use crate::error::{AppError, AppResult};

pub const AGENT_TOOL_NAME: &str = "Agent";

/// Strategy for building a child [`ChatRequest`] when [`AgentTool`] is
/// invoked through the [`Tool`] trait (i.e. the model called
/// `Agent(...)`).
///
/// Hosts implement this once over their settings/db/user-context layer;
/// AgentTool remains decoupled from any specific configuration source.
/// Returning the second tuple member is how the host injects
/// runtime-only context that doesn't fit on the `AgentDefinition`
/// (CLAUDE.md, drained task notifications, plan-mode banners, ...).
pub trait ChatRequestFactory: Send + Sync {
    /// Build a chat request for a spawned agent.
    ///
    /// When `session_id` is set, the host should prefer that session's
    /// model/provider over the global default (ConsultRoles / Agent tool
    /// inherit the parent conversation's model).
    fn build(
        &self,
        prompt: &str,
        agent_type: &str,
        definition: &AgentDefinition,
        session_id: Option<&str>,
    ) -> AppResult<(ChatRequest, Vec<Attachment>)>;
}

/// Temp child session prepared for a sub-agent run.
#[derive(Debug, Clone)]
pub struct SpawnedTempSession {
    pub session_id: String,
    pub user_message_id: String,
}

/// Streaming callbacks + opaque drain token for a child session.
pub struct ChildStreamHooks {
    pub on_text_delta: TextDeltaCallback,
    pub on_tool_event: ToolEventCallback,
}

/// Host-side persistence + streaming for temporary subagent sessions.
///
/// Implemented by the Tauri app layer so [`AgentTool`] stays free of SQLite /
/// `AppHandle` coupling.
pub trait SubagentSessionHost: Send + Sync {
    /// Create a hidden temp session, seed the dispatch prompt as a user
    /// message, and emit a mid-run `{ status: "running", child_session_id }`
    /// update onto the parent Agent tool_use block.
    fn prepare_temp_session(
        &self,
        parent_session_id: &str,
        parent_request_message_id: Option<&str>,
        tool_call_id: &str,
        title: &str,
        prompt: &str,
    ) -> AppResult<SpawnedTempSession>;

    /// Wire `gen://stream` / `gen://tool` for the child session.
    fn begin_child_stream(
        &self,
        child_session_id: &str,
        request_message_id: &str,
    ) -> ChildStreamHooks;

    /// Persist the child assistant message from the completed run.
    fn finalize_temp_session(
        &self,
        child: &SpawnedTempSession,
        result: &RunAgentResult,
        model: &str,
        provider: &str,
    ) -> AppResult<()>;

    /// Release a child session whose run failed or was cancelled.
    ///
    /// [`Self::prepare_temp_session`] has already told the UI that this child is
    /// running, and [`Self::begin_child_stream`] has already handed out a buffer
    /// the host is holding. Without this call the parent's tool card spins
    /// forever, the child session stays an orphan with a prompt and no reply,
    /// and the buffer is never reclaimed.
    fn abandon_temp_session(
        &self,
        child: &SpawnedTempSession,
        parent_session_id: &str,
        parent_request_message_id: Option<&str>,
        tool_call_id: Option<&str>,
        reason: &str,
    );
}

/// Arguments the model passes when calling the `Agent` tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentInvocation {
    /// Short description used by the routing classifier / UI.
    pub description: String,
    /// Free-form task body.
    pub prompt: String,
    /// `None` ⇒ use fork or general-purpose default.
    pub subagent_type: Option<String>,
    /// Force-background flag from the model.
    #[serde(default)]
    pub run_in_background: bool,
    /// Optional team coordination.
    #[serde(default)]
    pub team_name: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
}

/// What the tool returns to the parent model.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum AgentToolResult {
    /// Foreground run finished. Parent sees the synthesised text.
    Completed {
        agent_id: String,
        task_id: String,
        text: Option<String>,
        tool_calls: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        child_session_id: Option<String>,
    },
    /// Background run launched; the parent should expect a
    /// `<task-notification>` later.
    ///
    /// Nothing produces this today: `Background` / `Fork` dispatches are awaited
    /// inline and report [`Self::Completed`] (see `AgentTool::shape_result`).
    /// The variant stays because the renderer and the model prompt both already
    /// understand it, so a genuinely detached run can adopt it — but only once
    /// something actually queues the completion notification.
    AsyncLaunched {
        agent_id: String,
        task_id: String,
        output_file: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        child_session_id: Option<String>,
    },
    /// Mid-run draft written into the pending tool block so the UI can
    /// open the child session before completion.
    Running {
        child_session_id: String,
    },
}

/// The Agent tool itself. Wraps the policy from `AgentTool.call()`.
///
/// Holds `Arc` references to the shared services so it can be cloned freely
/// into the tool pool.
#[derive(Clone)]
pub struct AgentTool {
    pub registry: Arc<AgentRegistry>,
    pub tools: Arc<ToolPool>,
    pub task_store: Arc<TaskStore>,
    pub engine: Arc<dyn QueryEngine>,
    pub mcp: Arc<dyn McpRegistry>,
    /// Optional. When `Some`, the `Tool` impl can be invoked by the
    /// model directly; the factory turns the prompt + agent_type into a
    /// fully-formed [`ChatRequest`]. Without a factory the `Tool` impl
    /// returns an error explaining that the host needs to wire one.
    pub chat_factory: Option<Arc<dyn ChatRequestFactory>>,
    /// Optional host that creates temp child sessions + streams into them.
    pub session_host: Option<Arc<dyn SubagentSessionHost>>,
    /// Cached tool spec exposed to the model.
    spec: ToolSpec,
    /// Fork gate: mirrors the `tengu_*` feature flag check. When false,
    /// omitting `subagent_type` falls back to `general-purpose` instead of
    /// the fork agent.
    pub fork_enabled: bool,
    /// Whether this AgentTool is itself running inside a forked worker.
    /// Prevents recursive forks.
    pub is_forked_worker: bool,
    /// How many sub-agent hops deep this instance already is. 0 is the main
    /// loop's own tool; a sub-agent's pool gets one more.
    pub depth: usize,
}

/// Sub-agent nesting allowed before `Agent` stops being handed down.
///
/// Each level multiplies model calls and tool executions by its fan-out, so an
/// unbounded chain is both a runaway-cost and a runaway-concurrency hazard —
/// and a model that keeps delegating "one more layer" never does the work.
pub const MAX_SUBAGENT_DEPTH: usize = 3;

impl AgentTool {
    pub fn new(
        registry: Arc<AgentRegistry>,
        tools: Arc<ToolPool>,
        task_store: Arc<TaskStore>,
        engine: Arc<dyn QueryEngine>,
        mcp: Arc<dyn McpRegistry>,
    ) -> Self {
        Self {
            registry,
            tools,
            task_store,
            engine,
            mcp,
            chat_factory: None,
            session_host: None,
            spec: agent_tool_spec(),
            fork_enabled: false,
            is_forked_worker: false,
            depth: 0,
        }
    }

    /// The tool pool a sub-agent of this dispatch should see.
    ///
    /// Two things happen here. First the pool is narrowed to what the
    /// sub-agent's own definition allows — passing the parent's pool through
    /// unchanged made every `tools:` / `disallowed_tools:` list in every agent
    /// definition decorative, so a read-only researcher could still write files.
    /// Second, the `Agent` tool is re-registered one level deeper (or dropped at
    /// the limit), which is what makes nesting finite and blocks a forked worker
    /// from forking again.
    fn subagent_pool(&self, definition: &AgentDefinition, run_mode: AgentRunMode) -> Arc<ToolPool> {
        let child_depth = self.depth + 1;
        let agent_allowed = self
            .tools
            .filter_for_agent(&definition.tools, &definition.disallowed_tools)
            .contains_key(AGENT_TOOL_NAME);
        let nested: Option<Arc<dyn Tool>> = (agent_allowed && child_depth < MAX_SUBAGENT_DEPTH)
            .then(|| {
                let mut nested = self.clone();
                nested.depth = child_depth;
                nested.is_forked_worker =
                    self.is_forked_worker || matches!(run_mode, AgentRunMode::Fork);
                Arc::new(nested) as Arc<dyn Tool>
            });
        build_subagent_pool(self.tools.as_ref(), definition, nested)
    }

    /// Builder-style: attach a factory so the `Tool` impl can run.
    pub fn with_chat_factory(mut self, factory: Arc<dyn ChatRequestFactory>) -> Self {
        self.chat_factory = Some(factory);
        self
    }

    /// Builder-style: attach a host that materialises temp child sessions.
    pub fn with_session_host(mut self, host: Arc<dyn SubagentSessionHost>) -> Self {
        self.session_host = Some(host);
        self
    }

    /// Core entry point used by host code that already has a
    /// `ChatRequest` in hand. The `Tool` impl below uses
    /// [`AgentTool::dispatch`] directly because the factory needs to see
    /// the resolved [`AgentDefinition`] *before* building the request
    /// (to honour `omit_claude_md`, MCP filters, etc.).
    pub async fn call(
        &self,
        invocation: AgentInvocation,
        chat_request: ChatRequest,
        initial_attachments: Vec<Attachment>,
    ) -> AppResult<AgentToolResult> {
        let (agent_type, definition) = self.resolve_definition(&invocation)?;
        // Host-side path: parent prompt is the one currently in the
        // request (the host built it from the user's settings).
        let parent_hint = Some(chat_request.system_prompt.clone());
        self.dispatch(
            invocation,
            agent_type,
            definition,
            chat_request,
            initial_attachments,
            parent_hint,
            // Host-side path has no parent ToolUseContext; without a
            // DB-derived path the sub-agent runs without a CWD.
            None,
            None,
            None,
        )
        .await
    }

    /// Resolve `subagent_type` → concrete `(agent_type, AgentDefinition)`
    /// honouring the MCP-availability filter.
    fn resolve_definition(
        &self,
        invocation: &AgentInvocation,
    ) -> AppResult<(String, AgentDefinition)> {
        let agent_type = self.resolve_agent_type(invocation)?;
        let mcp_available = self.mcp.available_servers();
        let active = self.registry.filter_by_mcp(&mcp_available);
        let definition = active
            .get(&agent_type)
            .cloned()
            .ok_or_else(|| AppError::Invalid(format!("unknown agent type: {agent_type}")))?;
        Ok((agent_type, definition))
    }

    /// Run-mode + parent-prompt resolution + delegation to [`run_agent`].
    /// Shared between [`AgentTool::call`] and the `Tool` impl.
    ///
    /// `parent_hint` is whatever system prompt the caller knows about
    /// the parent agent. Only consulted in [`AgentRunMode::Fork`].
    #[allow(clippy::too_many_arguments)]
    async fn dispatch(
        &self,
        invocation: AgentInvocation,
        agent_type: String,
        definition: AgentDefinition,
        chat_request: ChatRequest,
        initial_attachments: Vec<Attachment>,
        parent_hint: Option<String>,
        parent_cwd: Option<std::path::PathBuf>,
        parent_ctx: Option<&crate::ai::agent::core::context::ToolUseContext>,
        tool_call_id: Option<&str>,
    ) -> AppResult<AgentToolResult> {
        let run_mode = if agent_type == AGENT_FORK {
            AgentRunMode::Fork
        } else if invocation.run_in_background || definition.background {
            AgentRunMode::Background
        } else {
            AgentRunMode::Foreground
        };

        let parent_system_prompt = if matches!(run_mode, AgentRunMode::Fork) {
            parent_hint.filter(|s| !s.trim().is_empty())
        } else {
            None
        };

        let model = chat_request.model.clone();
        let provider = chat_request.provider.id.clone();

        // Prefer a dedicated temp child session when the host wired one and
        // we have a parent session + tool call id to attach to.
        let mut child_session: Option<SpawnedTempSession> = None;
        let mut on_text_delta = None;
        let mut on_tool_event = None;
        let mut session_id = parent_ctx.and_then(|c| c.session_id.clone());
        let mut correlation_id = parent_ctx.and_then(|c| c.correlation_id.clone());

        if let (Some(host), Some(parent_sid), Some(call_id)) = (
            self.session_host.as_ref(),
            parent_ctx.and_then(|c| c.session_id.as_deref()),
            tool_call_id,
        ) {
            let title = if invocation.description.trim().is_empty() {
                invocation.prompt.chars().take(60).collect::<String>()
            } else {
                invocation.description.clone()
            };
            match host.prepare_temp_session(
                parent_sid,
                parent_ctx.and_then(|c| c.correlation_id.as_deref()),
                call_id,
                &title,
                &invocation.prompt,
            ) {
                Ok(spawned) => {
                    let hooks =
                        host.begin_child_stream(&spawned.session_id, &spawned.user_message_id);
                    on_text_delta = Some(hooks.on_text_delta);
                    on_tool_event = Some(hooks.on_tool_event);
                    session_id = Some(spawned.session_id.clone());
                    correlation_id = Some(spawned.user_message_id.clone());
                    child_session = Some(spawned);
                }
                Err(e) => {
                    // Temp session is required for the model-driven path —
                    // surface the failure rather than silently sharing the
                    // parent transcript.
                    return Err(e);
                }
            }
        }

        let child_session_id = child_session.as_ref().map(|c| c.session_id.clone());

        let worker_tools = self.subagent_pool(&definition, run_mode);
        let result = run_agent(RunAgentParams {
            definition,
            prompt: invocation.prompt.clone(),
            run_mode,
            chat_request,
            tools: worker_tools,
            task_store: self.task_store.clone(),
            engine: self.engine.clone(),
            initial_attachments,
            permission_override: None,
            parent_system_prompt,
            on_text_delta,
            on_tool_event,
            query_source: None,
            // Sub-agents inherit the parent's DB-derived project path.
            // NEVER the host process directory — if the parent has no
            // project path, the sub-agent gets none either.
            project_cwd: parent_cwd,
            abort_signal: parent_ctx.map(|c| c.abort.clone()),
            session_id,
            // Role board stays on the parent/project scope, not the temp id.
            role_state_scope_id: parent_ctx.and_then(|c| c.role_state_scope_id.clone()),
            correlation_id,
            token_stats: parent_ctx.and_then(|c| c.token_stats.clone()),
            session_logger: parent_ctx.and_then(|c| c.session_logger.clone()),
        })
        .await;

        match result {
            Ok(run) => {
                if let (Some(host), Some(child)) = (self.session_host.as_ref(), child_session.as_ref())
                {
                    if let Err(e) = host.finalize_temp_session(child, &run, &model, &provider) {
                        eprintln!(
                            "AgentTool: finalize temp session {} failed: {e}",
                            child.session_id
                        );
                    }
                }
                Ok(self.shape_result(run_mode, run, child_session_id))
            }
            Err(e) => {
                // Cancellation and failure both land here, and both have
                // already announced a running child to the UI.
                if let (Some(host), Some(child)) =
                    (self.session_host.as_ref(), child_session.as_ref())
                {
                    host.abandon_temp_session(
                        child,
                        parent_ctx
                            .and_then(|c| c.session_id.as_deref())
                            .unwrap_or_default(),
                        parent_ctx.and_then(|c| c.correlation_id.as_deref()),
                        tool_call_id,
                        &e.to_string(),
                    );
                }
                Err(e)
            }
        }
    }

    fn resolve_agent_type(&self, invocation: &AgentInvocation) -> AppResult<String> {
        if let Some(t) = invocation.subagent_type.as_ref().filter(|s| !s.is_empty()) {
            if t == AGENT_FORK && self.is_forked_worker {
                return Err(AppError::Invalid(
                    "recursive fork is not allowed".into(),
                ));
            }
            return Ok(t.clone());
        }
        if self.fork_enabled && !self.is_forked_worker {
            return Ok(AGENT_FORK.into());
        }
        Ok(AGENT_GENERAL_PURPOSE.into())
    }

    /// Describe a finished run to the parent.
    ///
    /// Every mode reports [`AgentToolResult::Completed`], because every mode
    /// gets here by awaiting [`run_agent`] to completion. `Background` and
    /// `Fork` used to report `AsyncLaunched` instead, which was wrong twice
    /// over: the model was told to wait for a `<task-notification>` that nobody
    /// ever queues (so it would idle or re-dispatch), and the UI renders that
    /// status as a permanently spinning card. Discarding `final_text` also threw
    /// away the only thing the sub-agent produced.
    fn shape_result(
        &self,
        _mode: AgentRunMode,
        result: RunAgentResult,
        child_session_id: Option<String>,
    ) -> AgentToolResult {
        AgentToolResult::Completed {
            agent_id: result.agent_id.0,
            task_id: result.task_id.0,
            text: result.final_text,
            tool_calls: result.tool_call_count,
            child_session_id,
        }
    }
}

impl Tool for AgentTool {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    fn validate(&self, input: &serde_json::Value) -> AppResult<()> {
        let prompt = input
            .get("prompt")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .unwrap_or("");
        if prompt.is_empty() {
            return Err(AppError::Invalid("Agent: `prompt` must be non-empty".into()));
        }
        Ok(())
    }

    fn execute<'a>(&'a self, invocation: ToolInvocation<'a>) -> ToolFuture<'a> {
        Box::pin(async move {
            let factory = match self.chat_factory.as_ref() {
                Some(f) => f.clone(),
                None => {
                    return Ok(ToolResult::error(
                        "Agent tool has no ChatRequestFactory wired; host must call \
                         AgentTool::with_chat_factory(...)",
                    ));
                }
            };

            let invocation_args: AgentInvocation = match serde_json::from_value(invocation.input.clone()) {
                Ok(a) => a,
                Err(e) => {
                    return Ok(ToolResult::error(format!("Agent input invalid: {e}")));
                }
            };

            let (agent_type, definition) = match self.resolve_definition(&invocation_args) {
                Ok(pair) => pair,
                Err(e) => return Ok(ToolResult::error(e.to_string())),
            };

            // Factory sees the resolved definition so it can honour
            // `omit_claude_md`, `requiredMcpServers`, etc., and emit
            // initial attachments (user-context, plan-mode banner, …).
            let (chat_request, initial_attachments) =
                match factory.build(
                    &invocation_args.prompt,
                    &agent_type,
                    &definition,
                    invocation.context.session_id.as_deref(),
                ) {
                    Ok(pair) => pair,
                    Err(e) => {
                        return Ok(ToolResult::error(format!(
                            "Agent could not build chat request: {e}"
                        )));
                    }
                };

            // Model-driven path: parent system prompt was stashed on
            // the parent's ToolUseContext at the start of its query
            // loop (see `runner::run_agent`).
            let parent_hint = invocation.context.parent_system_prompt.clone();

            // Propagate the parent's working directory only when it is a
            // real (DB-derived) path; an empty cwd stays empty.
            let parent_cwd = if invocation.context.cwd.as_os_str().is_empty() {
                None
            } else {
                Some(invocation.context.cwd.clone())
            };

            let tool_call_id = invocation.id.as_str().to_string();

            match self
                .dispatch(
                    invocation_args,
                    agent_type,
                    definition,
                    chat_request,
                    initial_attachments,
                    parent_hint,
                    parent_cwd,
                    Some(invocation.context),
                    Some(tool_call_id.as_str()),
                )
                .await
            {
                Ok(result) => {
                    let v = serde_json::to_value(&result).unwrap_or_else(|e| {
                        serde_json::json!({ "error": format!("serialise: {e}") })
                    });
                    Ok(ToolResult::ok(v))
                }
                Err(e) => Ok(ToolResult::error(e.to_string())),
            }
        })
    }
}

/// Narrow `parent` to what `definition` allows, substituting the `Agent` entry
/// with `nested` (or removing it when `nested` is `None`).
fn build_subagent_pool(
    parent: &ToolPool,
    definition: &AgentDefinition,
    nested: Option<Arc<dyn Tool>>,
) -> Arc<ToolPool> {
    let pool = ToolPool::new();
    for (name, tool) in parent.filter_for_agent(&definition.tools, &definition.disallowed_tools) {
        if name == AGENT_TOOL_NAME {
            continue;
        }
        pool.register_arc(tool);
    }
    // Shared so a sub-agent's task progress stays visible to the run that
    // spawned it.
    pool.share_todo_list_from(parent);
    if let Some(nested) = nested {
        pool.register_arc(nested);
    }
    Arc::new(pool)
}

/// Static [`ToolSpec`] for the `Agent` tool.
fn agent_tool_spec() -> ToolSpec {
    ToolSpec {
        name: AGENT_TOOL_NAME.to_string(),
        description: "Spawn a sub-agent to perform a focused multi-step task. \
            Provide a short `description`, a self-contained `prompt`, and \
            optionally `subagent_type` (one of the registered agent types). \
            Set `run_in_background: true` for non-blocking execution; \
            results then arrive via task-notification on the next turn."
            .to_string(),
        schema: serde_json::json!({
            "type": "object",
            "properties": {
                "description": { "type": "string" },
                "prompt": { "type": "string" },
                "subagent_type": { "type": "string" },
                "run_in_background": { "type": "boolean" },
                "team_name": { "type": "string" },
                "name": { "type": "string" }
            },
            "required": ["description", "prompt"]
        }),
        read_only: false,
        concurrency_safe: false,
    }
}

#[cfg(test)]
mod subagent_pool_tests {
    use super::*;
    use crate::ai::agent::tools::grep::GrepTool;
    use crate::ai::agent::tools::list_files::ListFilesTool;

    fn definition(tools: &[&str], denied: &[&str]) -> AgentDefinition {
        let mut def = AgentDefinition::builtin("test-agent", "");
        def.tools = tools.iter().map(|s| s.to_string()).collect();
        def.disallowed_tools = denied.iter().map(|s| s.to_string()).collect();
        def
    }

    fn parent_pool() -> ToolPool {
        let pool = ToolPool::new();
        pool.register(GrepTool::new());
        pool.register(ListFilesTool::new());
        pool
    }

    /// A sub-agent's declared tool list is a permission boundary, not a hint:
    /// handing it the parent's whole pool would let a read-only researcher edit
    /// the project.
    #[test]
    fn a_subagent_only_receives_the_tools_its_definition_allows() {
        let parent = parent_pool();
        let child = build_subagent_pool(&parent, &definition(&["Grep"], &[]), None);

        assert!(child.get("Grep").is_some(), "allowed tool is present");
        assert!(
            child.get("ListFiles").is_none(),
            "a tool the definition never listed must not leak through"
        );
    }

    #[test]
    fn a_denied_tool_is_removed_even_under_a_wildcard() {
        let parent = parent_pool();
        let child = build_subagent_pool(&parent, &definition(&["*"], &["ListFiles"]), None);

        assert!(child.get("Grep").is_some());
        assert!(child.get("ListFiles").is_none());
    }

    /// At the nesting limit the `Agent` tool simply stops being handed down, so
    /// the chain terminates instead of spawning forever.
    #[test]
    fn the_agent_tool_is_only_passed_down_when_one_is_supplied() {
        let parent = parent_pool();
        parent.register(GrepTool::new());

        let without = build_subagent_pool(&parent, &definition(&["*"], &[]), None);
        assert!(
            without.get(AGENT_TOOL_NAME).is_none(),
            "no replacement supplied ⇒ the child cannot spawn sub-agents"
        );

        let stand_in: Arc<dyn Tool> = Arc::new(GrepTool::new());
        let with = build_subagent_pool(&parent, &definition(&["*"], &[]), Some(stand_in));
        assert!(with.get("Grep").is_some());
    }

    /// Sub-agents must stay possible, and the chain must stay finite.
    const _NESTING_IS_BOUNDED: () = assert!(MAX_SUBAGENT_DEPTH >= 1 && MAX_SUBAGENT_DEPTH < 10);
}
