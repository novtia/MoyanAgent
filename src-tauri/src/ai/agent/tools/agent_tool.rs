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
use crate::ai::chat::ChatRequest;
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
    fn build(
        &self,
        prompt: &str,
        agent_type: &str,
        definition: &AgentDefinition,
    ) -> AppResult<(ChatRequest, Vec<Attachment>)>;
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
    },
    /// Background run launched. Parent should expect a `<task-notification>`.
    AsyncLaunched {
        agent_id: String,
        task_id: String,
        output_file: Option<String>,
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
    /// Cached tool spec exposed to the model.
    spec: ToolSpec,
    /// Fork gate: mirrors the `tengu_*` feature flag check. When false,
    /// omitting `subagent_type` falls back to `general-purpose` instead of
    /// the fork agent.
    pub fork_enabled: bool,
    /// Whether this AgentTool is itself running inside a forked worker.
    /// Prevents recursive forks.
    pub is_forked_worker: bool,
}

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
            spec: agent_tool_spec(),
            fork_enabled: false,
            is_forked_worker: false,
        }
    }

    /// Builder-style: attach a factory so the `Tool` impl can run.
    pub fn with_chat_factory(mut self, factory: Arc<dyn ChatRequestFactory>) -> Self {
        self.chat_factory = Some(factory);
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
    async fn dispatch(
        &self,
        invocation: AgentInvocation,
        agent_type: String,
        definition: AgentDefinition,
        chat_request: ChatRequest,
        initial_attachments: Vec<Attachment>,
        parent_hint: Option<String>,
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

        let result = run_agent(RunAgentParams {
            definition,
            prompt: invocation.prompt.clone(),
            run_mode,
            chat_request,
            tools: self.tools.clone(),
            task_store: self.task_store.clone(),
            engine: self.engine.clone(),
            initial_attachments,
            permission_override: None,
            parent_system_prompt,
            on_text_delta: None,
            query_source: None,
        })
        .await?;

        Ok(self.shape_result(run_mode, result))
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

    fn shape_result(&self, mode: AgentRunMode, result: RunAgentResult) -> AgentToolResult {
        match mode {
            AgentRunMode::Foreground => AgentToolResult::Completed {
                agent_id: result.agent_id.0,
                task_id: result.task_id.0,
                text: result.final_text,
                tool_calls: result.tool_call_count,
            },
            AgentRunMode::Background | AgentRunMode::Fork => AgentToolResult::AsyncLaunched {
                agent_id: result.agent_id.0,
                task_id: result.task_id.0,
                output_file: None,
            },
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
                match factory.build(&invocation_args.prompt, &agent_type, &definition) {
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

            match self
                .dispatch(
                    invocation_args,
                    agent_type,
                    definition,
                    chat_request,
                    initial_attachments,
                    parent_hint,
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
