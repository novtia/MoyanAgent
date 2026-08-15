//! Capture of file pre-images for the workspace rollback system.
//!
//! Every mutation is persisted to `file_snapshots` at the moment the tool is
//! about to touch disk, tagged with the session and the user turn
//! (`correlation_id`) that caused it. Binding to the assistant message happens
//! later, when that message is written — so a generation that is cancelled,
//! errors out, or dies with the process still leaves a traceable row instead
//! of an untracked file on disk.
//!
//! Recording is fallible on purpose: a tool that cannot snapshot must not
//! write, otherwise the workspace drifts out of the rollback record.

use std::path::Path;
use std::sync::Arc;

use crate::ai::agent::tools::text_decode::{detect_and_decode, TextEncoding};
use crate::data::db::DbPool;
use crate::data::paths;
use crate::error::{AppError, AppResult};

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

/// One captured pre-image, in the exact shape stored in `file_snapshots`.
#[derive(Debug, Clone)]
pub struct FileChangeRecord {
    /// Normalized path key shared with `pending_diffs`.
    pub path: String,
    pub op: FileOp,
    pub before_existed: bool,
    pub before_content: Option<String>,
    pub before_encoding: Option<TextEncoding>,
    pub before_had_bom: bool,
    pub restorable: bool,
}

impl FileChangeRecord {
    /// Pre-image for a path that does not exist yet: rolling back deletes
    /// whatever the tool is about to create.
    pub fn absent(path: &Path, op: FileOp) -> Self {
        Self {
            path: paths::normalized_path_key(path),
            op,
            before_existed: false,
            before_content: None,
            before_encoding: None,
            before_had_bom: false,
            restorable: true,
        }
    }

    /// Pre-image built from text the caller has *already* read.
    ///
    /// Tools that decide what to write based on the current contents (Edit, and
    /// Write when it preserves the existing encoding) must record that same
    /// read rather than going back to disk: a second read can observe a
    /// different file, and the snapshot would then describe a state the
    /// mutation was never computed from.
    pub fn from_read(
        path: &Path,
        op: FileOp,
        text: &str,
        encoding: TextEncoding,
        had_bom: bool,
    ) -> Self {
        let too_large = text.len() > MAX_SNAPSHOT_BYTES;
        Self {
            path: paths::normalized_path_key(path),
            op,
            before_existed: true,
            before_content: (!too_large).then(|| text.to_string()),
            before_encoding: (!too_large).then_some(encoding),
            before_had_bom: had_bom,
            restorable: !too_large,
        }
    }
}

/// Read the current on-disk state of `path` as a rollback pre-image.
///
/// Only [`std::io::ErrorKind::NotFound`] counts as "there was nothing here".
/// Every other failure is reported as an error so the caller aborts: treating a
/// locked, permission-denied or otherwise unreadable file as absent would
/// record `before_existed = false`, and rolling that row back **deletes the very
/// file whose contents could not be read**.
pub fn capture_before(path: &Path, op: FileOp) -> AppResult<FileChangeRecord> {
    let key = paths::normalized_path_key(path);
    let absent = FileChangeRecord {
        path: key.clone(),
        op,
        before_existed: false,
        before_content: None,
        before_encoding: None,
        before_had_bom: false,
        restorable: true,
    };

    // `symlink_metadata` does not traverse links, so a link planted inside the
    // project cannot make this snapshot describe a file somewhere else.
    let meta = match std::fs::symlink_metadata(path) {
        Ok(m) => m,
        // Absent file: rollback means deleting whatever gets created here.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(absent),
        Err(e) => {
            return Err(AppError::Other(format!(
                "file snapshot: cannot inspect {}: {e}. Refusing to modify a file whose \
                 current contents cannot be recorded — the change would not be undoable.",
                paths::display_path(path)
            )));
        }
    };

    if !meta.is_file() {
        return Err(AppError::Invalid(format!(
            "file snapshot: {} is not a regular file; refusing to modify it",
            paths::display_path(path)
        )));
    }

    if meta.len() > MAX_SNAPSHOT_BYTES as u64 {
        // Deliberately not read: pulling multiple megabytes into memory only to
        // discard them is what made this path an OOM risk. The row is still
        // written so a rollback can report that this file could not be restored
        // instead of quietly leaving it mutated.
        return Ok(FileChangeRecord {
            path: key,
            op,
            before_existed: true,
            before_content: None,
            before_encoding: None,
            before_had_bom: false,
            restorable: false,
        });
    }

    let bytes = std::fs::read(path).map_err(|e| {
        AppError::Other(format!(
            "file snapshot: cannot read {}: {e}. Refusing to modify a file whose current \
             contents cannot be recorded — the change would not be undoable.",
            paths::display_path(path)
        ))
    })?;
    let decoded = detect_and_decode(&bytes);
    Ok(FileChangeRecord {
        path: key,
        op,
        before_existed: true,
        before_content: Some(decoded.text),
        before_encoding: Some(decoded.encoding),
        before_had_bom: decoded.had_bom,
        restorable: true,
    })
}

/// Write-side entry point for the snapshot system, handed to every mutating
/// file tool.
#[derive(Debug, Default)]
pub struct FileSnapshotStore {
    pool: Option<Arc<DbPool>>,
}

impl FileSnapshotStore {
    /// Store without a database: records nothing. Used by tool unit tests that
    /// exercise file behaviour without the app's SQLite pool.
    pub fn new() -> Self {
        Self { pool: None }
    }

    pub fn with_pool(pool: Arc<DbPool>) -> Self {
        Self { pool: Some(pool) }
    }

    /// Persist the pre-image of `path` before the caller mutates it.
    ///
    /// Returns an error when the snapshot could not be stored; callers must
    /// abort the write rather than leave an unrecorded mutation behind. A
    /// missing `session_id` (no owning conversation) or a DB-less store are
    /// not errors — there is nothing to roll back to.
    pub fn record_before(
        &self,
        session_id: Option<&str>,
        request_message_id: Option<&str>,
        path: &Path,
        op: FileOp,
    ) -> AppResult<()> {
        if !self.is_recording(session_id) {
            return Ok(());
        }
        let change = capture_before(path, op)?;
        self.record(session_id, request_message_id, &change)
    }

    /// Whether a mutation for this session would actually be persisted. Tools
    /// consult it before doing work only needed to build a pre-image.
    pub fn is_recording(&self, session_id: Option<&str>) -> bool {
        self.pool.is_some() && session_id.is_some()
    }

    /// Persist a pre-image the caller captured itself (see
    /// [`FileChangeRecord::from_read`]).
    pub fn record(
        &self,
        session_id: Option<&str>,
        request_message_id: Option<&str>,
        change: &FileChangeRecord,
    ) -> AppResult<()> {
        let (Some(pool), Some(sid)) = (self.pool.as_ref(), session_id) else {
            return Ok(());
        };
        let conn = pool
            .get()
            .map_err(|e| AppError::Other(format!("file snapshot: db connection: {e}")))?;
        crate::data::file_snapshot::record_change(&conn, sid, request_message_id, change)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::db::test_support::TempDb;

    #[test]
    fn recording_persists_before_any_assistant_message_exists() {
        let db = TempDb::new("snapshot-store");
        let store = FileSnapshotStore::with_pool(Arc::new(db.pool()));
        let dir = std::env::temp_dir().join(format!("atelier-snap-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let target = dir.join("draft.md");
        let _ = std::fs::remove_file(&target);

        store
            .record_before(Some("s1"), Some("u1"), &target, FileOp::Create)
            .unwrap();

        let conn = db.conn();
        let (bound, request): (Option<String>, Option<String>) = conn
            .query_row(
                "SELECT message_id, request_message_id FROM file_snapshots WHERE session_id = ?1",
                rusqlite::params!["s1"],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .expect("row written at tool time");
        assert!(bound.is_none(), "not bound to a message yet");
        assert_eq!(request.as_deref(), Some("u1"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_store_without_a_database_records_nothing() {
        let store = FileSnapshotStore::new();
        let target = std::env::temp_dir().join("atelier-no-db.md");
        assert!(store
            .record_before(Some("s1"), Some("u1"), &target, FileOp::Create)
            .is_ok());
    }

    #[test]
    fn a_missing_file_is_recorded_as_absent() {
        let dir = std::env::temp_dir().join(format!("atelier-absent-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let target = dir.join("nope.md");
        let _ = std::fs::remove_file(&target);

        let change = capture_before(&target, FileOp::Create).expect("absent is not an error");
        assert!(!change.before_existed);
        assert!(change.restorable, "rollback deletes what gets created here");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The dangerous case: an existing file that cannot be read must never be
    /// recorded as absent, because rolling that row back would delete it.
    #[test]
    fn an_unreadable_file_aborts_instead_of_looking_absent() {
        let dir = std::env::temp_dir().join(format!("atelier-unreadable-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();

        // A directory stands in for any path that exists but yields no file
        // bytes; `read` fails on it exactly like a locked file does.
        let err = capture_before(&dir, FileOp::Update)
            .expect_err("a non-file must not be snapshot-able");
        assert!(
            err.to_string().contains("not a regular file"),
            "unexpected error: {err}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_oversized_file_is_recorded_as_present_but_unrestorable() {
        let dir = std::env::temp_dir().join(format!("atelier-big-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let target = dir.join("huge.md");
        std::fs::write(&target, vec![b'x'; MAX_SNAPSHOT_BYTES + 1]).unwrap();

        let change = capture_before(&target, FileOp::Update).expect("oversized still records");
        assert!(change.before_existed, "the file was there");
        assert!(!change.restorable, "its pre-image was not captured");
        assert!(
            change.before_content.is_none(),
            "the bytes must not be pulled into memory"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
