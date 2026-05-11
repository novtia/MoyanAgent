//! Shell-execution tool.
//!
//! Cross-platform — picks `cmd /C` / `powershell` on Windows and `sh -c`
//! on Unix. Honors a per-call timeout (defaults to 60s) and caps the
//! returned stdout/stderr so the model can't OOM itself on a noisy
//! command.
//!
//! Safety is enforced upstream:
//!
//! - [`crate::ai::agent::core::permission::PlanModeResolver`] denies
//!   any Bash call that starts with a mutating prefix
//!   (`BASH_WRITE_PREFIXES`) when the active agent is in
//!   [`crate::ai::agent::core::permission::PermissionMode::Plan`].
//! - The host's own [`crate::ai::agent::core::permission::PermissionResolver`]
//!   gets the full command on every invocation and can prompt / deny.

use std::process::Stdio;
use std::time::Duration;

use serde_json::{json, Value};
use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tokio::time::timeout;

use crate::ai::agent::tools::{Tool, ToolFuture, ToolInvocation, ToolResult, ToolSpec};
use crate::error::{AppError, AppResult};

const TOOL_NAME: &str = "Bash";
const DEFAULT_TIMEOUT_SECS: u64 = 60;
const MAX_TIMEOUT_SECS: u64 = 10 * 60;
const MAX_OUTPUT_BYTES: usize = 64 * 1024;

#[derive(Clone)]
pub struct BashTool {
    spec: ToolSpec,
}

impl Default for BashTool {
    fn default() -> Self {
        Self::new()
    }
}

impl BashTool {
    pub fn new() -> Self {
        Self {
            spec: ToolSpec {
                name: TOOL_NAME.to_string(),
                description: "Execute a shell command and return stdout/stderr/exit code. \
                    Uses cmd on Windows and sh on Unix. Times out (default 60s)."
                    .to_string(),
                schema: json!({
                    "type": "object",
                    "properties": {
                        "command": {
                            "type": "string",
                            "description": "Shell command to execute. Treated as a single -c / /C string."
                        },
                        "timeout_secs": {
                            "type": "integer",
                            "minimum": 1,
                            "maximum": MAX_TIMEOUT_SECS,
                            "description": "Hard timeout. Defaults to 60s, max 600s."
                        },
                        "cwd": {
                            "type": "string",
                            "description": "Working directory. Defaults to the agent's CWD."
                        }
                    },
                    "required": ["command"]
                }),
                read_only: false,
                concurrency_safe: false,
            },
        }
    }
}

impl Tool for BashTool {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    fn validate(&self, input: &Value) -> AppResult<()> {
        let command = input
            .get("command")
            .and_then(Value::as_str)
            .ok_or_else(|| AppError::Invalid("Bash: `command` must be a string".into()))?;
        if command.trim().is_empty() {
            return Err(AppError::Invalid("Bash: `command` must be non-empty".into()));
        }
        Ok(())
    }

    fn execute<'a>(&'a self, invocation: ToolInvocation<'a>) -> ToolFuture<'a> {
        Box::pin(async move {
            let command = invocation
                .input
                .get("command")
                .and_then(Value::as_str)
                .ok_or_else(|| AppError::Invalid("Bash: missing command".into()))?
                .to_string();
            let timeout_secs = invocation
                .input
                .get("timeout_secs")
                .and_then(Value::as_u64)
                .unwrap_or(DEFAULT_TIMEOUT_SECS)
                .min(MAX_TIMEOUT_SECS);
            let cwd = invocation
                .input
                .get("cwd")
                .and_then(Value::as_str)
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|| invocation.context.cwd.clone());

            let mut cmd = build_command(&command);
            if !cwd.as_os_str().is_empty() {
                cmd.current_dir(&cwd);
            }
            cmd.stdout(Stdio::piped());
            cmd.stderr(Stdio::piped());
            cmd.stdin(Stdio::null());

            let mut child = cmd
                .spawn()
                .map_err(|e| AppError::Other(format!("Bash: spawn failed: {e}")))?;

            let mut stdout_buf = Vec::with_capacity(8 * 1024);
            let mut stderr_buf = Vec::with_capacity(8 * 1024);
            let mut stdout_pipe = child.stdout.take();
            let mut stderr_pipe = child.stderr.take();

            let exec = async {
                if let Some(mut p) = stdout_pipe.take() {
                    let _ = p.read_to_end(&mut stdout_buf).await;
                }
                if let Some(mut p) = stderr_pipe.take() {
                    let _ = p.read_to_end(&mut stderr_buf).await;
                }
                child.wait().await
            };

            let status = match timeout(Duration::from_secs(timeout_secs), exec).await {
                Ok(res) => res.map_err(|e| AppError::Other(format!("Bash: wait failed: {e}")))?,
                Err(_) => {
                    return Ok(ToolResult::error(format!(
                        "Bash: command timed out after {timeout_secs}s"
                    )));
                }
            };

            let stdout = truncate_lossy(&stdout_buf);
            let stderr = truncate_lossy(&stderr_buf);

            let mut content = json!({
                "command": command,
                "exit_code": status.code(),
                "stdout": stdout.0,
                "stderr": stderr.0,
            });
            let m = content.as_object_mut().unwrap();
            if stdout.1 {
                m.insert("stdout_truncated".into(), Value::Bool(true));
            }
            if stderr.1 {
                m.insert("stderr_truncated".into(), Value::Bool(true));
            }

            let is_error = !status.success();
            Ok(ToolResult {
                content,
                is_error,
                metadata: None,
            })
        })
    }
}

#[cfg(windows)]
fn build_command(command: &str) -> Command {
    let mut cmd = Command::new("cmd");
    cmd.arg("/C").arg(command);
    cmd
}

#[cfg(not(windows))]
fn build_command(command: &str) -> Command {
    let mut cmd = Command::new("sh");
    cmd.arg("-c").arg(command);
    cmd
}

fn truncate_lossy(bytes: &[u8]) -> (String, bool) {
    if bytes.len() <= MAX_OUTPUT_BYTES {
        return (String::from_utf8_lossy(bytes).into_owned(), false);
    }
    let head = &bytes[..MAX_OUTPUT_BYTES];
    let mut s = String::from_utf8_lossy(head).into_owned();
    s.push_str("\n\n<truncated>");
    (s, true)
}
