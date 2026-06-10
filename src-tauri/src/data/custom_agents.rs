//! User-defined sub-agents persisted in the `custom_agents` table.
//!
//! These are saved globally (not per-session) so they can be reused across
//! sessions and arranged into per-session agent flow chains
//! (`sessions.agent_chain`). At generation time a [`CustomAgent`] is turned
//! into an [`AgentDefinition`] (see [`CustomAgent::to_definition`]) so the
//! existing agent runtime can drive it like any built-in agent.

use rusqlite::params;
use serde::{Deserialize, Serialize};

use crate::ai::agent::config::definition::{AgentDefinition, AgentSource};
use crate::data::db::{now_ms, DbConn};
use crate::error::{AppError, AppResult};

/// Prefix every user-defined agent id carries. Guarantees custom agents can
/// never shadow a built-in `agent_type` (`general-purpose`, `Plan`, …).
pub const CUSTOM_AGENT_PREFIX: &str = "custom:";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomAgent {
    pub agent_type: String,
    pub name: String,
    pub when_to_use: String,
    pub system_prompt: String,
    pub model: Option<String>,
    /// Explicit list of tool names this agent is allowed to use. An empty list
    /// means "full tool access" (`["*"]`), preserving the legacy behaviour for
    /// rows created before tool selection existed.
    #[serde(default)]
    pub tools: Vec<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

impl CustomAgent {
    /// Materialise this row into an [`AgentDefinition`] the agent runtime
    /// understands. The configured tool allow-list is applied verbatim; an
    /// empty list falls back to full access (`["*"]`). Runs as
    /// `AgentSource::User`.
    pub fn to_definition(&self) -> AgentDefinition {
        let mut d = AgentDefinition::builtin(self.agent_type.clone(), self.system_prompt.clone());
        d.when_to_use = self.when_to_use.clone();
        d.tools = if self.tools.is_empty() {
            vec!["*".into()]
        } else {
            self.tools.clone()
        };
        d.model = self.model.clone();
        d.source = AgentSource::User;
        d
    }
}

/// Serialise a tool allow-list for the `tools` column. `None` (NULL) keeps the
/// column empty for the "all tools" default.
fn tools_to_json(tools: &[String]) -> Option<String> {
    let cleaned: Vec<String> = tools
        .iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    if cleaned.is_empty() {
        None
    } else {
        serde_json::to_string(&cleaned).ok()
    }
}

fn tools_from_json(raw: Option<String>) -> Vec<String> {
    raw.and_then(|s| serde_json::from_str::<Vec<String>>(&s).ok())
        .unwrap_or_default()
}

fn normalize_id(raw: &str) -> AppResult<String> {
    let t = raw.trim();
    if t.is_empty() {
        return Err(AppError::Invalid("agent id must not be empty".into()));
    }
    let id = if t.starts_with(CUSTOM_AGENT_PREFIX) {
        t.to_string()
    } else {
        format!("{CUSTOM_AGENT_PREFIX}{t}")
    };
    Ok(id)
}

fn row_to_agent(row: &rusqlite::Row<'_>) -> rusqlite::Result<CustomAgent> {
    let tools_raw: Option<String> = row.get(5)?;
    Ok(CustomAgent {
        agent_type: row.get(0)?,
        name: row.get(1)?,
        when_to_use: row.get(2)?,
        system_prompt: row.get(3)?,
        model: row.get(4)?,
        tools: tools_from_json(tools_raw),
        created_at: row.get(6)?,
        updated_at: row.get(7)?,
    })
}

pub fn list(conn: &DbConn) -> AppResult<Vec<CustomAgent>> {
    let mut stmt = conn.prepare(
        "SELECT agent_type, name, when_to_use, system_prompt, model, tools, created_at, updated_at
         FROM custom_agents ORDER BY created_at ASC",
    )?;
    let rows = stmt.query_map(params![], row_to_agent)?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

pub fn get(conn: &DbConn, agent_type: &str) -> AppResult<Option<CustomAgent>> {
    let mut stmt = conn.prepare(
        "SELECT agent_type, name, when_to_use, system_prompt, model, tools, created_at, updated_at
         FROM custom_agents WHERE agent_type=?1",
    )?;
    let mut rows = stmt.query(params![agent_type])?;
    if let Some(row) = rows.next()? {
        Ok(Some(row_to_agent(row)?))
    } else {
        Ok(None)
    }
}

pub fn create(
    conn: &DbConn,
    name: &str,
    when_to_use: &str,
    system_prompt: &str,
    model: Option<&str>,
    tools: &[String],
) -> AppResult<CustomAgent> {
    let name = name.trim();
    if name.is_empty() {
        return Err(AppError::Invalid("agent name must not be empty".into()));
    }
    let agent_type = normalize_id(name)?;
    if get(conn, &agent_type)?.is_some() {
        return Err(AppError::Invalid(format!(
            "an agent named \"{name}\" already exists"
        )));
    }
    let model = model.map(str::trim).filter(|s| !s.is_empty());
    let tools_json = tools_to_json(tools);
    let now = now_ms();
    conn.execute(
        "INSERT INTO custom_agents(agent_type, name, when_to_use, system_prompt, model, tools, created_at, updated_at)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?7)",
        params![agent_type, name, when_to_use, system_prompt, model, tools_json, now],
    )?;
    get(conn, &agent_type)?
        .ok_or_else(|| AppError::NotFound(format!("custom agent {agent_type}")))
}

pub fn update(
    conn: &DbConn,
    agent_type: &str,
    name: &str,
    when_to_use: &str,
    system_prompt: &str,
    model: Option<&str>,
    tools: &[String],
) -> AppResult<CustomAgent> {
    let name = name.trim();
    if name.is_empty() {
        return Err(AppError::Invalid("agent name must not be empty".into()));
    }
    let model = model.map(str::trim).filter(|s| !s.is_empty());
    let tools_json = tools_to_json(tools);
    let updated = now_ms();
    let n = conn.execute(
        "UPDATE custom_agents SET name=?1, when_to_use=?2, system_prompt=?3, model=?4, tools=?5, updated_at=?6
         WHERE agent_type=?7",
        params![name, when_to_use, system_prompt, model, tools_json, updated, agent_type],
    )?;
    if n == 0 {
        return Err(AppError::NotFound(format!("custom agent {agent_type}")));
    }
    get(conn, agent_type)?
        .ok_or_else(|| AppError::NotFound(format!("custom agent {agent_type}")))
}

pub fn delete(conn: &DbConn, agent_type: &str) -> AppResult<()> {
    conn.execute(
        "DELETE FROM custom_agents WHERE agent_type=?1",
        params![agent_type],
    )?;
    Ok(())
}
