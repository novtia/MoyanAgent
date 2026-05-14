//! Built-in agent definitions.
//!
//! These mirror the agents listed in `agent-architecture.md` §5 and the
//! upstream definitions in `claude-code/tools/AgentTool/built-in/*.ts`.
//! The system-prompt bodies live in [`super::prompts`] so they can be
//! reviewed / diffed independently of the wiring code here.
//!
//! - `general-purpose` — multi-step research/execution, full tool access.
//! - `Explore`         — read-only investigation agent (fast).
//! - `Plan`            — read-only planning agent (architect).
//! - `claude-code-guide` — in-app guide; answers questions about this codebase.
//! - `verification`    — background adversarial verifier (read-only project files).
//! - `fork`            — synthetic; inherits parent prompt + tools.

use crate::ai::agent::config::definition::{AgentDefinition, AgentSource};
use crate::ai::agent::config::prompts;
use crate::ai::agent::core::permission::PermissionMode;
use crate::ai::agent::tools::agent_tool::AGENT_TOOL_NAME;

pub const AGENT_GENERAL_PURPOSE: &str = "general-purpose";
pub const AGENT_EXPLORE: &str = "Explore";
pub const AGENT_PLAN: &str = "Plan";
pub const AGENT_GUIDE: &str = "claude-code-guide";
pub const AGENT_VERIFICATION: &str = "verification";
pub const AGENT_FORK: &str = "fork";

/// Tool names that are unsafe for read-only agents (Explore / Plan /
/// Verification). Kept in one place so adding a write-tool only needs a
/// single edit.
const WRITE_TOOLS: &[&str] = &[
    "Edit",
    "Write",
    "NotebookEdit",
    "ExitPlanMode",
    "TodoList",
];

fn read_only_deny() -> Vec<String> {
    let mut v: Vec<String> = WRITE_TOOLS.iter().map(|s| s.to_string()).collect();
    v.push(AGENT_TOOL_NAME.to_string());
    v
}

/// Return the static list of built-in agents bundled with the binary.
///
/// At load time these go through [`crate::ai::agent::config::registry::AgentRegistry`]
/// just like external definitions, so the source-priority merge rules
/// still apply.
pub fn builtin_definitions() -> Vec<AgentDefinition> {
    vec![
        general_purpose(),
        explore(),
        plan(),
        guide(),
        verification(),
        fork(),
    ]
}

fn general_purpose() -> AgentDefinition {
    let mut d = AgentDefinition::builtin(AGENT_GENERAL_PURPOSE, prompts::GENERAL_PURPOSE_PROMPT);
    d.when_to_use = prompts::GENERAL_PURPOSE_WHEN_TO_USE.into();
    d.tools = vec!["*".into()];
    d
}

fn explore() -> AgentDefinition {
    let mut d = AgentDefinition::builtin(AGENT_EXPLORE, prompts::EXPLORE_PROMPT);
    d.when_to_use = prompts::EXPLORE_WHEN_TO_USE.into();
    d.tools = vec!["*".into()];
    d.disallowed_tools = read_only_deny();
    // Explore is a fast read-only search agent — it doesn't need
    // commit/PR/lint rules from CLAUDE.md. The parent agent has the full
    // context and interprets the report.
    d.omit_claude_md = true;
    d
}

fn plan() -> AgentDefinition {
    let mut d = AgentDefinition::builtin(AGENT_PLAN, prompts::PLAN_PROMPT);
    d.when_to_use = prompts::PLAN_WHEN_TO_USE.into();
    d.tools = vec!["*".into()];
    d.disallowed_tools = read_only_deny();
    // Two layers of protection that complement each other:
    //   1. `disallowed_tools`: removes write tools from the pool, so
    //      they don't even show up in the model's tool list.
    //   2. `permission_mode = Plan`: tells `PlanModeResolver` to deny
    //      *any* write attempt that slips through (e.g. a custom user
    //      tool the deny-list didn't anticipate, or a write-shaped Bash
    //      command).
    d.permission_mode = Some(PermissionMode::Plan);
    // Plan is read-only and can FileRead CLAUDE.md directly if it needs
    // conventions. Dropping it from the auto-injected context saves
    // tokens without blocking access.
    d.omit_claude_md = true;
    d
}

fn guide() -> AgentDefinition {
    let mut d = AgentDefinition::builtin(AGENT_GUIDE, prompts::GUIDE_PROMPT);
    d.when_to_use = prompts::GUIDE_WHEN_TO_USE.into();
    d.tools = vec![
        "FileRead".into(),
        "Grep".into(),
        "Glob".into(),
        "WebFetch".into(),
        "WebSearch".into(),
    ];
    d
}

fn verification() -> AgentDefinition {
    let mut d = AgentDefinition::builtin(AGENT_VERIFICATION, prompts::VERIFICATION_PROMPT);
    d.when_to_use = prompts::VERIFICATION_WHEN_TO_USE.into();
    d.tools = vec![
        "FileRead".into(),
        "Grep".into(),
        "Glob".into(),
        "Bash".into(),
    ];
    d.background = true;
    d.disallowed_tools = read_only_deny();
    d.critical_system_reminder = Some(prompts::VERIFICATION_CRITICAL_REMINDER.into());
    d
}

fn fork() -> AgentDefinition {
    // Synthetic definition; usually returned by `forkSubagent` rather
    // than looked up by name. Source is still `BuiltIn` so it cannot be
    // overridden.
    let mut d = AgentDefinition::builtin(AGENT_FORK, prompts::FORK_PROMPT);
    d.when_to_use = prompts::FORK_WHEN_TO_USE.into();
    d.background = true;
    d.tools = vec!["*".into()];
    d.source = AgentSource::BuiltIn;
    d
}
