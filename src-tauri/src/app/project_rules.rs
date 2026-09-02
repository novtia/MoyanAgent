//! Project rule files stored under `<projectRoot>/.moyan/*.md`.
//!
//! Enabled rules are concatenated into a single hidden user message and
//! prepended to chat history on every generation. A small `.moyan/rules.json`
//! manifest tracks which rule files are disabled; any `*.md` not listed there
//! is considered enabled.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tauri::State;

use crate::ai::chat::HistoryTurn;
use crate::error::{AppError, AppResult};

use super::reader_paths::{session_project_cwd, validate_reader_write_path};
use super::state::AppState;

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

/// Concatenate every enabled `.moyan/*.md` file into one string.
///
/// Each file becomes its own `<system-reminder>` section; the sections are
/// joined with a blank line. `None` when there is nothing to inject.
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
        blocks.push(format!(
            "<system-reminder>\nContents of {RULES_DIR}/{name} (project-rules):\n\n{content}\n</system-reminder>"
        ));
    }
    if blocks.is_empty() {
        return None;
    }
    Some(blocks.join("\n\n"))
}

/// Insert enabled project rules as the first hidden user turn.
pub fn prepend_project_rules(history: &mut Vec<HistoryTurn>, project_cwd: Option<&Path>) {
    let Some(cwd) = project_cwd.filter(|p| !p.as_os_str().is_empty()) else {
        return;
    };
    let Some(text) = collect_project_rules(cwd) else {
        return;
    };
    let mut head = vec![HistoryTurn {
        role: "user".into(),
        text: Some(text),
        thinking_content: None,
        images: Vec::new(),
        timeline: Vec::new(),
    }];
    head.append(history);
    *history = head;
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

#[cfg(test)]
mod collect_tests {
    use super::{collect_project_rules, prepend_project_rules, RULES_DIR};
    use crate::ai::chat::HistoryTurn;
    use std::fs;
    use std::path::PathBuf;

    fn temp_project(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "moyan-rules-{name}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join(RULES_DIR)).unwrap();
        dir
    }

    #[test]
    fn missing_folder_injects_nothing() {
        let dir = temp_project("missing");
        let _ = fs::remove_dir_all(dir.join(RULES_DIR));
        assert!(collect_project_rules(&dir).is_none());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn several_files_become_one_concatenated_text() {
        let dir = temp_project("many");
        fs::write(dir.join(RULES_DIR).join("a.md"), "alpha rule").unwrap();
        fs::write(dir.join(RULES_DIR).join("b.md"), "beta rule").unwrap();

        let text = collect_project_rules(&dir).expect("rules present");
        assert!(text.contains("Contents of .moyan/a.md (project-rules):"));
        assert!(text.contains("alpha rule"));
        assert!(text.contains("Contents of .moyan/b.md (project-rules):"));
        assert!(text.contains("beta rule"));
        assert!(
            text.find("alpha rule").unwrap() < text.find("beta rule").unwrap(),
            "files are concatenated in name order into a single string"
        );

        let mut history = vec![HistoryTurn {
            role: "user".into(),
            text: Some("你好".into()),
            thinking_content: None,
            images: Vec::new(),
            timeline: Vec::new(),
        }];
        prepend_project_rules(&mut history, Some(&dir));
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].text.as_deref(), Some(text.as_str()));
        assert_eq!(history[1].text.as_deref(), Some("你好"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn disabled_files_are_skipped() {
        let dir = temp_project("disabled");
        fs::write(dir.join(RULES_DIR).join("keep.md"), "keep me").unwrap();
        fs::write(dir.join(RULES_DIR).join("skip.md"), "drop me").unwrap();
        fs::write(
            dir.join(RULES_DIR).join("rules.json"),
            r#"{"disabled":["skip.md"]}"#,
        )
        .unwrap();

        let text = collect_project_rules(&dir).expect("keep.md still enabled");
        assert!(text.contains("keep me"));
        assert!(!text.contains("drop me"));

        let _ = fs::remove_dir_all(&dir);
    }
}
