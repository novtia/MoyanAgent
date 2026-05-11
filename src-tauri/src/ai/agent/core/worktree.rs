//! Git worktree isolation for sub-agents.
//!
//! When an [`crate::ai::agent::config::definition::AgentDefinition`]
//! specifies `isolation: worktree`, the runner allocates a fresh
//! `git worktree add` on a temporary branch and points the sub-agent's
//! CWD there. The handle's `Drop` invokes `git worktree remove --force`
//! so a panicking / errored run still cleans up.
//!
//! The implementation is deliberately blocking (`std::process::Command`)
//! — worktree setup is a one-shot synchronous step before any LLM
//! traffic, and avoiding tokio here keeps the dependency surface flat.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::error::{AppError, AppResult};

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// RAII handle for a temporary `git worktree`. The directory is removed
/// (along with the throwaway branch) when this handle is dropped.
pub struct WorktreeHandle {
    /// Absolute path to the worktree root.
    pub path: PathBuf,
    /// Branch the worktree is checked out on. We delete it on drop too.
    pub branch: String,
    /// Repo root the worktree was created from. Used as `cwd` for the
    /// cleanup `git worktree remove` invocation so it works regardless
    /// of where the parent process moved to.
    repo_root: PathBuf,
}

impl WorktreeHandle {
    /// Allocate a new worktree off `repo_root`. The repo root must be
    /// inside a git tree — we don't try to bootstrap one.
    pub fn acquire(repo_root: &Path) -> AppResult<Self> {
        ensure_git_repo(repo_root)?;

        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let branch = format!("agent/worktree-{stamp}-{id}");
        let path = repo_root
            .join(".atelier-worktrees")
            .join(format!("wt-{stamp}-{id}"));

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                AppError::Other(format!("worktree: mkdir {:?}: {e}", parent))
            })?;
        }

        let out = Command::new("git")
            .args(["worktree", "add", "-b"])
            .arg(&branch)
            .arg(&path)
            .arg("HEAD")
            .current_dir(repo_root)
            .output()
            .map_err(|e| AppError::Other(format!("worktree: spawn git: {e}")))?;
        if !out.status.success() {
            return Err(AppError::Other(format!(
                "worktree: `git worktree add` failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            )));
        }

        Ok(Self {
            path,
            branch,
            repo_root: repo_root.to_path_buf(),
        })
    }
}

impl Drop for WorktreeHandle {
    fn drop(&mut self) {
        // Best-effort cleanup; we deliberately swallow errors — the
        // worst case is a stale directory under `.atelier-worktrees/`
        // that the user can prune manually with `git worktree prune`.
        let _ = Command::new("git")
            .args(["worktree", "remove", "--force"])
            .arg(&self.path)
            .current_dir(&self.repo_root)
            .output();
        let _ = Command::new("git")
            .args(["branch", "-D"])
            .arg(&self.branch)
            .current_dir(&self.repo_root)
            .output();
    }
}

fn ensure_git_repo(path: &Path) -> AppResult<()> {
    let out = Command::new("git")
        .args(["rev-parse", "--is-inside-work-tree"])
        .current_dir(path)
        .output()
        .map_err(|e| AppError::Other(format!("worktree: spawn git: {e}")))?;
    if !out.status.success() {
        return Err(AppError::Other(format!(
            "worktree: {} is not inside a git work tree",
            path.display()
        )));
    }
    Ok(())
}
