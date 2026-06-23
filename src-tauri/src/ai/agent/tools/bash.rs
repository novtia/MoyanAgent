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
                    Platform and shell type are in the `<env>` block — read that instead of \
                    running `uname`. Requires a working directory from the database project \
                    `path` (or an explicit absolute `cwd`). \
                    Uses cmd on Windows and sh on Unix. Times out (default 60s)."
                    .to_string(),
                schema: json!({
                    "type": "object",
                    "properties": {
                        "command": {
                            "type": "string",
                            "description": "Shell command to execute. Treated as a single -c / /C string. \
                                On Windows use cmd syntax (dir, type); on Unix use sh syntax (ls, find)."
                        },
                        "timeout_secs": {
                            "type": "integer",
                            "minimum": 1,
                            "maximum": MAX_TIMEOUT_SECS,
                            "description": "Hard timeout. Defaults to 60s, max 600s."
                        },
                        "cwd": {
                            "type": "string",
                            "description": "Working directory. Must be absolute. On Windows use drive-letter \
                                paths (e.g. C:\\\\project); Unix-style paths like /tmp are invalid on Windows. \
                                Defaults to the database project path from `<env>`."
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

            // Refuse to run without an explicit working directory. Falling
            // through to the host process CWD would leak the app's own
            // directory; the only valid CWD sources are the DB project
            // path (via context) or an explicit absolute `cwd` argument.
            if cwd.as_os_str().is_empty() {
                return Ok(ToolResult::error(
                    "Bash: no working directory available. Set the project's `path` in the \
                     database, or pass an explicit absolute `cwd`. To detect the OS, read \
                     `<env>Platform</env>` in the system prompt — do not run `uname`.",
                ));
            }
            if !cwd.is_absolute() {
                return Ok(ToolResult::error(cwd_validation_error(&cwd)));
            }

            let mut cmd = build_command(&command);
            cmd.current_dir(&cwd);
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

            let stdout = truncate_console(&stdout_buf);
            let stderr = truncate_console(&stderr_buf);

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
    use std::os::windows::process::CommandExt;

    // Don't pop up a console window when the GUI (Tauri) app spawns the child.
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    let mut std_cmd = std::process::Command::new("cmd");
    std_cmd.arg("/C").arg(command);
    std_cmd.creation_flags(CREATE_NO_WINDOW);
    Command::from(std_cmd)
}

#[cfg(not(windows))]
fn build_command(command: &str) -> Command {
    let mut cmd = Command::new("sh");
    cmd.arg("-c").arg(command);
    cmd
}

fn truncate_console(bytes: &[u8]) -> (String, bool) {
    let (slice, truncated) = if bytes.len() <= MAX_OUTPUT_BYTES {
        (bytes, false)
    } else {
        (&bytes[..MAX_OUTPUT_BYTES], true)
    };
    let mut s = decode_console(slice);
    if truncated {
        s.push_str("\n\n<truncated>");
    }
    (s, truncated)
}

/// Decode raw child-process output. On Windows, console programs (e.g. `dir`)
/// emit bytes in the OEM code page (GBK/936 on a Chinese system), so decode
/// directly with it — non-ASCII file names come through correctly instead of
/// as mojibake.
#[cfg(windows)]
fn decode_console(bytes: &[u8]) -> String {
    decode_oem(bytes)
}

#[cfg(not(windows))]
fn decode_console(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// Convert bytes in the system OEM code page to a `String` via the Win32
/// `MultiByteToWideChar` API (`CP_OEMCP`). Falls back to a lossy UTF-8 decode
/// if the conversion fails.
#[cfg(windows)]
fn decode_oem(bytes: &[u8]) -> String {
    if bytes.is_empty() {
        return String::new();
    }

    // CP_OEMCP = 1: use the system OEM code page (the default for console output).
    const CP_OEMCP: u32 = 1;

    extern "system" {
        fn MultiByteToWideChar(
            code_page: u32,
            dw_flags: u32,
            lp_multi_byte_str: *const u8,
            cb_multi_byte: i32,
            lp_wide_char_str: *mut u16,
            cch_wide_char: i32,
        ) -> i32;
    }

    let len = bytes.len() as i32;
    unsafe {
        let needed =
            MultiByteToWideChar(CP_OEMCP, 0, bytes.as_ptr(), len, std::ptr::null_mut(), 0);
        if needed <= 0 {
            return String::from_utf8_lossy(bytes).into_owned();
        }
        let mut buf = vec![0u16; needed as usize];
        let written =
            MultiByteToWideChar(CP_OEMCP, 0, bytes.as_ptr(), len, buf.as_mut_ptr(), needed);
        if written <= 0 {
            return String::from_utf8_lossy(bytes).into_owned();
        }
        String::from_utf16_lossy(&buf[..written as usize])
    }
}

/// Platform-aware error when `cwd` fails `is_absolute()`.
fn cwd_validation_error(path: &std::path::Path) -> String {
    let display = path.display();
    #[cfg(windows)]
    {
        let looks_unix = path
            .to_str()
            .is_some_and(|s| s.starts_with('/') && !s.contains(':'));
        if looks_unix {
            return format!(
                "Bash: `cwd` must be a Windows absolute path (e.g. `C:\\\\`), got `{display}`. \
                 This host is Windows — read `<env>Platform</env>` instead of using Unix paths."
            );
        }
        format!(
            "Bash: `cwd` must be an absolute Windows path (e.g. `C:\\\\project`), got `{display}`"
        )
    }
    #[cfg(not(windows))]
    {
        format!("Bash: `cwd` must be an absolute path, got `{display}`")
    }
}
