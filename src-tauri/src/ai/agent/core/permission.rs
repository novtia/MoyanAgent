//! Permission model.
//!
//! Mirrors the TS-side permission machinery:
//!
//! - [`PermissionMode`]: the four classic modes (`Default`, `AcceptEdits`,
//!   `Plan`, `BypassPermissions`) plus an `Ask` variant for explicit prompts.
//! - [`PermissionDecision`]: the outcome of the resolver chain.
//! - [`PermissionResolver`]: pluggable strategy used by the executor.
//!
//! The actual resolution chain documented in `agent-architecture.md` §9 is:
//!
//! ```text
//!   zod validate → tool.validateInput → PreToolUse hooks → canUseTool
//!                                                         → tool.call
//!                                                         → PostToolUse hooks
//! ```
//!
//! In Rust we keep the same ordering but inside [`crate::ai::agent::tools::ToolPool::execute`].

use serde::{Deserialize, Serialize};

use crate::ai::agent::types::AgentId;
use crate::error::AppResult;

/// Permission mode for a single tool invocation context.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionMode {
    /// Normal interactive mode.
    Default,
    /// Plan-mode: read-only, plan submission only.
    Plan,
    /// Auto-accept edits inside the working tree.
    AcceptEdits,
    /// Skip the permission dialog entirely.
    BypassPermissions,
    /// Always ask, even when a previous decision exists.
    Ask,
}

impl Default for PermissionMode {
    fn default() -> Self {
        PermissionMode::Default
    }
}

/// Outcome of the resolver chain. Matches `permissionResolver`'s
/// `{allow, deny, ask}` decisions in the TS code.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "decision", rename_all = "snake_case")]
pub enum PermissionDecision {
    Allow {
        /// Optional reason recorded in telemetry.
        reason: Option<String>,
    },
    Deny {
        reason: String,
    },
    /// Defer to interactive prompt. The executor must surface a UI prompt
    /// before retrying the resolver.
    Ask {
        reason: Option<String>,
    },
}

impl PermissionDecision {
    pub fn allow() -> Self {
        PermissionDecision::Allow { reason: None }
    }

    pub fn deny(reason: impl Into<String>) -> Self {
        PermissionDecision::Deny {
            reason: reason.into(),
        }
    }

    pub fn is_allowed(&self) -> bool {
        matches!(self, PermissionDecision::Allow { .. })
    }
}

/// A request that the resolver chain inspects before a tool runs.
#[derive(Debug, Clone)]
pub struct PermissionRequest<'a> {
    pub agent_id: &'a AgentId,
    pub tool_name: &'a str,
    pub input: &'a serde_json::Value,
    pub mode: PermissionMode,
    /// True for sub-agents launched in the background.
    pub is_async: bool,
    /// True when running under coordinator mode.
    pub is_coordinator_worker: bool,
}

/// Strategy used by [`crate::ai::agent::tools::ToolPool::execute`] to decide
/// whether a tool call is allowed.
///
/// Implementations should be deterministic given the same input + mode,
/// and **must not** block on UI for the `is_async == true` path — those
/// callers should resolve to `Deny` or `Allow` without prompting.
pub trait PermissionResolver: Send + Sync {
    fn resolve(&self, request: PermissionRequest<'_>) -> AppResult<PermissionDecision>;
}

/// Allow-by-default resolver. Useful as a placeholder while wiring things up.
#[derive(Debug, Default, Clone, Copy)]
pub struct AllowAllResolver;

impl PermissionResolver for AllowAllResolver {
    fn resolve(&self, _request: PermissionRequest<'_>) -> AppResult<PermissionDecision> {
        Ok(PermissionDecision::allow())
    }
}

/// Tool names that mutate the workspace. Used by [`PlanModeResolver`]
/// and by `AgentDefinition::disallowed_tools` defaults.
///
/// Kept as a single const slice so adding a new write-tool (e.g.
/// `NotebookEdit`, `Patch`) only touches one place.
pub const WRITE_TOOLS: &[&str] = &[
    "Write",
    "Edit",
    "MultiEdit",
    "NotebookEdit",
    "Patch",
    "FileWrite",
    "FileEdit",
];

/// Bash-prefix patterns that imply the command will mutate state. Used
/// when [`PlanModeResolver`] sees a `Bash` invocation in Plan-mode.
pub const BASH_WRITE_PREFIXES: &[&str] = &[
    "mkdir ", "touch ", "rm ", "cp ", "mv ", "git add", "git commit",
    "git push", "git reset", "npm install", "pip install", "cargo add",
    "cargo install", "echo ",
];

/// Wrapper resolver that enforces [`PermissionMode::Plan`] regardless of
/// what the inner resolver returns. Behaviour:
///
/// - `Plan` + a name in [`WRITE_TOOLS`] ⇒ deny with a structured reason.
/// - `Plan` + `Bash` whose `command` starts with a write-prefix ⇒ deny.
/// - `BypassPermissions` ⇒ short-circuit allow.
/// - Everything else ⇒ delegate.
///
/// Mirrors the upstream `assertSafeForPlanMode` check that runs *before*
/// the tool-specific permission prompt.
pub struct PlanModeResolver<R: PermissionResolver> {
    inner: R,
}

impl<R: PermissionResolver> PlanModeResolver<R> {
    pub fn new(inner: R) -> Self {
        Self { inner }
    }
}

impl<R: PermissionResolver> PermissionResolver for PlanModeResolver<R> {
    fn resolve(&self, request: PermissionRequest<'_>) -> AppResult<PermissionDecision> {
        if matches!(request.mode, PermissionMode::BypassPermissions) {
            return self.inner.resolve(request);
        }

        if matches!(request.mode, PermissionMode::Plan) {
            // 1) Direct write tool.
            if WRITE_TOOLS.iter().any(|w| w.eq_ignore_ascii_case(request.tool_name)) {
                return Ok(PermissionDecision::deny(format!(
                    "{} is blocked in plan-mode; only read-only exploration is allowed",
                    request.tool_name
                )));
            }
            // 2) Bash with a write-shaped command.
            if request.tool_name.eq_ignore_ascii_case("Bash") {
                let cmd = request
                    .input
                    .get("command")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("")
                    .trim_start();
                if BASH_WRITE_PREFIXES
                    .iter()
                    .any(|p| cmd.to_ascii_lowercase().starts_with(p))
                {
                    return Ok(PermissionDecision::deny(format!(
                        "Bash command `{}` looks like a write; plan-mode forbids \
                         workspace mutations",
                        cmd.chars().take(40).collect::<String>()
                    )));
                }
            }
        }

        self.inner.resolve(request)
    }
}
