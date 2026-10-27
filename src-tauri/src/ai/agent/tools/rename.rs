//! `Rename` — change a file's name without touching its contents.
//!
//! The file stays in the same folder. `new_name` is a file name, not a path:
//! a bare title keeps the current extension (`第二章` on `第一章.md` becomes
//! `第二章.md`). An explicit extension is honoured only when it looks like
//! one (short, ASCII alphanumeric), so a dotted title such as `v1.2 大纲`
//! does not lose `.md`.
//!
//! Rollback is two snapshot rows recorded before the rename: the source as a
//! delete (restore its bytes at the old path) and the destination as a create
//! (delete the new path). A case-only rename of the same file records the
//! source alone, because the destination is not a second file.

use std::path::Path;
use std::sync::Arc;

use serde_json::{json, Value};

use crate::ai::agent::core::file_snapshot::{capture_before, FileOp, FileSnapshotStore};
use crate::ai::agent::tools::project_path::{self, display_path, FILE_REF_DESC};
use crate::ai::agent::tools::read_receipt::{self, take_receipt_key};
use crate::ai::agent::tools::{Tool, ToolFuture, ToolInvocation, ToolResult, ToolSpec};
use crate::data::db::DbPool;
use crate::data::paths;
use crate::error::{AppError, AppResult};

const TOOL_NAME: &str = "Rename";

#[derive(Clone)]
pub struct RenameTool {
    spec: ToolSpec,
    snapshots: Arc<FileSnapshotStore>,
    pool: Option<Arc<DbPool>>,
}

impl RenameTool {
    pub fn new(snapshots: Arc<FileSnapshotStore>) -> Self {
        Self {
            snapshots,
            pool: None,
            spec: ToolSpec {
                name: TOOL_NAME.to_string(),
                description: "Rename a file inside the project. Changes the file name only: \
                    the file stays in the same folder and its contents are not modified. \
                    Pass `path` (the current file) and `new_name` (the new file name, not a \
                    path). A bare title keeps the current extension — `第二章` on \
                    `第一章.md` becomes `第二章.md`. Include an extension only when you \
                    mean to change it (`notes.txt`). Fails if a different file already \
                    has that name. Use this to retitle an existing document; do not \
                    CreateDoc a second copy. Use Edit to change the body."
                    .to_string(),
                schema: json!({
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": FILE_REF_DESC
                        },
                        "new_name": {
                            "type": "string",
                            "description": "New file name in the same folder. Not a path. \
                                A title without an extension keeps the current one. \
                                Pass an extension only to change it."
                        }
                    },
                    "required": ["path", "new_name"]
                }),
                read_only: false,
                concurrency_safe: false,
            },
        }
    }

    pub fn with_pool(mut self, pool: Arc<DbPool>) -> Self {
        self.pool = Some(pool);
        self
    }
}

impl Tool for RenameTool {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    fn validate(&self, input: &Value) -> AppResult<()> {
        require_nonempty(input, "path")?;
        require_nonempty(input, "new_name")?;
        Ok(())
    }

    fn execute<'a>(&'a self, invocation: ToolInvocation<'a>) -> ToolFuture<'a> {
        Box::pin(async move {
            let raw = invocation
                .input
                .get("path")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let new_name = invocation
                .input
                .get("new_name")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let cwd = &invocation.context.cwd;

            let path = match project_path::resolve_project_file_strict(cwd, raw, TOOL_NAME) {
                Ok(path) => path,
                Err(e) => return Ok(ToolResult::error(e.to_string())),
            };
            if let Err(e) = crate::ai::agent::tools::pe_docs::refuse_nondoc(TOOL_NAME, &path) {
                return Ok(ToolResult::error(e.to_string()));
            }
            if !path.exists() {
                let hint = project_path::resolve_project_file(cwd, raw, TOOL_NAME)
                    .ok()
                    .filter(|found| found != &path)
                    .map(|found| format!(" Did you mean `{}`?", display_path(&found)))
                    .unwrap_or_default();
                return Ok(ToolResult::error(format!(
                    "{TOOL_NAME}: file does not exist: {}.{hint}",
                    display_path(&path)
                )));
            }
            if !path.is_file() {
                return Ok(ToolResult::error(format!(
                    "{TOOL_NAME}: not a regular file: {}",
                    display_path(&path)
                )));
            }

            let file_name = match compose_file_name(new_name, &path) {
                Ok(name) => name,
                Err(e) => return Ok(ToolResult::error(e.to_string())),
            };
            let current = path
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            if file_name == current {
                return Ok(ToolResult::error(format!(
                    "{TOOL_NAME}: `{current}` already has that name"
                )));
            }

            let parent = path.parent().ok_or_else(|| {
                AppError::Invalid(format!(
                    "{TOOL_NAME}: `{}` has no parent directory",
                    display_path(&path)
                ))
            })?;
            let dest = parent.join(&file_name);
            if let Err(e) = crate::ai::agent::tools::pe_docs::refuse_nondoc(TOOL_NAME, &dest) {
                return Ok(ToolResult::error(e.to_string()));
            }
            let root = project_path::resolve_project_root(cwd)?;
            let root_canon =
                paths::canonicalize_with_missing_tail(&root).unwrap_or_else(|_| root.clone());
            if !paths::is_within(&root_canon, &dest) {
                return Ok(ToolResult::error(format!(
                    "{TOOL_NAME}: `new_name` resolves outside the project"
                )));
            }

            let same_file = dest.exists() && same_existing_file(&path, &dest);
            if dest.exists() && !same_file {
                return Ok(ToolResult::error(format!(
                    "{TOOL_NAME}: `{}` already exists",
                    display_path(&dest)
                )));
            }

            // Source pre-image first, then the not-yet-existing destination, so
            // a rollback deletes the new path and writes the bytes back to the
            // old one. A case-only rename is the same file — a second "create"
            // row would snapshot it as already present and refuse to remove it.
            // An unrestorable source (too large to store) must not be renamed:
            // rolling the destination back would delete the only copy.
            let session_id = invocation.context.session_id.as_deref();
            let request_id = invocation.context.correlation_id.as_deref();
            if self.snapshots.is_recording(session_id) {
                let source = capture_before(&path, FileOp::Delete)?;
                if !source.restorable {
                    return Ok(ToolResult::error(format!(
                        "{TOOL_NAME}: `{}` is too large to snapshot, so it cannot be renamed",
                        display_path(&path)
                    )));
                }
                self.snapshots.record(session_id, request_id, &source)?;
                if !same_file {
                    self.snapshots
                        .record_before(session_id, request_id, &dest, FileOp::Create)?;
                }
            }

            let from_key = take_receipt_key(&path);
            std::fs::rename(&path, &dest).map_err(|e| {
                AppError::Other(format!("{TOOL_NAME}: rename {:?} -> {:?}: {e}", path, dest))
            })?;
            let canonical = std::fs::canonicalize(&dest).unwrap_or(dest);
            read_receipt::rekey_receipt(&invocation.context.read_file_state, &from_key, &canonical);

            if let Err(e) =
                self.remap_reviews(invocation.context.session_id.as_deref(), &path, &canonical)
            {
                let to_key = take_receipt_key(&canonical);
                if std::fs::rename(&canonical, &path).is_ok() {
                    read_receipt::rekey_receipt(
                        &invocation.context.read_file_state,
                        &to_key,
                        &path,
                    );
                }
                return Err(e);
            }

            Ok(ToolResult::ok(json!({
                "from": display_path(&path),
                "path": display_path(&canonical),
                "name": file_name,
                "renamed": true,
            })))
        })
    }
}

impl RenameTool {
    fn remap_reviews(&self, session_id: Option<&str>, from: &Path, to: &Path) -> AppResult<()> {
        let (Some(pool), Some(sid)) = (self.pool.as_ref(), session_id) else {
            return Ok(());
        };
        let conn = pool
            .get()
            .map_err(|e| AppError::Other(format!("{TOOL_NAME}: db connection: {e}")))?;
        crate::data::pending_diff::remap_path(&conn, sid, from, to)?;
        Ok(())
    }
}

fn require_nonempty(input: &Value, field: &str) -> AppResult<()> {
    let raw = input
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| AppError::Invalid(format!("{TOOL_NAME}: `{field}` must be a string")))?;
    if raw.trim().is_empty() {
        return Err(AppError::Invalid(format!(
            "{TOOL_NAME}: `{field}` must be non-empty"
        )));
    }
    Ok(())
}

/// Build the destination file name. Separators are rejected so a rename cannot
/// move the file into another folder.
fn compose_file_name(new_name: &str, source: &Path) -> AppResult<String> {
    let segments = project_path::parse_breadcrumb_segments(new_name, TOOL_NAME)?;
    if segments.len() != 1 {
        return Err(AppError::Invalid(format!(
            "{TOOL_NAME}: `new_name` must be a file name in the same folder, not a path"
        )));
    }
    let name = segments.into_iter().next().unwrap_or_default();
    if name.is_empty() || name == "." || name == ".." {
        return Err(AppError::Invalid(format!(
            "{TOOL_NAME}: `new_name` must be a file name"
        )));
    }
    if explicit_extension(&name).is_some() {
        return Ok(name);
    }
    match source.extension().and_then(|ext| ext.to_str()) {
        Some(ext) if !ext.is_empty() => Ok(format!("{name}.{ext}")),
        _ => Ok(name),
    }
}

/// `Some` when the last dot introduces a real extension the model chose, not a
/// dotted title (`v1.2 大纲`).
fn explicit_extension(name: &str) -> Option<&str> {
    let (stem, ext) = name.rsplit_once('.')?;
    if stem.is_empty() || ext.is_empty() {
        return None;
    }
    if ext.len() > 10 || !ext.chars().all(|c| c.is_ascii_alphanumeric()) {
        return None;
    }
    Some(ext)
}

fn same_existing_file(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(left), Ok(right)) => left == right,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    use crate::ai::agent::core::context::ToolUseContextBuilder;
    use crate::ai::agent::tools::read_receipt::{is_fresh_write, record_receipt};
    use crate::ai::agent::types::{AgentId, MessageId};
    use std::sync::atomic::{AtomicU32, Ordering};

    static COUNTER: AtomicU32 = AtomicU32::new(0);

    fn ctx_in_temp_dir() -> Arc<crate::ai::agent::core::context::ToolUseContext> {
        let dir = std::env::temp_dir().join(format!(
            "moyan-rename-test-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        ToolUseContextBuilder::new(AgentId::new(), dir).build().0
    }

    fn tool() -> RenameTool {
        RenameTool::new(Arc::new(FileSnapshotStore::new()))
    }

    async fn run(
        ctx: &crate::ai::agent::core::context::ToolUseContext,
        path: &str,
        new_name: &str,
    ) -> ToolResult {
        tool()
            .execute(ToolInvocation {
                id: MessageId("rename".into()),
                input: json!({ "path": path, "new_name": new_name }),
                context: ctx,
            })
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn bare_title_keeps_the_extension_and_the_bytes() {
        let ctx = ctx_in_temp_dir();
        let src = ctx.cwd.join("第一章.md");
        let bytes = "原稿正文".as_bytes();
        std::fs::write(&src, bytes).unwrap();
        record_receipt(&ctx.read_file_state, &src, "原稿正文", true);

        let result = run(ctx.as_ref(), "第一章.md", "v1.2 大纲").await;
        assert!(!result.is_error, "{result:?}");
        let dest = ctx.cwd.join("v1.2 大纲.md");
        assert!(!src.exists());
        assert_eq!(std::fs::read(&dest).unwrap(), bytes);
        assert_eq!(result.content["name"].as_str(), Some("v1.2 大纲.md"));
        assert!(is_fresh_write(&ctx.read_file_state, &dest, "原稿正文"));
        let _ = std::fs::remove_dir_all(&ctx.cwd);
    }

    #[tokio::test]
    async fn explicit_extension_replaces_the_old_one() {
        let ctx = ctx_in_temp_dir();
        std::fs::write(ctx.cwd.join("笔记.md"), "正文").unwrap();
        let result = run(ctx.as_ref(), "笔记.md", "笔记.txt").await;
        assert!(!result.is_error, "{result:?}");
        assert!(ctx.cwd.join("笔记.txt").is_file());
        assert!(!ctx.cwd.join("笔记.md").exists());
        let _ = std::fs::remove_dir_all(&ctx.cwd);
    }

    #[tokio::test]
    async fn refuses_to_overwrite_a_different_file() {
        let ctx = ctx_in_temp_dir();
        std::fs::write(ctx.cwd.join("甲.md"), "甲").unwrap();
        std::fs::write(ctx.cwd.join("乙.md"), "乙").unwrap();
        let result = run(ctx.as_ref(), "甲.md", "乙.md").await;
        assert!(result.is_error);
        assert_eq!(
            std::fs::read_to_string(ctx.cwd.join("甲.md")).unwrap(),
            "甲"
        );
        assert_eq!(
            std::fs::read_to_string(ctx.cwd.join("乙.md")).unwrap(),
            "乙"
        );
        let _ = std::fs::remove_dir_all(&ctx.cwd);
    }

    #[tokio::test]
    async fn refuses_a_path_and_an_unchanged_name() {
        let ctx = ctx_in_temp_dir();
        std::fs::write(ctx.cwd.join("第一章.md"), "原稿").unwrap();
        let nested = run(ctx.as_ref(), "第一章.md", r"草稿\第二章").await;
        assert!(nested.is_error);
        assert!(ctx.cwd.join("第一章.md").is_file());

        let same = run(ctx.as_ref(), "第一章.md", "第一章").await;
        assert!(same.is_error);
        assert!(ctx.cwd.join("第一章.md").is_file());
        let _ = std::fs::remove_dir_all(&ctx.cwd);
    }

    #[tokio::test]
    async fn nested_file_stays_in_its_folder() {
        let ctx = ctx_in_temp_dir();
        let dir = ctx.cwd.join("chapters");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("01.md"), "一").unwrap();
        let result = run(ctx.as_ref(), r"chapters\01.md", "开篇").await;
        assert!(!result.is_error, "{result:?}");
        assert!(dir.join("开篇.md").is_file());
        assert!(!dir.join("01.md").exists());
        let _ = std::fs::remove_dir_all(&ctx.cwd);
    }

    #[tokio::test]
    async fn rollback_plan_deletes_the_new_name_and_restores_the_old_one() {
        let db = crate::data::db::test_support::TempDb::new("rename-rollback");
        let dir = std::env::temp_dir().join(format!(
            "moyan-rename-rollback-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let (ctx, _abort) = ToolUseContextBuilder::new(AgentId::new(), dir.clone())
            .session_id("s-rename")
            .correlation_id("u-rename")
            .build();
        let src = dir.join("旧名.md");
        std::fs::write(&src, "原稿").unwrap();
        let from_key = paths::normalized_path_key(&src);
        crate::data::pending_diff::insert(
            &db.conn(),
            "s-rename",
            &from_key,
            "旧",
            "原稿",
            "旧",
            "原稿",
            Some("utf-8"),
            false,
            Some("u-rename"),
        )
        .unwrap();

        let snapshots = Arc::new(FileSnapshotStore::with_pool(Arc::new(db.pool())));
        let result = RenameTool::new(snapshots)
            .with_pool(Arc::new(db.pool()))
            .execute(ToolInvocation {
                id: MessageId("rename".into()),
                input: json!({ "path": "旧名.md", "new_name": "新名" }),
                context: ctx.as_ref(),
            })
            .await
            .unwrap();
        assert!(!result.is_error, "{result:?}");
        let dest = dir.join("新名.md");
        assert!(dest.is_file());
        assert!(!src.exists());

        let reviews = crate::data::pending_diff::list_for_session(&db.conn(), "s-rename").unwrap();
        assert_eq!(reviews.len(), 1);
        assert_eq!(reviews[0].path, paths::normalized_path_key(&dest));

        let plan = crate::data::file_snapshot::plan_rollback_from_message(
            &db.conn(),
            "s-rename",
            "u-rename",
        )
        .unwrap()
        .expect("rename recorded a rollback plan");
        for restore in &plan.restores {
            let file = PathBuf::from(&restore.path);
            if restore.delete {
                let _ = std::fs::remove_file(&file);
            } else if let Some(content) = &restore.content {
                std::fs::write(&file, content).unwrap();
            }
        }
        assert_eq!(std::fs::read_to_string(&src).unwrap(), "原稿");
        assert!(!dest.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
