//! `runAgent` equivalent — turn one [`AgentDefinition`] into one running
//! sub-agent that goes through the [`QueryEngine`] loop.
//!
//! Responsibilities lifted from `tools/AgentTool/runAgent.ts`:
//!
//! - Resolve model / effort / max-turns.
//! - Build the initial prompt (regular vs fork vs resume).
//! - Choose the user/system context to inject.
//! - Construct a sub-agent [`ToolUseContext`].
//! - Run `SubagentStart` hooks (placeholder).
//! - Init agent-specific MCP servers (placeholder).
//! - Call [`QueryEngine::query`].
//! - Persist transcript metadata.
//! - Run cleanup in a finally-like block.

use std::sync::Arc;

use crate::ai::agent::config::definition::{AgentDefinition, Isolation};
use crate::ai::agent::core::attachment::Attachment;
use crate::ai::agent::core::context::{AbortHandle, ToolUseContext};
use crate::ai::agent::core::permission::PermissionMode;
use crate::ai::agent::core::task::{Task, TaskId, TaskStore};
use crate::ai::agent::core::worktree::WorktreeHandle;
use crate::ai::agent::exec::query::{QueryEngine, QueryRequest, QueryResult};
use crate::ai::agent::tools::ToolPool;
use crate::ai::agent::types::{AgentId, AgentRunMode, QuerySource, TokenUsage};
use crate::ai::chat::ChatRequest;
use crate::error::AppResult;

/// Inputs to [`run_agent`].
pub struct RunAgentParams {
    pub definition: AgentDefinition,
    pub prompt: String,
    pub run_mode: AgentRunMode,
    pub chat_request: ChatRequest,
    pub tools: Arc<ToolPool>,
    pub task_store: Arc<TaskStore>,
    pub engine: Arc<dyn QueryEngine>,
    pub initial_attachments: Vec<Attachment>,
    /// Overrides the definition's `permissionMode` when set.
    pub permission_override: Option<PermissionMode>,
    /// Parent agent's already-rendered system prompt. Only consulted in
    /// [`AgentRunMode::Fork`]; ignored otherwise. Mirrors
    /// `forkSubagent`'s "inherit parent prompt" semantics.
    pub parent_system_prompt: Option<String>,
}

/// Output of [`run_agent`].
#[derive(Debug, Clone)]
pub struct RunAgentResult {
    pub agent_id: AgentId,
    pub task_id: TaskId,
    pub final_text: Option<String>,
    pub usage: TokenUsage,
    pub tool_call_count: u32,
}

/// Drive a single sub-agent end-to-end.
///
/// For background runs, the caller should wrap this in `tokio::spawn` and
/// rely on [`TaskStore`] / [`crate::ai::agent::core::attachment::NotificationQueue`]
/// to surface the result to the parent loop.
pub async fn run_agent(params: RunAgentParams) -> AppResult<RunAgentResult> {
    let RunAgentParams {
        definition,
        prompt,
        run_mode,
        mut chat_request,
        tools,
        task_store,
        engine,
        initial_attachments,
        permission_override,
        parent_system_prompt,
    } = params;

    let agent_id = AgentId::new();

    let mut task = Task::new_local(agent_id.clone(), &definition.agent_type, prompt.clone());
    task.background = matches!(run_mode, AgentRunMode::Background | AgentRunMode::Fork)
        || definition.background;
    let task_id = task_store.register(task);

    let host_cwd = std::env::current_dir().unwrap_or_default();

    // Optional git-worktree isolation. The handle's `Drop` removes the
    // worktree on the way out, including the error path — keep it
    // alive for the entire `drive` call below.
    let worktree = match definition.isolation {
        Isolation::Worktree => match WorktreeHandle::acquire(&host_cwd) {
            Ok(h) => Some(h),
            Err(e) => {
                task_store.fail(&task_id, format!("worktree setup failed: {e}"));
                return Err(e);
            }
        },
        Isolation::None | Isolation::Remote => None,
    };
    let cwd = worktree
        .as_ref()
        .map(|h| h.path.clone())
        .unwrap_or_else(|| host_cwd.clone());

    let permission_mode = permission_override
        .or(definition.permission_mode)
        .unwrap_or(PermissionMode::Default);

    // Apply the agent's prompts before the engine sees the chat. This
    // is the *only* point where `definition.system_prompt`,
    // `initial_prompt`, `critical_system_reminder`, and parent-prompt
    // inheritance can land — the engine treats `chat_request` as a
    // black-box payload from here on.
    chat_request.system_prompt = compose_system_prompt(
        &definition,
        run_mode,
        parent_system_prompt.as_deref(),
        &cwd,
    );
    chat_request.prompt = compose_user_prompt(&definition, &prompt);

    // Persist the rendered system_prompt onto the context so that any
    // `Agent(...)` tool call inside this run can fork-inherit it
    // without having to thread `chat_request` through the Tool trait.
    let (context, abort) = ToolUseContext::builder(agent_id.clone(), cwd)
        .query_source(match run_mode {
            AgentRunMode::Fork => QuerySource::Forked,
            _ => QuerySource::Subagent,
        })
        .permission_mode(permission_mode)
        .parent_system_prompt(chat_request.system_prompt.clone())
        .build();

    task_store.set_state(&task_id, crate::ai::agent::core::task::TaskState::Running);

    let result = drive(
        engine,
        chat_request,
        context,
        tools,
        initial_attachments,
        definition.max_turns,
    )
    .await;

    // Cleanup window. Mirrors the `finally` block in `runAgent.ts`:
    // MCP teardown, prompt-cache release, file cache eviction,
    // transcript flush, and `git worktree remove` via Drop.
    drop(abort);
    drop(worktree);

    match result {
        Ok(qr) => {
            task_store.complete(&task_id, qr.final_text.clone(), qr.usage.clone());
            Ok(RunAgentResult {
                agent_id,
                task_id,
                final_text: qr.final_text,
                usage: qr.usage,
                tool_call_count: qr.tool_call_count,
            })
        }
        Err(e) => {
            task_store.fail(&task_id, e.to_string());
            Err(e)
        }
    }
}

async fn drive(
    engine: Arc<dyn QueryEngine>,
    chat_request: ChatRequest,
    context: Arc<ToolUseContext>,
    tools: Arc<ToolPool>,
    initial_attachments: Vec<Attachment>,
    max_turns: Option<u32>,
) -> AppResult<QueryResult> {
    let request = QueryRequest {
        chat: chat_request,
        source: context.query_source,
        max_turns,
        initial_attachments,
        on_text_delta: None,
    };
    engine.query(request, context, tools).await
}

/// Avoid an unused-import warning while `AbortHandle` is exported for callers.
#[allow(dead_code)]
fn _abort_handle_marker(_h: AbortHandle) {}

// ───────────────────────── prompt composition ─────────────────────────

/// Assemble the final `system_prompt` for a sub-agent run.
///
/// Order (mirrors upstream `runAgent.ts` + `enhanceSystemPromptWithEnvDetails`):
///
/// 1. `[parent_prompt + "\n\n---\n\n"]` — only in Fork mode.
/// 2. `definition.system_prompt`.
/// 3. `<env>` block (cwd / platform / date) — skipped when env details are
///    already present (idempotent for resumes).
/// 4. `critical_system_reminder` — appended last so it stays visible
///    after long contexts get compacted.
fn compose_system_prompt(
    def: &AgentDefinition,
    run_mode: AgentRunMode,
    parent_system_prompt: Option<&str>,
    cwd: &std::path::Path,
) -> String {
    let mut out = String::with_capacity(2048);

    if matches!(run_mode, AgentRunMode::Fork) {
        if let Some(parent) = parent_system_prompt.map(str::trim).filter(|s| !s.is_empty()) {
            out.push_str(parent);
            out.push_str("\n\n---\n\n");
        }
    }

    let body = def.system_prompt.trim();
    if !body.is_empty() {
        out.push_str(body);
        out.push('\n');
    }

    let env = env_details_block(cwd);
    if !out.contains("<env>") && !env.is_empty() {
        out.push('\n');
        out.push_str(&env);
    }

    if let Some(reminder) = def
        .critical_system_reminder
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        out.push_str("\n\n");
        out.push_str(reminder);
    }

    out
}

/// Compose the user prompt — `initial_prompt` prepended to the caller's
/// prompt text. The initial prompt is a one-time conditioning string
/// (e.g. "Focus on the database layer"); the user's actual request comes
/// after it.
fn compose_user_prompt(def: &AgentDefinition, user_prompt: &str) -> String {
    match def
        .initial_prompt
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        Some(initial) => format!("{initial}\n\n{}", user_prompt.trim_start()),
        None => user_prompt.to_string(),
    }
}

/// Render the `<env>` block: working directory + platform. Mirrors
/// `enhanceSystemPromptWithEnvDetails` minus the date (we don't pull a
/// chrono dep just for one line — providers stamp their own date into
/// the request).
fn env_details_block(cwd: &std::path::Path) -> String {
    let cwd_str = cwd.display().to_string();
    let platform = std::env::consts::OS;
    let arch = std::env::consts::ARCH;
    format!("<env>\nWorking directory: {cwd_str}\nPlatform: {platform}/{arch}\n</env>")
}
