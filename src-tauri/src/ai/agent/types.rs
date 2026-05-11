//! Common identifier and message types for the agent subsystem.
//!
//! These map roughly to the TS-side concepts:
//!
//! | TS                          | Rust                  |
//! | --------------------------- | --------------------- |
//! | `agentId: string`           | [`AgentId`]           |
//! | `taskId: string` (`a*/r*/t*`) | [`TaskId`] in `task`  |
//! | `messageId: string`         | [`MessageId`]         |
//! | `querySource`               | [`QuerySource`]       |
//! | `MessageEvent`              | [`MessageEvent`]      |
//! | `usage`                     | [`TokenUsage`]        |

use std::fmt;

use serde::{Deserialize, Serialize};
use ulid::Ulid;

/// Unique id of a running or completed agent.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AgentId(pub String);

impl AgentId {
    pub fn new() -> Self {
        Self(Ulid::new().to_string())
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for AgentId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for AgentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Unique id of a message in a transcript / sidechain.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MessageId(pub String);

impl MessageId {
    pub fn new() -> Self {
        Self(Ulid::new().to_string())
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for MessageId {
    fn default() -> Self {
        Self::new()
    }
}

/// Author of a message in the agent transcript.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageRole {
    System,
    User,
    Assistant,
    Tool,
    /// Hidden user-meta message (e.g. `<task-notification>`).
    Meta,
}

/// Where the current `query()` call originated from. Mirrors the TS
/// `querySource` discriminator used by Session Memory, autoCompact,
/// hooks and the SDK boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuerySource {
    /// Primary REPL / main thread.
    ReplMainThread,
    /// Inside a sub-agent loop (general-purpose, Explore, Plan, ...).
    Subagent,
    /// Forked sub-agent that inherits parent context.
    Forked,
    /// In-process teammate runner.
    Teammate,
    /// Special source used by `extractSessionMemory()`.
    SessionMemory,
    /// Internal compact summarisation call.
    Compact,
    /// SDK / programmatic embedding.
    Sdk,
}

/// How an agent should be executed relative to the parent loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentRunMode {
    /// Block the parent tool_use until the child completes.
    Foreground,
    /// Detach and report back via task-notification.
    Background,
    /// Fork the parent loop in-place (background, exact tool pool).
    Fork,
}

/// One event emitted by the model / executor inside the query loop.
///
/// Roughly equivalent to the TS `MessageEvent` union.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MessageEvent {
    Assistant {
        id: MessageId,
        text: String,
    },
    User {
        id: MessageId,
        text: String,
        role: MessageRole,
    },
    ToolUse {
        id: MessageId,
        tool: String,
        input: serde_json::Value,
    },
    ToolResult {
        id: MessageId,
        tool: String,
        output: serde_json::Value,
        is_error: bool,
    },
    Progress {
        id: MessageId,
        note: String,
    },
    /// Marker inserted by compact/sessionMemoryCompact.
    CompactBoundary {
        id: MessageId,
        summary_message_id: MessageId,
    },
}

/// Token accounting for one query / agent run.
///
/// We re-export [`crate::ai::tokens::TokenUsage`] so that the agent layer
/// speaks the same usage type as the provider layer — no per-layer
/// conversions, no duplicated fields.
pub use crate::ai::tokens::TokenUsage;
