//! In-memory buffer of pending file mutations for the file-snapshot system.
//!
//! File tools (`Write` / `Edit` / `CreateDoc` / `Delete`) cannot know which
//! assistant message they belong to at execution time — the message row is
//! only inserted once the whole generation finishes. So each tool captures the
//! file's pre-image HERE, keyed by `session_id`, and the persistence layer
//! drains the buffer at finalize / cancel / error and binds the changes to the
//! freshly-inserted message id.
//!
//! This mirrors how `StreamBlocks` accumulates ordered blocks during a run and
//! flushes them on completion.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Pre-images larger than this are not stored (the row is still recorded with
/// `restorable = false` so a `create` rollback can still delete the file).
const MAX_SNAPSHOT_BYTES: usize = 2 * 1024 * 1024;

/// The kind of mutation a tool performed on a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileOp {
    Create,
    Update,
    Delete,
}

impl FileOp {
    pub fn as_str(&self) -> &'static str {
        match self {
            FileOp::Create => "create",
            FileOp::Update => "update",
            FileOp::Delete => "delete",
        }
    }
}

/// A single queued file change, capturing the state BEFORE the mutation.
#[derive(Debug, Clone)]
pub struct PendingFileChange {
    pub path: PathBuf,
    pub op: FileOp,
    /// Whether the file existed before the mutation.
    pub before_existed: bool,
    /// The pre-image text. `None` when the file didn't exist, was binary, or
    /// exceeded [`MAX_SNAPSHOT_BYTES`].
    pub before_content: Option<String>,
    /// `false` when the pre-image couldn't be captured as UTF-8 / was too big;
    /// such rows cannot restore content (but `create` rollbacks still delete).
    pub restorable: bool,
}

/// Session-scoped buffer of pending file changes. Lives on `AppState` so the
/// file tools, the persistence layer and the rollback path all share it.
#[derive(Debug, Default)]
pub struct FileSnapshotStore {
    /// session_id → ordered list of pending changes (execution order).
    pending: Mutex<HashMap<String, Vec<PendingFileChange>>>,
}

impl FileSnapshotStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Capture the on-disk state of `path` BEFORE a mutation and queue it under
    /// `session_id`. No-op for sessionless runs (`session_id == None`).
    pub fn record_before(&self, session_id: Option<&str>, path: &Path, op: FileOp) {
        let Some(sid) = session_id else {
            return;
        };
        // Canonicalize when the file exists so rollback targets the same path
        // regardless of how the tool addressed it. For brand-new files (Create)
        // canonicalize fails, so keep the absolute path the tool supplied.
        let resolved = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());

        let (before_existed, before_content, restorable) = match std::fs::read(&resolved) {
            Ok(bytes) => {
                if bytes.len() > MAX_SNAPSHOT_BYTES {
                    (true, None, false)
                } else {
                    match String::from_utf8(bytes) {
                        Ok(text) => (true, Some(text), true),
                        Err(_) => (true, None, false),
                    }
                }
            }
            // No pre-image: the file is being created. Rollback = delete it.
            Err(_) => (false, None, true),
        };

        let change = PendingFileChange {
            path: resolved,
            op,
            before_existed,
            before_content,
            restorable,
        };
        if let Ok(mut g) = self.pending.lock() {
            g.entry(sid.to_string()).or_default().push(change);
        }
    }

    /// Remove and return all pending changes for `session_id`.
    pub fn take(&self, session_id: &str) -> Vec<PendingFileChange> {
        self.pending
            .lock()
            .ok()
            .and_then(|mut g| g.remove(session_id))
            .unwrap_or_default()
    }

    /// Drop any buffered (un-persisted) changes for `session_id`.
    pub fn clear(&self, session_id: &str) {
        if let Ok(mut g) = self.pending.lock() {
            g.remove(session_id);
        }
    }
}
