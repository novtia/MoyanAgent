//! Agent definitions and discovery.
//!
//! - [`definition`]   `AgentDefinition` + `AgentSource` priority
//! - [`builtin`]      hard-coded built-in agent prompts
//! - [`registry`]     load / merge / filter agent definitions
//! - [`mcp`]          MCP-server registry that gates `requiredMcpServers`
//!
//! Everything here is **static configuration** — no runtime state, no
//! provider IO. The registry is consulted by `AgentTool` to resolve a
//! `subagent_type` into a concrete `AgentDefinition`.

pub mod builtin;
pub mod definition;
pub mod mcp;
pub mod prompts;
pub mod registry;
