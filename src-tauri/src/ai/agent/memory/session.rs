//! Session Memory: per-session `summary.md` extractor.
//!
//! The TS-side `extractSessionMemory()` (see
//! `context-memory-architecture.md` §11) runs a *forked agent* whose
//! only permission is to `Edit` the summary file. Our agent loop doesn't
//! yet have a tool-permission carve-out granular enough to enforce
//! "exact path Edit-only", so this implementation skips the forked
//! agent and writes the summary file directly from the host process.
//!
//! What we keep:
//!
//! - The path layout (`<session_dir>/session-memory/summary.md`).
//! - The token / message thresholds that gate when to update.
//! - The default template (Session Title, Current State, ...).
//! - The [`SessionMemoryExtractor`] trait surface so a future
//!   forked-agent implementation can drop in unchanged.
//!
//! What we omit:
//!
//! - Per-extract permission-resolver carve-outs.
//! - The post-sampling hook that schedules extraction asynchronously.
//!
//! `extract_now` exists so a Tauri command can trigger extraction
//! synchronously for tests, demos, and the eventual "auto" path.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::ai::agent::memory::{SessionMemory, SessionMemoryExtractor};
use crate::ai::agent::core::task::Task;
use crate::ai::tokens::TokenUsage;
use crate::error::AppResult;

/// Configuration thresholds mirroring `sessionMemoryUtils.ts`.
#[derive(Debug, Clone)]
pub struct SessionMemoryConfig {
    /// Minimum total tokens before the first extraction is allowed.
    pub minimum_tokens_to_init: u64,
    /// Minimum delta between updates.
    pub minimum_tokens_between_updates: u64,
    /// Minimum tool calls between updates (or any assistant turn
    /// without tool_use also counts as "enough activity").
    pub tool_calls_between_updates: u32,
}

impl Default for SessionMemoryConfig {
    fn default() -> Self {
        Self {
            minimum_tokens_to_init: 10_000,
            minimum_tokens_between_updates: 5_000,
            tool_calls_between_updates: 3,
        }
    }
}

/// Default markdown template. Matches the sections enumerated in
/// `services/SessionMemory/prompts.ts`.
pub const DEFAULT_TEMPLATE: &str = "# Session Memory

## Session Title

## Current State

## Task specification

## Files and Functions

## Workflow

## Errors & Corrections

## Codebase and System Documentation

## Learnings

## Key results

## Worklog
";

/// Filesystem-backed extractor. The host caller is expected to provide
/// the resolved `<session_dir>` path; we own the `session-memory/`
/// subdirectory creation and the `summary.md` file lifecycle.
pub struct FsSessionMemoryExtractor {
    config: SessionMemoryConfig,
    state: Mutex<Option<SessionMemory>>,
}

impl FsSessionMemoryExtractor {
    pub fn new() -> Self {
        Self {
            config: SessionMemoryConfig::default(),
            state: Mutex::new(None),
        }
    }

    pub fn with_config(config: SessionMemoryConfig) -> Self {
        Self {
            config,
            state: Mutex::new(None),
        }
    }

    /// Compute the summary path for a given session directory.
    pub fn summary_path(session_dir: &Path) -> PathBuf {
        session_dir.join("session-memory").join("summary.md")
    }

    /// Ensure the summary file exists, populated with the default
    /// template if missing. Returns the canonical absolute path.
    pub fn ensure_summary_file(session_dir: &Path) -> AppResult<PathBuf> {
        let mem_dir = session_dir.join("session-memory");
        fs::create_dir_all(&mem_dir)?;
        let path = mem_dir.join("summary.md");
        if !path.exists() {
            fs::write(&path, DEFAULT_TEMPLATE)?;
        }
        Ok(path)
    }

    /// Decide whether an extraction should run given the current usage
    /// and tool-call activity.
    pub fn should_update(
        &self,
        usage: &TokenUsage,
        tool_calls_since_last: u32,
    ) -> bool {
        let guard = match self.state.lock() {
            Ok(g) => g,
            Err(_) => return false,
        };
        let current_total = usage.total_tokens.unwrap_or(0).max(0) as u64;
        let Some(prev) = guard.as_ref() else {
            return current_total >= self.config.minimum_tokens_to_init;
        };
        let prev_total = prev.last_usage.total_tokens.unwrap_or(0).max(0) as u64;
        let delta = current_total.saturating_sub(prev_total);
        delta >= self.config.minimum_tokens_between_updates
            || tool_calls_since_last >= self.config.tool_calls_between_updates
    }

    /// Run an extraction now. Writes a fresh summary section block
    /// derived from `task` (or the default template if absent).
    ///
    /// This is the synchronous-host fallback for the docs' forked-agent
    /// strategy; once the tool-loop has fine-grained per-tool path
    /// carve-outs we can swap in the forked path behind the same
    /// trait surface.
    pub fn extract_now(
        &self,
        session_id: &str,
        session_dir: &Path,
        task: Option<&Task>,
    ) -> AppResult<SessionMemory> {
        let summary_path = Self::ensure_summary_file(session_dir)?;

        let mut body = DEFAULT_TEMPLATE.to_string();
        if let Some(t) = task {
            body = render_template_from_task(t);
        }
        fs::write(&summary_path, body)?;

        let sm = SessionMemory {
            session_id: session_id.to_string(),
            agent_id: task
                .map(|t| t.agent_id.clone())
                .unwrap_or_else(crate::ai::agent::types::AgentId::new),
            summary_path,
            last_summarized_message_id: None,
            last_usage: task.map(|t| t.usage.clone()).unwrap_or_default(),
        };
        if let Ok(mut g) = self.state.lock() {
            *g = Some(sm.clone());
        }
        Ok(sm)
    }
}

impl Default for FsSessionMemoryExtractor {
    fn default() -> Self {
        Self::new()
    }
}

impl SessionMemoryExtractor for FsSessionMemoryExtractor {
    fn extract(&self, current: &SessionMemory) -> AppResult<SessionMemory> {
        let session_dir = current
            .summary_path
            .parent()
            .and_then(Path::parent)
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        self.extract_now(&current.session_id, &session_dir, None)
    }
}

fn render_template_from_task(task: &Task) -> String {
    let prompt = task.prompt.lines().take(20).collect::<Vec<_>>().join("\n");
    let result = task.result.as_deref().unwrap_or("(no result yet)");
    let err = task.error.as_deref().unwrap_or("");
    let usage = format!(
        "prompt={} completion={} total={}",
        task.usage.prompt_tokens.unwrap_or(0),
        task.usage.completion_tokens.unwrap_or(0),
        task.usage.total_tokens.unwrap_or(0),
    );
    format!(
        "# Session Memory\n\n\
         ## Session Title\n{title}\n\n\
         ## Current State\nLatest agent task: `{agent_type}` ({state:?}). Token usage: {usage}.\n\n\
         ## Task specification\n{prompt}\n\n\
         ## Files and Functions\n\n\
         ## Workflow\n\n\
         ## Errors & Corrections\n{err}\n\n\
         ## Codebase and System Documentation\n\n\
         ## Learnings\n\n\
         ## Key results\n{result}\n\n\
         ## Worklog\n- task `{task_id}` ended at ms={ended:?}\n",
        title = task.agent_type,
        agent_type = task.agent_type,
        state = task.state,
        usage = usage,
        prompt = prompt,
        err = err,
        result = result,
        task_id = task.id,
        ended = task.ended_at_ms,
    )
}
