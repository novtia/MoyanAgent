//! Agent subsystem.
//!
//! This module mirrors the architecture documented in
//! `claude-code/docs/agent-architecture.md` and `context-memory-architecture.md`,
//! adapted to this project's Tauri + Rust runtime.
//!
//! # Module map
//!
//! ```text
//!   agent/
//!     types.rs       shared primitives (AgentId, MessageEvent, ...)
//!
//!     core/          ── runtime bedrock; no provider IO
//!       permission   permission mode + resolver
//!       context      ToolUseContext — per-agent isolation boundary
//!       task         Task / TaskStore lifecycle
//!       attachment   hidden user-meta messages + NotificationQueue
//!
//!     config/        ── static configuration; no runtime state
//!       definition   AgentDefinition + AgentSource priority
//!       builtin      hard-coded built-in agent prompts
//!       registry     load / merge / filter definitions
//!       mcp          MCP-server availability registry
//!
//!     memory/        ── L2..L5 context-memory layers
//!       (mod.rs)     traits: MemoryFile, UserContext, SessionMemory, ...
//!       user_context CLAUDE.md / rules loader (Fs-backed)
//!       session      per-session summary.md extractor (Fs-backed)
//!       nested       path-scoped rule injection driven by tool reads
//!
//!     tools/         ── tool trait + built-in tools
//!       (mod.rs)     Tool trait, ToolPool, ToolInvocation, ...
//!       fs           FileReadTool
//!       edit         FileWriteTool, FileEditTool
//!       bash         BashTool
//!       agent_tool   the `Agent` meta-tool: spawn sub-agents
//!
//!     exec/          ── execution; the only layer that knows providers
//!       query        QueryEngine trait + request/result types
//!       engine       ProviderEngine + ProviderQueryEngine + run_chat_request
//!       runner       run_agent — drives a child agent end-to-end
//! ```
//!
//! # Call chain
//!
//! ```text
//!   AgentTool        ── entry / router / lifecycle glue (tools/agent_tool)
//!     │
//!     ▼
//!   run_agent        ── builds child run context, drives QueryEngine
//!     │
//!     ▼
//!   ProviderEngine   ── single-turn provider call (chat path)
//!   QueryEngine      ── multi-turn text + tool loop
//!     │
//!     ▼
//!   ToolUseContext   ── isolation boundary per agent
//!   ToolPool         ── filtered, deny-listed tool set
//!
//!   TaskStore        ── pending / running / completed tasks
//!   NotificationQueue── async <task-notification> injection
//! ```
//!
//! Today's runtime entry point is [`exec::engine::run_chat_request`]: a thin
//! wrapper that registers a [`core::task::Task`] in the [`core::task::TaskStore`]
//! and drives a chat request through [`exec::engine::ProviderEngine`]. The
//! richer [`tools::agent_tool::AgentTool::call`] / [`exec::runner::run_agent`]
//! path is fully wired and is invoked when the model emits a `tool_call`
//! against the registered `Agent` tool.
//!
//! The submodules below intentionally publish more surface than the rest
//! of the crate currently consumes — they are the public API for code
//! that will be added in subsequent steps (tool-loop engine, MCP
//! integration, etc.). Silence dead-code warnings at the module boundary
//! rather than scattering attributes across every type.

#![allow(dead_code)]

pub mod config;
pub mod core;
pub mod exec;
pub mod memory;
pub mod tools;
pub mod types;

// These re-exports form the flat, public-facing API of the agent
// module. Only a handful are consumed today — the rest are exposed so
// the rest of the crate (and future Tauri commands) can pull them in
// without reaching deep into submodule paths.
mod re_exports {
    #![allow(unused_imports)]
    // tools
    pub use super::tools::agent_tool::{AgentInvocation, AgentTool, AgentToolResult};
    pub use super::tools::bash::BashTool;
    pub use super::tools::edit::{FileEditTool, FileWriteTool};
    pub use super::tools::fs::FileReadTool;
    pub use super::tools::{Tool, ToolInvocation, ToolPool, ToolResult, ToolSpec};
    // core
    pub use super::core::attachment::{
        Attachment, AttachmentKind, NotificationQueue, TaskNotification,
    };
    pub use super::core::context::{ToolUseContext, ToolUseContextBuilder};
    pub use super::core::permission::{
        AllowAllResolver, BASH_WRITE_PREFIXES, PermissionDecision, PermissionMode,
        PermissionResolver, PlanModeResolver, WRITE_TOOLS,
    };
    pub use super::core::task::{Task, TaskId, TaskKind, TaskState, TaskStore};
    // config
    pub use super::config::definition::{AgentDefinition, AgentSource, Isolation};
    pub use super::config::mcp::{McpRegistry, StaticMcpRegistry};
    pub use super::config::registry::AgentRegistry;
    // memory
    pub use super::memory::nested::{collect_nested_memory, glob_match};
    pub use super::memory::session::{
        DEFAULT_TEMPLATE as SESSION_MEMORY_TEMPLATE, FsSessionMemoryExtractor, SessionMemoryConfig,
    };
    pub use super::memory::user_context::{FsUserContextLoader, UserContextConfig};
    // exec
    pub use super::exec::engine::{
        AgentChatOutcome, EngineTurn, ProviderEngine, ProviderQueryEngine, ToolUseRequest,
        inject_attachments_into_history, run_chat_request,
    };
    pub use super::exec::query::{QueryEngine, QueryRequest, QueryResult};
    pub use super::exec::runner::{RunAgentParams, RunAgentResult, run_agent};
    // shared
    pub use super::types::{
        AgentId, AgentRunMode, MessageEvent, MessageId, MessageRole, QuerySource,
    };
}

pub use re_exports::*;
