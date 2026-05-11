//! Agent definition registry.
//!
//! Loads definitions from the six layered sources (`agent-architecture.md`
//! §4) and exposes a merged, priority-resolved view. Higher-priority sources
//! overwrite lower-priority ones at the same `agentType`.

use std::collections::HashMap;

use crate::ai::agent::config::builtin::builtin_definitions;
use crate::ai::agent::config::definition::{AgentDefinition, AgentSource};

#[derive(Default)]
pub struct AgentRegistry {
    /// Ordered by source ascending. Each entry shadows the previous one
    /// for the same `agentType`.
    layers: Vec<(AgentSource, Vec<AgentDefinition>)>,
}

impl AgentRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Construct a registry pre-populated with the built-in agents.
    pub fn with_builtins() -> Self {
        let mut r = Self::new();
        r.push(AgentSource::BuiltIn, builtin_definitions());
        r
    }

    pub fn push(&mut self, source: AgentSource, defs: Vec<AgentDefinition>) {
        self.layers.push((source, defs));
        self.layers.sort_by_key(|(s, _)| *s);
    }

    /// Resolve the active set of agents, honoring source priority.
    pub fn active(&self) -> HashMap<String, AgentDefinition> {
        let mut out: HashMap<String, AgentDefinition> = HashMap::new();
        for (_src, defs) in &self.layers {
            for d in defs {
                out.insert(d.agent_type.clone(), d.clone());
            }
        }
        out
    }

    pub fn get(&self, agent_type: &str) -> Option<AgentDefinition> {
        self.active().remove(agent_type)
    }

    /// Hide agents whose `requiredMcpServers` are not satisfied by the
    /// `available_mcp_servers` slice.
    pub fn filter_by_mcp(
        &self,
        available_mcp_servers: &[String],
    ) -> HashMap<String, AgentDefinition> {
        self.active()
            .into_iter()
            .filter(|(_, d)| d.required_mcp_satisfied(available_mcp_servers))
            .collect()
    }
}
