use rusqlite::params;
use serde::{Deserialize, Serialize};
use ulid::Ulid;

use crate::data::db::{now_ms, DbConn};
use crate::data::settings::{validate_model_param_settings, ModelParamSettings, DEFAULT_HISTORY_TURNS};
use crate::error::{AppError, AppResult};

fn decode_llm_params(raw: Option<String>) -> ModelParamSettings {
    raw.and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
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
    pub created_at: i64,
    pub updated_at: i64,
}

fn map_project(r: &rusqlite::Row<'_>) -> rusqlite::Result<Project> {
    let raw: Option<String> = r.get(6)?;
    Ok(Project {
        id: r.get(0)?,
        name: r.get(1)?,
        path: r.get(2)?,
        sort_order: r.get(3)?,
        system_prompt: r.get(4).unwrap_or_default(),
        history_turns: r.get(5).unwrap_or(DEFAULT_HISTORY_TURNS),
        llm_params: decode_llm_params(raw),
        context_window: r.get(7)?,
        created_at: r.get(8)?,
        updated_at: r.get(9)?,
    })
}

const SELECT_COLS: &str =
    "id, name, path, sort_order, system_prompt, history_turns, llm_params, context_window, created_at, updated_at";

pub fn create(conn: &DbConn, name: &str, path: Option<&str>) -> AppResult<Project> {
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
        params![id, name, path, sort_order, now],
    )?;
    Ok(Project {
        id,
        name: name.to_string(),
        path: path.map(|s| s.to_string()),
        sort_order,
        system_prompt: String::new(),
        history_turns: DEFAULT_HISTORY_TURNS,
        llm_params: ModelParamSettings::default(),
        context_window: None,
        created_at: now,
        updated_at: now,
    })
}

pub fn list(conn: &DbConn) -> AppResult<Vec<Project>> {
    let sql = format!(
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
