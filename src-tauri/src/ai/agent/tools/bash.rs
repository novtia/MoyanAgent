//! Shell-execution tool.
//!
//! Cross-platform — picks `cmd /C` / `powershell` on Windows and `sh -c`
//! on Unix. Honors a per-call timeout (defaults to 60s) and caps the
//! returned stdout/stderr so the model can't OOM itself on a noisy
//! command.
//!
//! Safety comes from three places:
//!
//! - [`crate::ai::agent::core::permission::PlanModeResolver`] refuses Bash
//!   outright while the agent is in
//!   [`crate::ai::agent::core::permission::PermissionMode::Plan`], because a
//!   shell command cannot be reliably classified as read-only.
//! - [`crate::ai::agent::core::permission::DefaultModeResolver`] rejects
//!   commands that would destroy the machine regardless of mode.
//! - This tool confines the working directory to the project root, so a
//!   command cannot be aimed at unrelated parts of the filesystem via `cwd`.
//!
//! None of that constrains what the command body itself can reach — a shell is
//! not a sandbox — but it removes the accidents: a wrong `cwd`, a `Plan`-mode
//! write, and the classic catastrophic one-liners.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use serde_json::{json, Value};
use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tokio::time::timeout;

use crate::ai::agent::tools::project_path;
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
            let requested_cwd = invocation.input.get("cwd").and_then(Value::as_str);
            let cwd = match resolve_cwd(&invocation.context.cwd, requested_cwd) {
                Ok(dir) => dir,
                Err(message) => return Ok(ToolResult::error(message)),
            };

            let mut cmd = build_command(&command);
            cmd.current_dir(&cwd);
            cmd.stdout(Stdio::piped());
            cmd.stderr(Stdio::piped());
            cmd.stdin(Stdio::null());
            // Reap the shell when this future is dropped — on timeout, or when
            // the agent loop cancels the turn. Without it the command keeps
            // running unsupervised and can still be writing to project files
            // long after the tool reported failure.
            cmd.kill_on_drop(true);

            let mut child = cmd
                .spawn()
                .map_err(|e| AppError::Other(format!("Bash: spawn failed: {e}")))?;
            let pid = child.id();

            let stdout_pipe = child.stdout.take();
            let stderr_pipe = child.stderr.take();

            // Both pipes must be drained *concurrently*. Draining stdout to EOF
            // first deadlocks any command that fills the stderr pipe buffer
            // meanwhile: it blocks writing to stderr, so it never closes stdout,
            // so neither side ever finishes and only the timeout breaks the tie.
            let exec = async {
                let (stdout_buf, stderr_buf, status) = tokio::join!(
                    async {
                        let mut buf = Vec::with_capacity(8 * 1024);
                        if let Some(p) = stdout_pipe {
                            drain_capped(p, &mut buf).await;
                        }
                        buf
                    },
                    async {
                        let mut buf = Vec::with_capacity(8 * 1024);
                        if let Some(p) = stderr_pipe {
                            drain_capped(p, &mut buf).await;
                        }
                        buf
                    },
                    child.wait(),
                );
                (stdout_buf, stderr_buf, status)
            };

            let (stdout_buf, stderr_buf, status) = tokio::select! {
                biased;
                // A cancelled turn must not leave a build or test run churning
                // over project files. Dropping the future kills the shell
                // (`kill_on_drop`), but its children survive that, so the tree
                // is torn down explicitly.
                () = invocation.context.abort.wait_aborted() => {
                    kill_process_tree(pid);
                    return Ok(ToolResult::error(
                        "Bash: cancelled before the command finished".to_string(),
                    ));
                }
                result = timeout(Duration::from_secs(timeout_secs), exec) => match result {
                    Ok(v) => v,
                    Err(_) => {
                        kill_process_tree(pid);
                        return Ok(ToolResult::error(format!(
                            "Bash: command timed out after {timeout_secs}s"
                        )));
                    }
                },
            };
            let status = status.map_err(|e| AppError::Other(format!("Bash: wait failed: {e}")))?;

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

/// Decide where the command runs.
///
/// `project_root` comes from the database project path; an explicit `cwd`
/// argument may only narrow it. Letting the model pick a directory freely turns
/// every shell call into a whole-filesystem tool: `cd C:\ && del /s` needs no
/// suspicious-looking command, just a suspicious `cwd`.
fn resolve_cwd(project_root: &Path, requested: Option<&str>) -> Result<PathBuf, String> {
    // Refuse to run without a working directory. Falling through to the host
    // process CWD would leak the app's own install directory.
    if project_root.as_os_str().is_empty() {
        return Err("Bash: no working directory available. Set the project's `path` in the \
             database, or pass an explicit absolute `cwd` inside it. To detect the OS, read \
             `<env>Platform</env>` in the system prompt — do not run `uname`."
            .to_string());
    }
    if !project_root.is_absolute() {
        return Err(cwd_validation_error(project_root));
    }

    let Some(raw) = requested.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(project_root.to_path_buf());
    };

    let requested = PathBuf::from(raw);
    if !requested.is_absolute() {
        return Err(cwd_validation_error(&requested));
    }
    if !project_path::is_within(project_root, &requested) {
        return Err(format!(
            "Bash: `cwd` must be inside the project folder `{}`, got `{}`. Run the command from \
             the project and use relative paths.",
            project_root.display(),
            requested.display()
        ));
    }
    if !requested.is_dir() {
        return Err(format!(
            "Bash: `cwd` is not an existing directory: `{}`",
            requested.display()
        ));
    }
    Ok(requested)
}

/// Kill the shell *and everything it started*.
///
/// `kill_on_drop` only reaps the shell itself, so a `npm test` or `cargo build`
/// it launched keeps running — still writing to project files long after the
/// tool reported a timeout or cancellation.
#[cfg(windows)]
fn kill_process_tree(pid: Option<u32>) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    let Some(pid) = pid else { return };
    let _ = std::process::Command::new("taskkill")
        .args(["/T", "/F", "/PID", &pid.to_string()])
        .creation_flags(CREATE_NO_WINDOW)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

#[cfg(not(windows))]
fn kill_process_tree(pid: Option<u32>) {
    let Some(pid) = pid else { return };
    // The shell is its own process-group leader (see `build_command`), so a
    // negative pid signals the whole group in one call.
    let _ = std::process::Command::new("kill")
        .args(["-KILL", &format!("-{pid}")])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
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
    // Own process group, so [`kill_process_tree`] can signal the shell's
    // descendants as well.
    cmd.process_group(0);
    cmd
}

/// Read a child pipe into `buf`, retaining at most `MAX_OUTPUT_BYTES + 1`
/// bytes — one past the display cap, so [`truncate_console`] still sees the
/// output as truncated.
///
/// Reading past the cap is *discarded* rather than skipped: a command whose
/// pipe fills up blocks forever, so the far end has to keep being consumed for
/// the process to reach exit. `read_to_end` would instead buffer the whole
/// stream, which a single `type <big file>` turns into an out-of-memory kill.
async fn drain_capped<R: tokio::io::AsyncRead + Unpin>(mut pipe: R, buf: &mut Vec<u8>) {
    const RETAIN: usize = MAX_OUTPUT_BYTES + 1;
    let mut chunk = [0u8; 8 * 1024];
    loop {
        match pipe.read(&mut chunk).await {
            Ok(0) | Err(_) => return,
            Ok(n) => {
                if buf.len() < RETAIN {
                    let take = n.min(RETAIN - buf.len());
                    buf.extend_from_slice(&chunk[..take]);
                }
            }
        }
    }
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

#[cfg(test)]
mod output_tests {
    use super::*;

    #[tokio::test]
    async fn drain_capped_consumes_everything_but_retains_a_bounded_prefix() {
        let source = vec![b'x'; 4 * MAX_OUTPUT_BYTES];
        let mut buf = Vec::new();
        drain_capped(&source[..], &mut buf).await;

        assert_eq!(
            buf.len(),
            MAX_OUTPUT_BYTES + 1,
            "a noisy command must not be buffered in full"
        );
        assert!(truncate_console(&buf).1, "the result still reads truncated");
    }

    #[tokio::test]
    async fn drain_capped_keeps_short_output_intact() {
        let mut buf = Vec::new();
        drain_capped(&b"hello"[..], &mut buf).await;
        assert_eq!(buf, b"hello");
        assert!(!truncate_console(&buf).1);
    }
}

#[cfg(test)]
mod cwd_tests {
    use super::*;

    fn project() -> PathBuf {
        let root = std::env::temp_dir().join(format!("moyan-bash-cwd-{}", std::process::id()));
        std::fs::create_dir_all(root.join("chapters")).unwrap();
        root
    }

    #[test]
    fn defaults_to_the_project_root() {
        let root = project();
        assert_eq!(resolve_cwd(&root, None).unwrap(), root);
        assert_eq!(
            resolve_cwd(&root, Some("   ")).unwrap(),
            root,
            "a blank cwd is not a request"
        );
    }

    #[test]
    fn accepts_a_subdirectory_of_the_project() {
        let root = project();
        let nested = root.join("chapters");
        assert_eq!(
            resolve_cwd(&root, Some(nested.to_str().unwrap())).unwrap(),
            nested
        );
    }

    /// The whole point of the check: a shell aimed outside the project turns
    /// every command into a filesystem-wide one.
    #[test]
    fn refuses_a_directory_outside_the_project() {
        let root = project();
        let outside = std::env::temp_dir();
        let err = resolve_cwd(&root, Some(outside.to_str().unwrap()))
            .expect_err("a parent directory is out of bounds");
        assert!(err.contains("inside the project folder"), "got: {err}");

        let escape = root.join("..").join("elsewhere");
        assert!(
            resolve_cwd(&root, Some(escape.to_str().unwrap())).is_err(),
            "`..` must not walk out either"
        );
    }

    #[test]
    fn refuses_a_relative_or_missing_directory() {
        let root = project();
        assert!(resolve_cwd(&root, Some("chapters")).is_err(), "must be absolute");
        let ghost = root.join("does-not-exist");
        let err = resolve_cwd(&root, Some(ghost.to_str().unwrap())).expect_err("missing dir");
        assert!(err.contains("not an existing directory"), "got: {err}");
    }

    #[test]
    fn refuses_to_run_without_a_project_path() {
        let err = resolve_cwd(Path::new(""), None).expect_err("no cwd at all");
        assert!(err.contains("no working directory"), "got: {err}");
    }
}
