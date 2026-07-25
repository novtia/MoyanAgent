use crate::ai::agent::config::mcp::McpRegistry;
use crate::data::{custom_agents, db, llm_catalog, project, session, settings};
use crate::error::{AppError, AppResult};

use crate::app::state::AppState;

/// Effective generation parameters for a session.
///
/// If the session belongs to a project, the project's shared parameters are
/// used (system prompt, history turns, LLM params, context window override).
/// Otherwise the session's own parameters apply.
#[allow(dead_code)]
pub(crate) struct EffectiveSessionParams {
    pub system_prompt: String,
    pub history_turns: i64,
    pub llm_params: settings::ModelParamSettings,
    /// Context window override; reserved for future use in compaction / UI hints.
    pub context_window: Option<i64>,
}

pub(crate) fn effective_session_params(conn: &db::DbConn, sess: &session::Session) -> EffectiveSessionParams {
    if let Some(ref pid) = sess.project_id {
        if let Ok(proj) = project::get(conn, pid) {
            // Projects share sampling params across their sessions, but the
            // thinking toggle is a per-session composer control (not part of the
            // project config UI), so it must stay session-owned even here.
            let mut llm = proj.llm_params;
            llm.thinking_enabled = sess.llm_params.thinking_enabled;
            llm.thinking_effort = sess.llm_params.thinking_effort.clone();
            return EffectiveSessionParams {
                system_prompt: proj.system_prompt,
                history_turns: proj.history_turns,
                llm_params: llm,
                context_window: proj.context_window.or(sess.context_window),
            };
        }
    }
    EffectiveSessionParams {
        system_prompt: sess.system_prompt.clone(),
        history_turns: sess.history_turns,
        llm_params: sess.llm_params.clone(),
        context_window: sess.context_window,
    }
}

/// Single, unified source of every parameter a generation call needs.
///
/// Bundles the session's resolved model identity (provider + model) together
/// with its effective sampling / prompt / history config, so all generation
/// entry points take their parameters from one place instead of mixing global
/// settings (for model/provider) with per-session config (for the rest).
pub(crate) struct ResolvedGeneration {
    pub(crate) provider: settings::ModelProvider,
    pub(crate) model: String,
    pub(crate) system_prompt: String,
    pub(crate) history_turns: i64,
    pub(crate) llm_params: settings::ModelParamSettings,
    #[allow(dead_code)]
    pub(crate) context_window: Option<i64>,
}

/// Resolve every generation parameter for a session, sourcing model + provider
/// from the session itself and falling back to the global default only when the
/// session has not been initialised yet. When a fallback happens the resolved
/// identity is written back to the session so it becomes self-owned.
pub(crate) fn resolve_session_generation(
    conn: &db::DbConn,
    settings: &settings::Settings,
    sess: &session::Session,
) -> AppResult<ResolvedGeneration> {
    let eff = effective_session_params(conn, sess);

    // Prefer the session's own provider when it still exists and is enabled;
    // otherwise fall back to the global active provider (new-session default).
    let session_provider = sess.provider_id.as_ref().and_then(|pid| {
        settings
            .model_services
            .iter()
            .find(|p| &p.id == pid && p.enabled)
    });
    let provider = match session_provider {
        Some(p) => p.clone(),
        None => settings::active_provider(settings)
            .cloned()
            .ok_or_else(|| AppError::Config("no enabled model provider configured".into()))?,
    };

    // Prefer the session's own model, else the global default model.
    let model = sess
        .model
        .as_ref()
        .map(|m| m.trim().to_string())
        .filter(|m| !m.is_empty())
        .unwrap_or_else(|| settings.model.trim().to_string());

    // Lazily persist the resolved identity so legacy / uninitialised sessions
    // start owning their own model + provider from now on.
    let provider_changed = sess.provider_id.as_deref() != Some(provider.id.as_str());
    let model_changed = sess.model.as_deref().map(str::trim) != Some(model.as_str());
    if (provider_changed || model_changed) && !model.is_empty() && !provider.id.is_empty() {
        let cw = sess.context_window.or_else(|| {
            llm_catalog::lookup_context_window(conn, &provider.id, &provider.sdk, &model)
                .ok()
                .flatten()
        });
        let _ = session::set_provider_model_and_context(
            conn,
            &sess.id,
            Some(provider.id.as_str()),
            Some(model.as_str()),
            cw,
        );
    }

    Ok(ResolvedGeneration {
        provider,
        model,
        system_prompt: eff.system_prompt,
        history_turns: eff.history_turns,
        llm_params: eff.llm_params,
        context_window: eff.context_window,
    })
}

/// Resolve the agent flow chain that should drive a session's generation.
///
/// Sessions belonging to a project share the project's single agent flow
/// record, so editing the chain on any conversation applies to all of them and
/// new conversations inherit it. Plain (project-less) sessions keep their own
/// per-session chain.
pub(crate) fn effective_agent_chain(
    conn: &db::DbConn,
    sess: &session::Session,
) -> Option<Vec<session::ChainNode>> {
    if let Some(ref pid) = sess.project_id {
        if let Ok(proj) = project::get(conn, pid) {
            return proj.agent_chain;
        }
    }
    sess.agent_chain.clone()
}

/// Resolve the [`AgentDefinition`] that should drive a primary-session
/// generation for `agent_type`. Built-in agents come from the registry
/// (MCP-gated); user-defined agents (`custom:*`) are loaded from the
/// `custom_agents` table on demand.
pub(crate) fn resolve_generation_definition(
    state: &AppState,
    agent_type: &str,
) -> AppResult<crate::ai::agent::AgentDefinition> {
    let mcp_available = state.mcp.available_servers();
    if let Some(d) = state
        .agent_registry
        .filter_by_mcp(&mcp_available)
        .get(agent_type)
        .cloned()
    {
        return Ok(d);
    }
    if agent_type.starts_with(custom_agents::CUSTOM_AGENT_PREFIX) {
        let conn = state.conn()?;
        if let Some(ca) = custom_agents::get(&conn, agent_type)? {
            return Ok(ca.to_definition());
        }
    }
    Err(AppError::Invalid(format!(
        "unknown or MCP-unavailable agent type for main session: {agent_type}"
    )))
}

/// Human-friendly label for an agent flow stage. Custom agents show their
/// stored name; built-ins show the `agent_type` directly.
pub(crate) fn stage_display_name(state: &AppState, agent_type: &str) -> String {
    if agent_type.starts_with(custom_agents::CUSTOM_AGENT_PREFIX) {
        if let Ok(conn) = state.conn() {
            if let Ok(Some(ca)) = custom_agents::get(&conn, agent_type) {
                return ca.name;
            }
        }
    }
    agent_type.to_string()
}
