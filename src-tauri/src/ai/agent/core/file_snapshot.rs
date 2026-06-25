//! In-memory buffer of pending file mutations for the file-snapshot system.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::ai::agent::tools::text_decode::{detect_and_decode, TextEncoding};

const MAX_SNAPSHOT_BYTES: usize = 2 * 1024 * 1024;

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

#[derive(Debug, Clone)]
pub struct PendingFileChange {
    pub path: PathBuf,
    pub op: FileOp,
    pub before_existed: bool,
    pub before_content: Option<String>,
    pub before_encoding: Option<TextEncoding>,
    pub before_had_bom: bool,
    pub restorable: bool,
}

#[derive(Debug, Default)]
pub struct FileSnapshotStore {
    pending: Mutex<HashMap<String, Vec<PendingFileChange>>>,
}

impl FileSnapshotStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record_before(&self, session_id: Option<&str>, path: &Path, op: FileOp) {
        let Some(sid) = session_id else {
            return;
        };
        let resolved = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());

        let (before_existed, before_content, before_encoding, before_had_bom, restorable) =
            match std::fs::read(&resolved) {
                Ok(bytes) => {
                    if bytes.len() > MAX_SNAPSHOT_BYTES {
                        (true, None, None, false, false)
                    } else {
                        let decoded = detect_and_decode(&bytes);
                        (
                            true,
                            Some(decoded.text),
                            Some(decoded.encoding),
                            decoded.had_bom,
                            true,
                        )
                    }
                }
                Err(_) => (false, None, None, false, true),
            };

        let change = PendingFileChange {
            path: resolved,
            op,
            before_existed,
            before_content,
            before_encoding,
            before_had_bom,
            restorable,
        };
        if let Ok(mut g) = self.pending.lock() {
            g.entry(sid.to_string()).or_default().push(change);
        }
    }

    pub fn take(&self, session_id: &str) -> Vec<PendingFileChange> {
        self.pending
            .lock()
            .ok()
            .and_then(|mut g| g.remove(session_id))
            .unwrap_or_default()
    }

    pub fn clear(&self, session_id: &str) {
        if let Ok(mut g) = self.pending.lock() {
            g.remove(session_id);
        }
    }
}
