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
    "CreateDoc",
    "Delete",
];

/// Bash-prefix patterns that imply the command will mutate state.
///
/// **No longer a security boundary.** Prefix matching against the raw command
/// string is trivially bypassed (`ls && rm -rf .`, `dir > out.txt`,
/// `powershell -c ...`), and the list is Unix-only while this app also ships on
/// Windows (`del`, `rd`, `copy`, `move`, `md`). Plan-mode now refuses `Bash`
/// outright; this list is kept only as a human-readable hint for diagnostics.
pub const BASH_WRITE_PREFIXES: &[&str] = &[
    "mkdir ", "touch ", "rm ", "cp ", "mv ", "git add", "git commit",
    "git push", "git reset", "npm install", "pip install", "cargo add",
    "cargo install", "echo ",
];

/// True when `name` is one of the workspace-mutating [`WRITE_TOOLS`].
pub fn is_write_tool(name: &str) -> bool {
    WRITE_TOOLS.iter().any(|w| w.eq_ignore_ascii_case(name))
}

/// True when `name` is the shell tool.
pub fn is_shell_tool(name: &str) -> bool {
    name.eq_ignore_ascii_case("Bash")
}

/// Split a compound shell line into the individual commands it runs.
///
/// Prefix checks on the whole line are meaningless because every shell chains
/// commands (`a && b`, `a; b`, `a | b`). Splitting on those separators lets a
/// check inspect each command's own leading token. Quoting is deliberately
/// *not* honoured: treating a quoted separator as a real one over-splits, which
/// only ever makes the safety check see more candidate commands, never fewer.
pub fn split_command_segments(command: &str) -> Vec<String> {
    let mut segments = Vec::new();
    let mut current = String::new();
    let mut chars = command.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '&' | '|' => {
                // Collapse `&&` / `||` into a single break.
                if chars.peek() == Some(&c) {
                    chars.next();
                }
                segments.push(std::mem::take(&mut current));
            }
            ';' | '\n' | '\r' => segments.push(std::mem::take(&mut current)),
            _ => current.push(c),
        }
    }
    segments.push(current);
    segments
        .into_iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// Normalised leading token of one command segment (lowercased, unquoted, and
/// stripped of any directory part so `/bin/rm` and `C:\Windows\System32\del`
/// compare equal to `rm` / `del`).
fn command_head(segment: &str) -> String {
    let raw = segment
        .split_whitespace()
        .next()
        .unwrap_or("")
        .trim_matches(|c| c == '"' || c == '\'');
    let base = raw
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(raw)
        .to_ascii_lowercase();
    // `del.exe` / `rm.exe` invoke the same thing as `del` / `rm`.
    base.strip_suffix(".exe").unwrap_or(&base).to_string()
}

/// True when a shell command contains an unambiguously catastrophic,
/// non-recoverable operation (wiping a filesystem root, formatting a volume,
/// destroying backup shadow copies, a fork bomb).
///
/// This is a *safety net*, not a sandbox: containment comes from forcing the
/// shell's working directory inside the project root. It exists because the
/// operations listed here cannot be undone by the snapshot/rollback system, so
/// a model mistake is unrecoverable rather than merely annoying.
pub fn command_is_catastrophic(command: &str) -> bool {
    // A fork bomb has no useful leading token to inspect.
    let squeezed: String = command.chars().filter(|c| !c.is_whitespace()).collect();
    if squeezed.contains(":(){:|:&};:") {
        return true;
    }

    for segment in split_command_segments(command) {
        let tokens: Vec<&str> = segment.split_whitespace().collect();
        if tokens.is_empty() {
            continue;
        }
        if verb_is_catastrophic(&tokens) {
            return true;
        }
        // A privilege / exec wrapper hides the real verb further along the
        // line (`sudo rm -rf /`, `sudo -u root rm -rf /`, `env X=1 rm -rf /`).
        // Only look past the head in that case: scanning every position of
        // every command would also flag harmless text that merely *mentions* a
        // destructive command, e.g. `git commit -m "rm -rf / cleanup"`.
        if COMMAND_WRAPPERS.contains(&command_head(tokens[0]).as_str()) {
            for start in 1..tokens.len() {
                if verb_is_catastrophic(&tokens[start..]) {
                    return true;
                }
            }
        }
    }
    false
}

/// Shell wrappers that execute another command, so the verb worth inspecting is
/// a later token rather than the head.
const COMMAND_WRAPPERS: &[&str] = &[
    "sudo", "doas", "runas", "nohup", "command", "nice", "ionice", "time",
    "timeout", "env", "setsid", "stdbuf", "xargs",
];

/// True when `tokens[0]` is a destructive verb applied to a target that makes
/// the operation unrecoverable.
fn verb_is_catastrophic(tokens: &[&str]) -> bool {
    let Some(raw_head) = tokens.first() else {
        return false;
    };
    let head = command_head(raw_head);
    let args: Vec<String> = tokens[1..]
        .iter()
        .map(|a| a.to_ascii_lowercase())
        .collect();
    let any = |pred: &dyn Fn(&str) -> bool| args.iter().any(|a| pred(a.as_str()));

    match head.as_str() {
        // Filesystem / partition table destruction.
        "mkfs" | "diskpart" | "fdisk" => true,
        h if h.starts_with("mkfs.") => true,
        // `format C:` wipes a volume; bare `format` is harmless.
        "format" => any(&targets_volume_root),
        // Backup / shadow-copy destruction defeats system restore.
        "vssadmin" | "wbadmin" => any(&|a| a == "delete"),
        "cipher" => any(&|a| a == "/w" || a.starts_with("/w:")),
        // Raw block-device writes.
        "dd" => any(&|a| a.starts_with("of=/dev/") && a != "of=/dev/null"),
        // Recursive delete aimed at a filesystem root or the home directory.
        "rm" => {
            let recursive = args.iter().any(|a| {
                (a.starts_with('-') && !a.starts_with("--") && a.contains('r'))
                    || a == "--recursive"
            });
            recursive && any(&targets_filesystem_root)
        }
        "del" | "erase" | "rd" | "rmdir" => any(&targets_volume_root),
        _ => false,
    }
}

/// True when `arg` names a Unix filesystem root or the user's home directory
/// rather than a path inside the project.
fn targets_filesystem_root(arg: &str) -> bool {
    let a = arg.trim_matches(|c| c == '"' || c == '\'');
    matches!(a, "/" | "/*" | "~" | "~/" | "~/*" | "$HOME" | "$HOME/*")
        // `/etc`, `/usr`, ... — a single top-level component under root.
        || (a.starts_with('/')
            && a.trim_end_matches('*').trim_end_matches('/').matches('/').count() == 1
            && a.len() > 1)
        || targets_volume_root(a)
}

/// True when `arg` names a Windows volume root (`C:`, `C:\`, `C:\*`, `\`).
fn targets_volume_root(arg: &str) -> bool {
    let a = arg.trim_matches(|c| c == '"' || c == '\'');
    let stripped = a.trim_end_matches('*').trim_end_matches(['\\', '/']);
    if stripped.len() == 2 {
        let bytes = stripped.as_bytes();
        if bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
            return true;
        }
    }
    matches!(a, "\\" | "\\*" | "/" | "/*")
}

/// Production resolver for the non-Plan modes.
///
/// The app's whole purpose is letting the agent write documents, so in-project
/// writes are allowed without a prompt (the tools themselves confine every path
/// to the project root, and every mutation is snapshotted for rollback). What
/// this resolver *does* refuse is the small class of shell commands that the
/// snapshot system cannot undo — see [`command_is_catastrophic`].
#[derive(Debug, Default, Clone, Copy)]
pub struct DefaultModeResolver;

impl PermissionResolver for DefaultModeResolver {
    fn resolve(&self, request: PermissionRequest<'_>) -> AppResult<PermissionDecision> {
        if matches!(request.mode, PermissionMode::BypassPermissions) {
            return Ok(PermissionDecision::allow());
        }
        if is_shell_tool(request.tool_name) {
            let cmd = request
                .input
                .get("command")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("");
            if command_is_catastrophic(cmd) {
                return Ok(PermissionDecision::deny(
                    "this command performs an irreversible destruction (filesystem root \
                     delete, volume format, shadow-copy purge or raw device write) that the \
                     snapshot system cannot roll back; it is refused in every permission mode \
                     except `bypass_permissions`",
                ));
            }
        }
        Ok(PermissionDecision::allow())
    }
}

/// Wrapper resolver that enforces [`PermissionMode::Plan`] regardless of
/// what the inner resolver returns. Behaviour:
///
/// - `BypassPermissions` ⇒ short-circuit allow.
/// - `Plan` + a name in [`WRITE_TOOLS`] ⇒ deny with a structured reason.
/// - `Plan` + `Bash` ⇒ deny. Plan-mode is read-only exploration and already
///   has `Read` / `Grep` / `ListFiles`; a shell is arbitrary code execution
///   that no static inspection of the command string can make safe.
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
        // Bypass is the documented escape hatch: short-circuit rather than
        // delegate, so an inner resolver can never veto it.
        if matches!(request.mode, PermissionMode::BypassPermissions) {
            return Ok(PermissionDecision::allow());
        }

        if matches!(request.mode, PermissionMode::Plan) {
            // 1) Direct write tool.
            if is_write_tool(request.tool_name) {
                return Ok(PermissionDecision::deny(format!(
                    "{} is blocked in plan-mode; only read-only exploration is allowed",
                    request.tool_name
                )));
            }
            // 2) Any shell at all. Deciding whether a command string mutates
            //    state is undecidable in practice — the previous prefix
            //    blacklist was bypassed by `ls && rm -rf .`, `dir > out.txt`,
            //    `powershell -c ...`, and by every Windows verb (`del`, `rd`,
            //    `copy`, `move`) that the Unix-shaped list never mentioned.
            if is_shell_tool(request.tool_name) {
                return Ok(PermissionDecision::deny(
                    "Bash is blocked in plan-mode; a shell is arbitrary code execution and \
                     cannot be statically proven read-only. Use Read / Grep / ListFiles to \
                     explore, and propose shell steps in the plan instead of running them",
                ));
            }
        }

        self.inner.resolve(request)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decide(mode: PermissionMode, tool: &str, command: &str) -> PermissionDecision {
        let agent_id = AgentId::new();
        let input = serde_json::json!({ "command": command });
        PlanModeResolver::new(DefaultModeResolver)
            .resolve(PermissionRequest {
                agent_id: &agent_id,
                tool_name: tool,
                input: &input,
                mode,
                is_async: false,
                is_coordinator_worker: false,
            })
            .expect("resolver is infallible")
    }

    fn denied(mode: PermissionMode, tool: &str, command: &str) -> bool {
        matches!(decide(mode, tool, command), PermissionDecision::Deny { .. })
    }

    /// The old prefix blacklist matched the *start of the whole line*, so every
    /// one of these reached the shell in plan-mode.
    #[test]
    fn plan_mode_refuses_shell_regardless_of_how_the_write_is_hidden() {
        for cmd in [
            "ls && rm -rf .",
            "cd x && rm -rf y",
            "dir > out.txt",
            "type nul > file",
            "echo.>file",
            "powershell -c \"Remove-Item x\"",
            "sudo rm -rf project",
            "del /f /s /q build",
            "rd /s /q build",
            "python -c \"open('f','w')\"",
            "git  add .",
            "ls",
        ] {
            assert!(
                denied(PermissionMode::Plan, "Bash", cmd),
                "plan-mode must refuse the shell for `{cmd}`"
            );
        }
    }

    #[test]
    fn plan_mode_refuses_write_tools_case_insensitively() {
        for tool in ["Write", "edit", "CreateDoc", "DELETE", "Patch"] {
            assert!(denied(PermissionMode::Plan, tool, ""));
        }
    }

    #[test]
    fn plan_mode_still_allows_read_only_tools() {
        for tool in ["Read", "Grep", "ListFiles", "WebFetch"] {
            assert!(!denied(PermissionMode::Plan, tool, ""));
        }
    }

    /// A writing app has to let the agent write; only unrecoverable shell
    /// destruction is refused outside plan-mode.
    #[test]
    fn default_mode_allows_in_project_work() {
        for tool in ["Write", "Edit", "CreateDoc", "Delete", "Read"] {
            assert!(!denied(PermissionMode::Default, tool, ""));
        }
        for cmd in [
            "git status",
            "cargo check",
            "npm run build",
            "rm -rf build",
            "rm -rf ./target",
            "del /f /s /q build",
            "dir",
            // Mentioning a destructive command in prose must not trip the net.
            "git commit -m \"rm -rf / cleanup\"",
        ] {
            assert!(
                !denied(PermissionMode::Default, "Bash", cmd),
                "`{cmd}` is recoverable and must stay allowed"
            );
        }
    }

    #[test]
    fn catastrophic_commands_are_refused_in_every_mode_but_bypass() {
        let fatal = [
            "rm -rf /",
            "rm -rf /*",
            "rm -rf ~",
            "rm -rf /usr",
            "sudo rm -rf / --no-preserve-root",
            "sudo -u root rm -rf /",
            "env FOO=1 rm -rf /",
            "ls && rm -rf /",
            "format C:",
            "format c:\\ /q",
            "del /f /s /q C:\\",
            "rd /s /q C:\\",
            "mkfs.ext4 /dev/sda1",
            "dd if=/dev/zero of=/dev/sda",
            "vssadmin delete shadows /all",
            "wbadmin delete catalog",
            "cipher /w:C",
            ":(){ :|:& };:",
            "/bin/rm -rf /",
            "C:\\Windows\\System32\\del.exe /f /s /q C:\\",
        ];
        for cmd in fatal {
            for mode in [
                PermissionMode::Default,
                PermissionMode::AcceptEdits,
                PermissionMode::Ask,
                PermissionMode::Plan,
            ] {
                assert!(
                    denied(mode, "Bash", cmd),
                    "`{cmd}` must be refused in {mode:?}"
                );
            }
            assert!(
                !denied(PermissionMode::BypassPermissions, "Bash", cmd),
                "bypass_permissions is the documented escape hatch for `{cmd}`"
            );
        }
    }

    #[test]
    fn bypass_short_circuits_even_for_write_tools() {
        for tool in ["Write", "Delete", "Bash"] {
            assert!(!denied(PermissionMode::BypassPermissions, tool, "rm -rf /"));
        }
    }

    #[test]
    fn compound_lines_split_into_their_own_commands() {
        assert_eq!(
            split_command_segments("ls && rm -rf . ; echo done | cat"),
            vec!["ls", "rm -rf .", "echo done", "cat"]
        );
        assert_eq!(split_command_segments("   "), Vec::<String>::new());
        assert_eq!(split_command_segments("a\nb\r\nc"), vec!["a", "b", "c"]);
    }

    #[test]
    fn volume_root_detection_does_not_swallow_real_paths() {
        assert!(targets_volume_root("C:"));
        assert!(targets_volume_root("c:\\"));
        assert!(targets_volume_root("D:\\*"));
        assert!(!targets_volume_root("C:\\project"));
        assert!(!targets_volume_root("build"));
        assert!(targets_filesystem_root("/"));
        assert!(targets_filesystem_root("/etc"));
        assert!(targets_filesystem_root("~"));
        assert!(!targets_filesystem_root("/home/u/project/build"));
        assert!(!targets_filesystem_root("./build"));
    }
}
