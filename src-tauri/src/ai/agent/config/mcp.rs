//! Minimal MCP registry abstraction.
//!
//! `AgentTool` uses an [`McpRegistry`] to ask which MCP servers are
//! currently available, so that agents whose `requiredMcpServers` cannot
//! be satisfied are hidden (see
//! [`crate::ai::agent::config::registry::AgentRegistry::filter_by_mcp`]).
//!
//! The real MCP client/management code is intentionally out of scope for
//! the agent skeleton. Today we ship a static, in-memory snapshot
//! ([`StaticMcpRegistry`]) so the wiring is honest end-to-end; future
//! work can introduce a dynamic registry that watches the user's MCP
//! configuration and notifies the agent layer on changes.

use std::sync::Mutex;

/// Read-only view of currently available MCP servers, identified by name.
pub trait McpRegistry: Send + Sync {
    fn available_servers(&self) -> Vec<String>;
}

/// Static snapshot — useful as a default for environments that do not
/// (yet) ship an MCP runtime. The list is mutable behind a `Mutex` so
/// host code can swap it after configuration changes without forcing
/// `AgentTool` to be reconstructed.
#[derive(Default)]
pub struct StaticMcpRegistry {
    servers: Mutex<Vec<String>>,
}

impl StaticMcpRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_servers<I, S>(servers: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            servers: Mutex::new(servers.into_iter().map(Into::into).collect()),
        }
    }

    pub fn set(&self, servers: Vec<String>) {
        if let Ok(mut g) = self.servers.lock() {
            *g = servers;
        }
    }
}

impl McpRegistry for StaticMcpRegistry {
    fn available_servers(&self) -> Vec<String> {
        self.servers.lock().ok().map(|g| g.clone()).unwrap_or_default()
    }
}
