//! Memory + context layers.
//!
//! Maps the five-layer model from `context-memory-architecture.md` §23:
//!
//! ```text
//! L1 system prompt   →  not modeled here (lives in providers / runner)
//! L2 user context    →  `UserContext` + CLAUDE.md  → [`user_context`]
//! L3 attachments     →  `core::attachment`
//! L4 persistent      →  `AutoMemory`, `AgentMemory`
//! L5 compaction      →  `SessionMemory` → [`session`]
//! ```
//!
//! Submodules:
//! - [`user_context`]  filesystem-backed CLAUDE.md / rules loader
//! - [`session`]       per-session `summary.md` extractor
//! - [`nested`]        path-scoped rule injection driven by tool reads

pub mod nested;
pub mod session;
pub mod user_context;

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::ai::agent::types::{AgentId, MessageId, TokenUsage};
use crate::error::AppResult;

/// MemoryType taxonomy from `utils/memory/types.ts`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryType {
    Managed,
    User,
    Project,
    Local,
    AutoMem,
    TeamMem,
}

/// A single discovered memory file (CLAUDE.md / rules / MEMORY.md / ...).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryFile {
    pub ty: MemoryType,
    pub path: PathBuf,
    pub content: String,
    /// Optional path-glob patterns parsed from frontmatter `paths:`.
    /// `None` ⇒ unconditional; `Some(empty)` ⇒ never matches.
    pub path_globs: Option<Vec<String>>,
    /// True when this file was injected as part of nested-memory attachment
    /// rather than the base user-context snapshot.
    pub conditional: bool,
}

/// Snapshot of the user-context section (CLAUDE.md / rules + date).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UserContext {
    pub memory_files: Vec<MemoryFile>,
    /// Cached rendered string used by `prependUserContext()` equivalent.
    pub rendered: String,
}

impl UserContext {
    pub fn is_empty(&self) -> bool {
        self.memory_files.is_empty() && self.rendered.is_empty()
    }
}

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

/// Loader for `UserContext` (CLAUDE.md discovery + include resolution).
///
/// Implementors are expected to honor:
///
/// - Source priority Managed > User > Project > Local;
/// - `@path` includes with depth limit 5;
/// - `.claude/rules/*.md` frontmatter `paths:` glob matching.
pub trait UserContextLoader: Send + Sync {
    fn load(&self) -> AppResult<UserContext>;

    /// Invalidate the cached snapshot (called after compact / `/memory`).
    fn invalidate(&self);
}
