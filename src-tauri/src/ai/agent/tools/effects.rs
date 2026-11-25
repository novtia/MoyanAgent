//! Structured workspace synchronization effects and static UI metadata for tools.
//!
//! Tools declare their runtime side effects via [`WorkspaceEffect`] on
//! [`super::ToolResult`] so the backend can broadcast a single unified
//! `workspace://sync` event to the frontend's right-panel sync bus.
//!
//! Static UI capabilities (such as which JSON input fields should stream live
//! into tool cards) are declared via [`ToolUiMeta`] on [`super::Tool::ui_meta`]
//! and synced to the frontend through `list_agent_tool_specs`.

use serde::{Deserialize, Serialize};

/// Static UI / sync capabilities declared by a tool at registration time.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolUiMeta {
    /// Input JSON field names whose partial string values should be streamed
    /// live into the UI tool card while the model is still generating arguments.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub streaming_input_fields: Vec<String>,
}

impl ToolUiMeta {
    pub fn streaming_fields(fields: &[&str]) -> Self {
        Self {
            streaming_input_fields: fields.iter().map(|s| (*s).to_string()).collect(),
        }
    }
}

/// One structured workspace mutation emitted by a tool or host operation.
///
/// Consumed by the frontend's right-panel sync bus (`workspace://sync`), never
/// sent back to the LLM.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WorkspaceEffect {
    /// A new file was created on disk.
    FileCreated {
        path: String,
        /// When true, the reader pre-caches this document in memory.
        #[serde(default)]
        cache_in_reader: bool,
    },
    /// An existing file's content was modified on disk.
    FileUpdated {
        path: String,
        /// Present when the backend also recorded a reviewable `pending_diff`.
        #[serde(skip_serializing_if = "Option::is_none")]
        pending_diff_id: Option<String>,
        /// When true, the reader opens/caches the document even if not yet open
        /// so reviewable diffs are immediately available.
        #[serde(default)]
        ensure_in_reader: bool,
    },
    /// A file was deleted from disk.
    FileDeleted { path: String },
    /// A file was renamed or moved on disk.
    FileRenamed { from: String, to: String },
    /// The project file tree may have changed (e.g. shell command or unknown write).
    FsTreeDirty,
    /// An incremental RoleState mutation was applied for a session/project scope.
    RoleStateOp {
        scope_id: String,
        op: serde_json::Value,
    },
    /// The RoleState board for a scope should be reloaded from persistence.
    RoleStateReset { scope_id: String },
    /// Workspace files were rolled back to a previous snapshot.
    WorkspaceRolledBack { paths: Vec<String> },
}
