//! Memory + context layers.
//!
//! Maps the five-layer model from `context-memory-architecture.md` §23:
//!
//! ```text
//! L1 system prompt   →  not modeled here (lives in providers / runner)
//! L2 project rules   →  `.moyan/*.md` prepended as a hidden user turn
//!                       (see `app/project_rules`)
//! L3 attachments     →  `core::attachment`
//! L4 persistent      →  `AutoMemory`, `AgentMemory`
//! L5 compaction      →  `SessionMemory` → [`session`]
//! ```
//!
//! Submodules:
//! - [`session`]       per-session `summary.md` extractor
//! - [`compaction`]    history compression once token-budget crosses a threshold
//! - [`tool_chain`]    in-loop tool round window (latest TodoList round only)

pub mod compaction;
pub mod session;
pub mod tool_chain;

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::ai::agent::types::{AgentId, MessageId, TokenUsage};
use crate::error::AppResult;

/// AutoMem scope. Persistent, cross-session memory.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AutoMemory {
    pub dir: Option<PathBuf>,
    pub enabled: bool,
}

/// Per-agent persistent memory (`agent-memory/<agentType>/...`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentMemory {
    pub agent_type: String,
    pub scope: MemoryScope,
    pub dir: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryScope {
    User,
    Project,
    Local,
}

/// Current-session summary file (`session-memory/summary.md`).
///
/// The runner is responsible for invoking [`SessionMemoryExtractor::extract`]
/// from a post-sampling hook when token-pressure thresholds are reached.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMemory {
    pub session_id: String,
    pub agent_id: AgentId,
    pub summary_path: PathBuf,
    pub last_summarized_message_id: Option<MessageId>,
    pub last_usage: TokenUsage,
}

/// Strategy for producing a [`SessionMemory`] update. Implementations
/// typically delegate to a forked agent constrained to `Edit` the summary
/// file, exactly like `extractSessionMemory()` in TS.
pub trait SessionMemoryExtractor: Send + Sync {
    fn extract(&self, current: &SessionMemory) -> AppResult<SessionMemory>;
}
