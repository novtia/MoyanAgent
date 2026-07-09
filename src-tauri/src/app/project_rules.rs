//! Project rule files stored under `<projectRoot>/.moyan/*.md`.
//!
//! Enabled rules are concatenated into the agent system prompt on every
//! generation. A small `.moyan/rules.json` manifest tracks which rule files are
//! disabled; any `*.md` not listed there is considered enabled.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tauri::State;

use crate::error::{AppError, AppResult};

use super::{session_project_cwd, validate_reader_write_path, AppState};

/// Folder (relative to the project root) that holds rule files.
pub const RULES_DIR: &str = ".moyan";
/// Manifest file inside [`RULES_DIR`] tracking disabled rules.
pub const RULES_MANIFEST: &str = "rules.json";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectRule {
    pub name: String,
    pub path: String,
    pub enabled: bool,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct RulesManifest {
    #[serde(default)]
    disabled: Vec<String>,
}

fn manifest_path(rules_dir: &Path) -> PathBuf {
    rules_dir.join(RULES_MANIFEST)
}

fn read_disabled(rules_dir: &Path) -> BTreeSet<String> {
    let text = match fs::read_to_string(manifest_path(rules_dir)) {
        Ok(t) => t,
        Err(_) => return BTreeSet::new(),
    };
    let manifest: RulesManifest = serde_json::from_str(&text).unwrap_or_default();
    manifest.disabled.into_iter().collect()
}

fn write_disabled(rules_dir: &Path, disabled: &BTreeSet<String>) -> AppResult<()> {
    let manifest = RulesManifest {
        disabled: disabled.iter().cloned().collect(),
    };
    let text = serde_json::to_string_pretty(&manifest)
        .map_err(|e| AppError::Other(format!("project_rules: serialize manifest: {e}")))?;
    fs::create_dir_all(rules_dir)
        .map_err(|e| AppError::Other(format!("project_rules: mkdir {:?}: {e}", rules_dir)))?;
    fs::write(manifest_path(rules_dir), text.as_bytes())
        .map_err(|e| AppError::Other(format!("project_rules: write manifest: {e}")))?;
    Ok(())
}

fn is_markdown(name: &str) -> bool {
    name.to_lowercase().ends_with(".md")
}

/// Top-level `*.md` file names inside `rules_dir`, sorted case-insensitively.
fn list_rule_files(rules_dir: &Path) -> Vec<String> {
    let mut names = Vec::new();
    let read = match fs::read_dir(rules_dir) {
        Ok(r) => r,
        Err(_) => return names,
    };
    for entry in read.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !is_markdown(&name) {
            continue;
        }
        if entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
            names.push(name);
        }
    }
    names.sort_by(|a, b| a.to_lowercase().cmp(&b.to_lowercase()));
    names
}

/// Concatenate all enabled rule files into a single `<project-rules>` block, or
/// `None` when there is nothing to inject.
pub fn collect_project_rules(project_cwd: &Path) -> Option<String> {
    let rules_dir = project_cwd.join(RULES_DIR);
    if !rules_dir.is_dir() {
        return None;
    }
    let disabled = read_disabled(&rules_dir);
    let mut blocks = Vec::new();
    for name in list_rule_files(&rules_dir) {
        if disabled.contains(&name) {
            continue;
        }
        let content = match fs::read_to_string(rules_dir.join(&name)) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let content = content.trim();
        if content.is_empty() {
            continue;
        }
        blocks.push(format!("## {name}\n{content}"));
    }
    if blocks.is_empty() {
        return None;
    }
    Some(format!(
        "<project-rules>\n{}\n</project-rules>",
        blocks.join("\n\n")
    ))
}

/// Enumerate rule files with their enabled state for the UI.
fn list_rules(project_cwd: &Path) -> Vec<ProjectRule> {
    let rules_dir = project_cwd.join(RULES_DIR);
    let mut out = Vec::new();
    if !rules_dir.is_dir() {
        return out;
    }
    let disabled = read_disabled(&rules_dir);
    for name in list_rule_files(&rules_dir) {
        let path = rules_dir.join(&name);
        out.push(ProjectRule {
            enabled: !disabled.contains(&name),
            name: name.clone(),
            path: path.to_string_lossy().into_owned(),
        });
    }
    out
}

fn set_rule_enabled_in_dir(rules_dir: &Path, name: &str, enabled: bool) -> AppResult<()> {
    let mut disabled = read_disabled(rules_dir);
    if enabled {
        disabled.remove(name);
    } else {
        disabled.insert(name.to_string());
    }
    write_disabled(rules_dir, &disabled)
}

#[tauri::command]
pub fn list_project_rules(
    state: State<'_, std::sync::Arc<AppState>>,
    session_id: String,
) -> Result<Vec<ProjectRule>, AppError> {
    let conn = state.conn()?;
    match session_project_cwd(&conn, &session_id) {
        Some(cwd) => Ok(list_rules(&cwd)),
        None => Ok(Vec::new()),
    }
}

#[tauri::command]
pub fn set_project_rule_enabled(
    state: State<'_, std::sync::Arc<AppState>>,
    session_id: String,
    path: String,
    enabled: bool,
) -> Result<(), AppError> {
    let conn = state.conn()?;
    let file_path = PathBuf::from(&path);
    let cwd = session_project_cwd(&conn, &session_id);
    let resolved = validate_reader_write_path(&file_path, cwd.as_deref())?;
    let rules_dir = resolved
        .parent()
        .ok_or_else(|| AppError::Invalid("set_project_rule_enabled: rule path has no parent".into()))?;
    let name = resolved
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .ok_or_else(|| {
            AppError::Invalid("set_project_rule_enabled: rule path has no file name".into())
        })?;
    if !is_markdown(&name) {
        return Err(AppError::Invalid(
            "set_project_rule_enabled: rule must be a .md file".into(),
        ));
    }
    set_rule_enabled_in_dir(rules_dir, &name, enabled)
}
