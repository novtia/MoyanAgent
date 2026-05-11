//! Agent definitions: the frontmatter-shaped configuration loaded from
//! built-in / plugin / settings sources.
//!
//! See `agent-architecture.md` §4 for the source-priority ordering and
//! the canonical field list.

use serde::{Deserialize, Serialize};

use crate::ai::agent::core::permission::PermissionMode;

/// Where an [`AgentDefinition`] came from. Higher numeric variant wins
/// during merge (mirrors `loadAgentsDir.ts` priority ordering).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentSource {
    BuiltIn = 0,
    Plugin = 1,
    User = 2,
    Project = 3,
    Flag = 4,
    Managed = 5,
}

/// Filesystem isolation mode for a sub-agent run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Isolation {
    /// Run inside the current working tree.
    None,
    /// Allocate a temporary `git worktree` and run there.
    Worktree,
    /// Hand the run off to a remote CCR session.
    Remote,
}

impl Default for Isolation {
    fn default() -> Self {
        Isolation::None
    }
}

/// Memory hook attached to a custom agent (`agent-memory/<agentType>/...`).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AgentMemorySettings {
    /// If false, the agent doesn't get its own persistent memory directory.
    pub enabled: bool,
    /// `user` | `project` | `local`, mirrors `AgentMemoryScope` in TS.
    pub scope: Option<String>,
}

/// The canonical agent definition. Field names follow the TS frontmatter
/// surface so YAML can be deserialised directly.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentDefinition {
    /// Stable identifier, e.g. `general-purpose`, `Explore`, `Plan`.
    #[serde(rename = "agentType")]
    pub agent_type: String,

    /// Short description used by the routing classifier.
    #[serde(default, rename = "whenToUse")]
    pub when_to_use: String,

    /// Body of the markdown file → becomes the agent system prompt.
    #[serde(default, rename = "systemPrompt")]
    pub system_prompt: String,

    /// `["*"]` means "all non-denied tools".
    #[serde(default)]
    pub tools: Vec<String>,
    #[serde(default, rename = "disallowedTools")]
    pub disallowed_tools: Vec<String>,

    #[serde(default)]
    pub skills: Vec<String>,
    #[serde(default, rename = "mcpServers")]
    pub mcp_servers: Vec<String>,
    #[serde(default, rename = "requiredMcpServers")]
    pub required_mcp_servers: Vec<String>,
    #[serde(default)]
    pub hooks: Vec<String>,

    /// Override the active model. `None` ⇒ inherit parent.
    #[serde(default)]
    pub model: Option<String>,
    /// Override reasoning effort (low/medium/high/...).
    #[serde(default)]
    pub effort: Option<String>,

    #[serde(default, rename = "permissionMode")]
    pub permission_mode: Option<PermissionMode>,

    #[serde(default, rename = "maxTurns")]
    pub max_turns: Option<u32>,

    /// When true, this agent is always launched in the background.
    #[serde(default)]
    pub background: bool,

    /// Initial prompt inserted ahead of the user prompt.
    #[serde(default, rename = "initialPrompt")]
    pub initial_prompt: Option<String>,

    #[serde(default)]
    pub memory: AgentMemorySettings,

    #[serde(default)]
    pub isolation: Isolation,

    /// Skip CLAUDE.md / user-context injection for this agent.
    #[serde(default, rename = "omitClaudeMd")]
    pub omit_claude_md: bool,

    /// Experimental: extra system reminder appended every turn.
    #[serde(default, rename = "criticalSystemReminder_EXPERIMENTAL")]
    pub critical_system_reminder: Option<String>,

    /// Source this definition came from. Used by the merger.
    #[serde(default = "default_source")]
    pub source: AgentSource,
}

fn default_source() -> AgentSource {
    AgentSource::User
}

impl AgentDefinition {
    /// Convenience constructor for built-ins.
    pub fn builtin(agent_type: impl Into<String>, system_prompt: impl Into<String>) -> Self {
        Self {
            agent_type: agent_type.into(),
            when_to_use: String::new(),
            system_prompt: system_prompt.into(),
            tools: vec!["*".into()],
            disallowed_tools: vec![],
            skills: vec![],
            mcp_servers: vec![],
            required_mcp_servers: vec![],
            hooks: vec![],
            model: None,
            effort: None,
            permission_mode: None,
            max_turns: None,
            background: false,
            initial_prompt: None,
            memory: AgentMemorySettings::default(),
            isolation: Isolation::None,
            omit_claude_md: false,
            critical_system_reminder: None,
            source: AgentSource::BuiltIn,
        }
    }

    /// `true` when all of this agent's `requiredMcpServers` are available.
    pub fn required_mcp_satisfied(&self, available: &[String]) -> bool {
        self.required_mcp_servers
            .iter()
            .all(|req| available.iter().any(|a| a == req))
    }
}
