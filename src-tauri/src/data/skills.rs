//! Local skill packages (`SKILL.md` with YAML-ish frontmatter).
//!
//! Layout:
//! - Builtin: embedded via `include_str!`, merged at list time
//! - User: `<app_data>/atelier/skills/<id>/SKILL.md`

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tauri::AppHandle;

use crate::data::paths;
use crate::error::{AppError, AppResult};

pub const SKILL_FILE: &str = "SKILL.md";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillInfo {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub when_to_use: String,
    /// Markdown body (without frontmatter).
    pub body: String,
    /// `builtin` | `user`
    pub source: String,
    /// Absolute path to SKILL.md when source is user; empty for builtin.
    #[serde(default)]
    pub path: String,
    pub enabled: bool,
}

/// Directory for user-installed skills.
pub fn skills_dir(app: &AppHandle) -> AppResult<PathBuf> {
    paths::skills_dir(app)
}

fn builtin_skill_sources() -> Vec<&'static str> {
    vec![
        include_str!("../../skills/role-card/SKILL.md"),
        include_str!("../../skills/trpg-director/SKILL.md"),
    ]
}

/// Parse `---` frontmatter + body. Keys are simple `key: value` lines;
/// `tags` accepts `[a, b]` or comma-separated.
pub fn parse_skill_md(raw: &str, source: &str, path: &str) -> AppResult<SkillInfo> {
    let text = raw.trim_start_matches('\u{feff}');
    let (front, body) = if text.starts_with("---") {
        let rest = &text[3..];
        let end = rest
            .find("\n---")
            .ok_or_else(|| AppError::Invalid("SKILL.md: missing closing frontmatter ---".into()))?;
        let front = rest[..end].trim();
        let after = rest[end + 4..].trim_start_matches(['\r', '\n']);
        (front, after.to_string())
    } else {
        ("", text.to_string())
    };

    let mut id = String::new();
    let mut name = String::new();
    let mut description = String::new();
    let mut author = String::new();
    let mut version = String::new();
    let mut when_to_use = String::new();
    let mut tags: Vec<String> = Vec::new();

    for line in front.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((k, v)) = line.split_once(':') else {
            continue;
        };
        let key = k.trim();
        let val = v.trim().trim_matches('"').trim_matches('\'');
        match key {
            "id" => id = val.to_string(),
            "name" => name = val.to_string(),
            "description" => description = val.to_string(),
            "author" => author = val.to_string(),
            "version" => version = val.to_string(),
            "when_to_use" => when_to_use = val.to_string(),
            "tags" => tags = parse_tags(val),
            _ => {}
        }
    }

    if id.trim().is_empty() {
        // Fallback: directory name from path, or slug from name.
        if let Some(parent) = Path::new(path).parent() {
            if let Some(n) = parent.file_name().and_then(|s| s.to_str()) {
                id = n.to_string();
            }
        }
        if id.trim().is_empty() && !name.is_empty() {
            id = slugify(&name);
        }
    }
    if id.trim().is_empty() {
        return Err(AppError::Invalid("SKILL.md: missing id".into()));
    }
    if name.trim().is_empty() {
        name = id.clone();
    }

    Ok(SkillInfo {
        id: id.trim().to_string(),
        name,
        description,
        author,
        version,
        tags,
        when_to_use,
        body,
        source: source.to_string(),
        path: path.to_string(),
        enabled: false,
    })
}

fn parse_tags(val: &str) -> Vec<String> {
    let v = val.trim();
    let inner = v
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .unwrap_or(v);
    inner
        .split(',')
        .map(|s| s.trim().trim_matches('"').trim_matches('\'').to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

fn slugify(s: &str) -> String {
    let mut out = String::new();
    for ch in s.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if ch == '-' || ch == '_' || ch.is_whitespace() {
            if !out.ends_with('-') {
                out.push('-');
            }
        }
    }
    out.trim_matches('-').to_string()
}

fn load_user_skills(app: &AppHandle) -> AppResult<Vec<SkillInfo>> {
    let root = skills_dir(app)?;
    let mut out = Vec::new();
    let entries = match std::fs::read_dir(&root) {
        Ok(e) => e,
        Err(_) => return Ok(out),
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let skill_path = path.join(SKILL_FILE);
        if !skill_path.is_file() {
            continue;
        }
        let raw = std::fs::read_to_string(&skill_path).map_err(|e| {
            AppError::Other(format!("read {}: {e}", skill_path.display()))
        })?;
        match parse_skill_md(&raw, "user", &skill_path.to_string_lossy()) {
            Ok(info) => {
                out.push(info);
            }
            Err(e) => {
                eprintln!("[skills] skip {}: {e}", skill_path.display());
            }
        }
    }
    Ok(out)
}

fn load_builtin_skills() -> Vec<SkillInfo> {
    let mut out = Vec::new();
    for raw in builtin_skill_sources() {
        match parse_skill_md(raw, "builtin", "") {
            Ok(info) => out.push(info),
            Err(e) => eprintln!("[skills] builtin parse error: {e}"),
        }
    }
    out
}

/// List all skills, applying `enabled_ids`. Builtin wins on id conflict over user? —
/// user overrides builtin with same id.
pub fn list_skills(app: &AppHandle, enabled_ids: &[String]) -> AppResult<Vec<SkillInfo>> {
    let enabled: std::collections::HashSet<&str> =
        enabled_ids.iter().map(|s| s.as_str()).collect();

    let mut by_id: std::collections::HashMap<String, SkillInfo> = std::collections::HashMap::new();
    for mut s in load_builtin_skills() {
        s.enabled = enabled.contains(s.id.as_str());
        by_id.insert(s.id.clone(), s);
    }
    for mut s in load_user_skills(app)? {
        s.enabled = enabled.contains(s.id.as_str());
        by_id.insert(s.id.clone(), s); // user overrides
    }

    let mut list: Vec<SkillInfo> = by_id.into_values().collect();
    list.sort_by(|a, b| {
        b.enabled
            .cmp(&a.enabled)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    Ok(list)
}

pub fn get_skill(app: &AppHandle, id: &str, enabled_ids: &[String]) -> AppResult<SkillInfo> {
    let id = id.trim();
    list_skills(app, enabled_ids)?
        .into_iter()
        .find(|s| s.id == id)
        .ok_or_else(|| AppError::Invalid(format!("unknown skill id {id:?}")))
}

/// Resolve skill bodies for `@skill:{"id":…}` cites found in prompt text.
pub fn resolve_invoked_from_prompt(
    app: &AppHandle,
    prompt: &str,
    enabled_ids: &[String],
) -> AppResult<Vec<(String, String)>> {
    let ids = extract_skill_cite_ids(prompt);
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let all = list_skills(app, enabled_ids)?;
    let mut out = Vec::new();
    for id in ids {
        if let Some(s) = all.iter().find(|x| x.id == id) {
            // Allow cite even if not enabled — user explicitly @-mentioned it.
            out.push((s.name.clone(), s.body.clone()));
        }
    }
    Ok(out)
}

/// Scan prompt for `@skill:{"id":"…"}` tokens.
pub fn extract_skill_cite_ids(prompt: &str) -> Vec<String> {
    let mut ids = Vec::new();
    let mut search = prompt;
    while let Some(at) = search.find("@skill:") {
        let after = &search[at + 7..];
        if !after.starts_with('{') {
            search = &search[at + 7..];
            continue;
        }
        let mut depth = 0usize;
        let mut end = None;
        for (i, c) in after.char_indices() {
            match c {
                '{' => depth += 1,
                '}' => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        end = Some(i + c.len_utf8());
                        break;
                    }
                }
                _ => {}
            }
        }
        if let Some(end) = end {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&after[..end]) {
                if let Some(id) = v.get("id").and_then(|x| x.as_str()) {
                    let id = id.trim();
                    if !id.is_empty() && !ids.iter().any(|x| x == id) {
                        ids.push(id.to_string());
                    }
                }
            }
            search = &after[end..];
        } else {
            break;
        }
    }
    ids
}

/// Import a SKILL.md (or a folder containing it) into the user skills dir.
pub fn import_skill_file(app: &AppHandle, src: &Path) -> AppResult<SkillInfo> {
    let skill_path = if src.is_dir() {
        src.join(SKILL_FILE)
    } else {
        src.to_path_buf()
    };
    if !skill_path.is_file() {
        return Err(AppError::Invalid(format!(
            "expected SKILL.md at {}",
            skill_path.display()
        )));
    }
    let raw = std::fs::read_to_string(&skill_path)
        .map_err(|e| AppError::Other(format!("read {}: {e}", skill_path.display())))?;
    let parsed = parse_skill_md(&raw, "user", "")?;
    let dest_dir = skills_dir(app)?.join(&parsed.id);
    std::fs::create_dir_all(&dest_dir)
        .map_err(|e| AppError::Other(format!("create {}: {e}", dest_dir.display())))?;
    let dest = dest_dir.join(SKILL_FILE);
    std::fs::write(&dest, &raw)
        .map_err(|e| AppError::Other(format!("write {}: {e}", dest.display())))?;
    // Copy sibling icon if present next to source.
    if let Some(parent) = skill_path.parent() {
        for name in ["icon.png", "icon.svg", "icon.jpg"] {
            let icon = parent.join(name);
            if icon.is_file() {
                let _ = std::fs::copy(&icon, dest_dir.join(name));
            }
        }
    }
    parse_skill_md(&raw, "user", &dest.to_string_lossy())
}

/// Delete a user skill directory. Builtin skills cannot be deleted.
pub fn uninstall_skill(app: &AppHandle, id: &str) -> AppResult<()> {
    let id = id.trim();
    if id.is_empty() {
        return Err(AppError::Invalid("empty skill id".into()));
    }
    // Refuse deleting builtins.
    if load_builtin_skills().iter().any(|s| s.id == id) {
        let user_dir = skills_dir(app)?.join(id);
        if user_dir.is_dir() {
            // User override of builtin — remove override only.
            std::fs::remove_dir_all(&user_dir).map_err(|e| {
                AppError::Other(format!("remove {}: {e}", user_dir.display()))
            })?;
            return Ok(());
        }
        return Err(AppError::Invalid("cannot uninstall builtin skill".into()));
    }
    let dir = skills_dir(app)?.join(id);
    if !dir.is_dir() {
        return Err(AppError::Invalid(format!("skill {id:?} not found")));
    }
    std::fs::remove_dir_all(&dir)
        .map_err(|e| AppError::Other(format!("remove {}: {e}", dir.display())))?;
    Ok(())
}

/// Reveal skills directory in the OS file manager (returns path for frontend).
pub fn skills_dir_path(app: &AppHandle) -> AppResult<String> {
    Ok(skills_dir(app)?.to_string_lossy().to_string())
}
