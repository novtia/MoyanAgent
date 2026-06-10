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
use crate::ai::agent::core::context::{AbortHandle, AbortSignal, ToolUseContext};
use crate::ai::agent::core::permission::PermissionMode;
use crate::ai::agent::core::task::{Task, TaskId, TaskStore};
use crate::ai::agent::core::worktree::WorktreeHandle;
use crate::ai::agent::exec::query::{QueryEngine, QueryRequest, QueryResult, ToolEventCallback};
use crate::ai::agent::tools::ToolPool;
use crate::ai::agent::types::{AgentId, AgentRunMode, QuerySource, TokenUsage};
use crate::ai::chat::{ChatRequest, ImageResult, TextDeltaCallback};
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
    /// Forward streaming deltas to the host (main REPL). Sub-agent runs
    /// leave this `None`.
    pub on_text_delta: Option<TextDeltaCallback>,
    /// Forward structured tool events (`ToolUse` / `ToolResult`) to the
    /// host the moment the engine records them. Sub-agent runs and
    /// non-agent flows leave this `None`.
    pub on_tool_event: Option<ToolEventCallback>,
    /// When set, overrides the default `Subagent` / `Forked`
    /// [`QuerySource`] (e.g. [`QuerySource::ReplMainThread`] for the
    /// primary session).
    pub query_source: Option<QuerySource>,
    /// Project working directory for this run. MUST originate from the
    /// database (the project's `path` column) — never from the host
    /// process environment.
    ///
    /// - `Some(path)` → tools execute inside this directory and the
    ///   `<env>` block in the system prompt shows this path.
    /// - `None` → tools get **no** working directory (tools that require
    ///   one fail with an explicit error) and no `<env>` block is emitted.
    ///   The host process CWD is NEVER used as a fallback.
    pub project_cwd: Option<std::path::PathBuf>,
    /// Shared cancellation controller for the main session. When set,
    /// `cancel_generation` can abort in-flight provider/tool work promptly.
    pub abort_signal: Option<AbortSignal>,
    /// Database session id this run belongs to. Threaded onto the
    /// [`ToolUseContext`] so session-scoped tools (e.g. `RoleState`) can
    /// persist their state against the right conversation.
    pub session_id: Option<String>,
}

/// Output of [`run_agent`].
#[derive(Debug, Clone)]
pub struct RunAgentResult {
    pub agent_id: AgentId,
    pub task_id: TaskId,
    pub final_text: Option<String>,
    /// Reasoning / extended-thinking text from the final turn, when the
    /// provider returns it separately from the visible assistant reply.
    pub thinking_content: Option<String>,
    pub usage: TokenUsage,
    pub tool_call_count: u32,
    pub images: Vec<ImageResult>,
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
        on_text_delta,
        on_tool_event,
        query_source,
        project_cwd,
        abort_signal,
        session_id,
    } = params;

    let agent_id = AgentId::new();

    let mut task = Task::new_local(agent_id.clone(), &definition.agent_type, prompt.clone());
    task.background = matches!(run_mode, AgentRunMode::Background | AgentRunMode::Fork)
        || definition.background;
    let task_id = task_store.register(task);

    // Optional git-worktree isolation. The handle's `Drop` removes the
    // worktree on the way out, including the error path — keep it
    // alive for the entire `drive` call below.
    //
    // The worktree root MUST be the DB-provided project path; the host
    // process directory is never used.
    let worktree = match definition.isolation {
        Isolation::Worktree => match project_cwd.as_deref() {
            Some(root) => match WorktreeHandle::acquire(root) {
                Ok(h) => Some(h),
                Err(e) => {
                    task_store.fail(&task_id, format!("worktree setup failed: {e}"));
                    return Err(e);
                }
            },
            None => {
                let e = crate::error::AppError::Invalid(
                    "worktree isolation requires a project path (set the project's \
                     `path` in the database); refusing to use the host process directory"
                        .into(),
                );
                task_store.fail(&task_id, e.to_string());
                return Err(e);
            }
        },
        Isolation::None | Isolation::Remote => None,
    };

    // CWD used by tools: worktree → DB project path → none (empty).
    // NEVER fall back to the host process directory.
    let tool_cwd = worktree
        .as_ref()
        .map(|h| h.path.clone())
        .or_else(|| project_cwd.clone())
        .unwrap_or_default();

    let permission_mode = permission_override
        .or(definition.permission_mode)
        .unwrap_or(PermissionMode::Default);

    // Apply the agent's prompts before the engine sees the chat. This
    // is the *only* point where `definition.system_prompt`,
    // `initial_prompt`, `critical_system_reminder`, and parent-prompt
    // inheritance can land — the engine treats `chat_request` as a
    // black-box payload from here on.
    //
    // The `<env>` block is only included when a project CWD is provided.
    // Plain chat sessions (no project / no project path) get no env block.
    chat_request.system_prompt = compose_system_prompt(
        &definition,
        run_mode,
        parent_system_prompt.as_deref(),
        project_cwd.as_deref(),
    );
    chat_request.prompt = compose_user_prompt(&definition, &prompt);

    // Persist the rendered system_prompt onto the context so that any
    // `Agent(...)` tool call inside this run can fork-inherit it
    // without having to thread `chat_request` through the Tool trait.
    let resolved_source = query_source.unwrap_or_else(|| match run_mode {
        AgentRunMode::Fork => QuerySource::Forked,
        _ => QuerySource::Subagent,
    });
    let mut ctx_builder = ToolUseContext::builder(agent_id.clone(), tool_cwd)
        .query_source(resolved_source)
        .permission_mode(permission_mode)
        .parent_system_prompt(chat_request.system_prompt.clone());
    if let Some(signal) = abort_signal {
        ctx_builder = ctx_builder.abort_signal(signal);
    }
    if let Some(sid) = session_id {
        ctx_builder = ctx_builder.session_id(sid);
    }
    let (context, abort) = ctx_builder.build();

    task_store.set_state(&task_id, crate::ai::agent::core::task::TaskState::Running);

    let result = drive(
        engine,
        chat_request,
        context,
        tools,
        initial_attachments,
        definition.max_turns,
        on_text_delta,
        on_tool_event,
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
                thinking_content: qr.thinking_content,
                usage: qr.usage,
                tool_call_count: qr.tool_call_count,
                images: qr.images,
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
    on_text_delta: Option<TextDeltaCallback>,
    on_tool_event: Option<ToolEventCallback>,
) -> AppResult<QueryResult> {
    let request = QueryRequest {
        chat: chat_request,
        source: context.query_source,
        max_turns,
        initial_attachments,
        on_text_delta,
        on_tool_event,
    };
    engine.query(request, context, tools).await
}

/// Avoid an unused-import warning while `AbortHandle` is exported for callers.
#[allow(dead_code)]
fn _abort_handle_marker(_h: AbortHandle) {}

// ───────────────────────── prompt composition ─────────────────────────

/// Assemble the final `system_prompt` for a sub-agent run.
///
/// Always appends an `<env>` block with platform/shell metadata.
/// The working directory line is included only when a DB project path
/// is available (`env_cwd: Some`).
fn compose_system_prompt(
    def: &AgentDefinition,
    run_mode: AgentRunMode,
    parent_system_prompt: Option<&str>,
    env_cwd: Option<&std::path::Path>,
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

    let env = env_details_block(env_cwd);
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

/// Render the `<env>` block injected into every agent system prompt.
///
/// Platform and shell are always present so the model never needs to
/// probe the OS with Bash. The working directory comes exclusively from
/// the database project `path` column — never from the host process.
fn env_details_block(project_cwd: Option<&std::path::Path>) -> String {
    let platform = std::env::consts::OS;
    let arch = std::env::consts::ARCH;
    let shell = shell_description();
    let mut lines = vec![
        "<env>".to_string(),
        format!("Platform: {platform}/{arch}"),
        format!("Shell: {shell}"),
    ];
    match project_cwd.filter(|p| !p.as_os_str().is_empty()) {
        Some(cwd) => lines.push(format!("Working directory: {}", cwd.display())),
        None => lines.push(
            "Working directory: (none — set the project's `path` in the database to enable \
             file/shell tools)"
                .to_string(),
        ),
    }
    lines.push("</env>".to_string());
    lines.join("\n")
}

#[cfg(windows)]
fn shell_description() -> &'static str {
    "cmd.exe — use `dir`, `cd`, `type`; do NOT use `ls`, `pwd`, `find`, `uname`"
}

#[cfg(not(windows))]
fn shell_description() -> &'static str {
    "sh -c (POSIX) — use `ls`, `find`, `pwd`, etc."
}
