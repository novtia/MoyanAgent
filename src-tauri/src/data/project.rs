use rusqlite::params;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use ulid::Ulid;
use crate::data::db::{now_ms, DbConn};
use crate::data::paths;
use crate::data::session::{normalize_chain, ChainNode};
use crate::data::settings::{validate_model_param_settings, ModelParamSettings, DEFAULT_HISTORY_TURNS};
use crate::error::{AppError, AppResult};

fn decode_llm_params(raw: Option<String>) -> ModelParamSettings {
    raw.and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// Decode the project-scoped agent flow chain (JSON array of chain nodes, each
/// a bare agent_type string or `{ agent_type, overrides }`). Empty / blank
/// entries are dropped; an empty result is treated as "no chain" (single-agent
/// runs).
fn decode_agent_chain(raw: Option<String>) -> Option<Vec<ChainNode>> {
    let raw = raw?;
    let parsed: Vec<ChainNode> = serde_json::from_str(&raw).ok()?;
    let cleaned = normalize_chain(&parsed);
    if cleaned.is_empty() {
        None
    } else {
        Some(cleaned)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub id: String,
    pub name: String,
    pub path: Option<String>,
    pub sort_order: i64,
    /// Shared system prompt applied to all sessions in this project.
    pub system_prompt: String,
    /// Number of history turns sent to the model for project sessions.
    pub history_turns: i64,
    /// Shared LLM sampling params for project sessions.
    pub llm_params: ModelParamSettings,
    /// Optional context window override (tokens) for project sessions.
    pub context_window: Option<i64>,
    /// Shared agent flow chain applied to every session in this project. `None`
    /// / empty means single-agent runs. All sessions in the project read and
    /// write this one record, so editing the flow on any conversation updates
    /// the whole project. Each node is an agent type plus optional per-node
    /// config overrides (see [`crate::data::session::ChainNode`]).
    pub agent_chain: Option<Vec<ChainNode>>,
    pub created_at: i64,
    pub updated_at: i64,
}

fn map_project(r: &rusqlite::Row<'_>) -> rusqlite::Result<Project> {
    let raw: Option<String> = r.get(6)?;
    let chain_raw: Option<String> = r.get(10)?;
    Ok(Project {
        id: r.get(0)?,
        name: r.get(1)?,
        path: r.get(2)?,
        sort_order: r.get(3)?,
        system_prompt: r.get(4).unwrap_or_default(),
        history_turns: r.get(5).unwrap_or(DEFAULT_HISTORY_TURNS),
        llm_params: decode_llm_params(raw),
        context_window: r.get(7)?,
        agent_chain: decode_agent_chain(chain_raw),
        created_at: r.get(8)?,
        updated_at: r.get(9)?,
    })
}

const SELECT_COLS: &str =
    "id, name, path, sort_order, system_prompt, history_turns, llm_params, context_window, created_at, updated_at, agent_chain";

/// Root directory for auto-created blank projects: `Documents/MoYanAgent/Project`.
fn blank_projects_root() -> AppResult<PathBuf> {
    paths::blank_projects_root()
}

pub fn create(conn: &DbConn, name: &str, path: Option<&str>) -> AppResult<Project> {
    let resolved_path = match path {
        Some(p) if !p.trim().is_empty() => Some(p.trim().to_string()),
        _ => Some(allocate_blank_project_dir(name)?.to_string_lossy().into_owned()),
    };
    let id = Ulid::new().to_string();
    let now = now_ms();
    let sort_order: i64 = conn
        .query_row(
            "SELECT COALESCE(MAX(sort_order), -1) + 1 FROM projects",
            params![],
            |r| r.get(0),
        )
        .unwrap_or(0);
    conn.execute(
        "INSERT INTO projects(id, name, path, sort_order, created_at, updated_at) VALUES(?1,?2,?3,?4,?5,?5)",
        params![id, name, resolved_path.as_deref(), sort_order, now],
    )?;
    Ok(Project {
        id,
        name: name.to_string(),
        path: resolved_path,
        sort_order,
        system_prompt: String::new(),
        history_turns: DEFAULT_HISTORY_TURNS,
        llm_params: ModelParamSettings::default(),
        context_window: None,
        agent_chain: None,
        created_at: now,
        updated_at: now,
    })
}

/// Create a unique subdirectory under [`blank_projects_root`] for a blank project.
fn allocate_blank_project_dir(project_name: &str) -> AppResult<PathBuf> {
    let root = blank_projects_root()?;
    let base = sanitize_folder_name(project_name);
    for candidate in unique_dir_candidates(&root, &base) {
        match std::fs::create_dir(&candidate) {
            Ok(()) => {
                return candidate
                    .canonicalize()
                    .map_err(|e| AppError::Other(format!("canonicalize {}: {e}", candidate.display())));
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => {
                return Err(AppError::Other(format!(
                    "failed to create project directory {}: {e}",
                    candidate.display()
                )));
            }
        }
    }
    Err(AppError::Other(format!(
        "could not allocate a unique directory under {}",
        root.display()
    )))
}

fn sanitize_folder_name(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| match c {
            '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();
    let s = s.trim().trim_end_matches('.').to_string();
    if s.is_empty() {
        "project".to_string()
    } else {
        s
    }
}

fn unique_dir_candidates<'a>(
    root: &'a Path,
    base: &'a str,
) -> impl Iterator<Item = PathBuf> + 'a {
    std::iter::once(root.join(base)).chain((2..).map(move |n| root.join(format!("{base}-{n}"))))
}

pub fn list(conn: &DbConn) -> AppResult<Vec<Project>> {    let sql = format!(
        "SELECT {SELECT_COLS} FROM projects ORDER BY sort_order ASC, created_at ASC"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params![], map_project)?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

pub fn get(conn: &DbConn, id: &str) -> AppResult<Project> {
    let sql = format!("SELECT {SELECT_COLS} FROM projects WHERE id=?1");
    let mut stmt = conn.prepare(&sql)?;
    let mut rows = stmt.query(params![id])?;
    if let Some(row) = rows.next()? {
        Ok(map_project(row)?)
    } else {
        Err(AppError::NotFound(format!("project {id}")))
    }
}

pub fn rename(conn: &DbConn, id: &str, name: &str) -> AppResult<()> {
    let updated = now_ms();
    let n = conn.execute(
        "UPDATE projects SET name=?1, updated_at=?2 WHERE id=?3",
        params![name, updated, id],
    )?;
    if n == 0 {
        return Err(AppError::NotFound(format!("project {id}")));
    }
    Ok(())
}

/// Update a project's working-directory path. An empty / whitespace-only
/// value clears the path (stored as NULL).
pub fn set_path(conn: &DbConn, id: &str, path: Option<&str>) -> AppResult<()> {
    let normalized = match path {
        Some(p) if !p.trim().is_empty() => Some(p.trim().to_string()),
        _ => None,
    };
    let updated = now_ms();
    let n = conn.execute(
        "UPDATE projects SET path=?1, updated_at=?2 WHERE id=?3",
        params![normalized.as_deref(), updated, id],
    )?;
    if n == 0 {
        return Err(AppError::NotFound(format!("project {id}")));
    }
    Ok(())
}

pub fn update_config(
    conn: &DbConn,
    id: &str,
    system_prompt: &str,
    history_turns: i64,
    llm_params: &ModelParamSettings,
    context_window: Option<i64>,
) -> AppResult<()> {
    if history_turns < 0 {
        return Err(AppError::Invalid(
            "history_turns must be non-negative".into(),
        ));
    }
    validate_model_param_settings(llm_params)?;
    let params_json = serde_json::to_string(llm_params)
        .map_err(|e| AppError::Invalid(format!("failed to serialize llm_params: {e}")))?;
    let updated = now_ms();
    let n = conn.execute(
        "UPDATE projects SET system_prompt=?1, history_turns=?2, llm_params=?3, context_window=?4, updated_at=?5 WHERE id=?6",
        params![system_prompt, history_turns, params_json, context_window, updated, id],
    )?;
    if n == 0 {
        return Err(AppError::NotFound(format!("project {id}")));
    }
    Ok(())
}

/// Persist the shared agent flow chain for a project. An empty list clears the
/// chain (sessions fall back to single-agent generation). Because the chain is
/// stored once per project, this immediately affects every session under it.
pub fn set_agent_chain(conn: &DbConn, id: &str, chain: &[ChainNode]) -> AppResult<()> {
    let cleaned = normalize_chain(chain);
    let stored: Option<String> = if cleaned.is_empty() {
        None
    } else {
        Some(
            serde_json::to_string(&cleaned)
                .map_err(|e| AppError::Invalid(format!("failed to serialize agent_chain: {e}")))?,
        )
    };
    let updated = now_ms();
    let n = conn.execute(
        "UPDATE projects SET agent_chain=?1, updated_at=?2 WHERE id=?3",
        params![stored, updated, id],
    )?;
    if n == 0 {
        return Err(AppError::NotFound(format!("project {id}")));
    }
    Ok(())
}

pub fn delete(conn: &DbConn, id: &str) -> AppResult<()> {
    conn.execute("DELETE FROM projects WHERE id=?1", params![id])?;
    Ok(())
}

pub fn reorder(conn: &DbConn, ordered_ids: &[String]) -> AppResult<()> {
    for (i, id) in ordered_ids.iter().enumerate() {
        conn.execute(
            "UPDATE projects SET sort_order=?1 WHERE id=?2",
            params![i as i64, id],
        )?;
    }
    Ok(())
}

pub fn assign_session(conn: &DbConn, session_id: &str, project_id: Option<&str>) -> AppResult<()> {
    let n = conn.execute(
        "UPDATE sessions SET project_id=?1 WHERE id=?2",
        params![project_id, session_id],
    )?;
    if n == 0 {
        return Err(AppError::NotFound(format!("session {session_id}")));
    }
    Ok(())
}
