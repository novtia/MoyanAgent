//! Memory + context layers.
//!
//! Maps the layered model from `context-memory-architecture.md` §23:
//!
//! ```text
//! L1 system prompt   →  not modeled here (lives in providers / runner)
//! L2 project rules   →  `.moyan/*.md` prepended as a hidden user turn
//!                       (see `app/project_rules`)
//! L3 attachments     →  `core::attachment`
//! L4 persistent      →  `AutoMemory`, `AgentMemory`
//! ```

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

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
