use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tauri::AppHandle;

use crate::ai::agent::{self, Task, TaskState};
use crate::data::{custom_agents, paths};
use crate::error::AppError;

use super::generation::params::resolve_generation_definition;
use super::state::AppState;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AnswerAskUserItem {
    pub(crate) prompt: String,
    pub(crate) answer: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AnswerAskUserArgs {
    pub(crate) prompt_id: String,
    pub(crate) answer: String,
    #[serde(default)]
    pub(crate) items: Vec<AnswerAskUserItem>,
}

/// Wake a blocked AskUser tool so the in-flight agent loop continues.
#[tauri::command]
pub fn answer_ask_user(
    state: tauri::State<Arc<AppState>>,
    args: AnswerAskUserArgs,
) -> Result<bool, AppError> {
    let answer = crate::ai::agent::tools::prompt_registry::PromptAnswer {
        answer: args.answer,
        items: args
            .items
            .into_iter()
            .map(|i| crate::ai::agent::tools::prompt_registry::PromptAnswerItem {
                prompt: i.prompt,
                answer: i.answer,
            })
            .collect(),
    };
    Ok(state.prompt_registry.answer(&args.prompt_id, answer))
}

// ????????? Agent task commands ?????????

#[tauri::command]
pub fn list_agent_tasks(state: tauri::State<Arc<AppState>>) -> Result<Vec<Task>, AppError> {
    Ok(state.task_store.list())
}

#[tauri::command]
pub fn cancel_agent_task(state: tauri::State<Arc<AppState>>, task_id: String) -> Result<(), AppError> {
    let id = agent::TaskId(task_id);
    state.task_store.set_state(&id, TaskState::Killed);
    // After state transitions, surface the kill to the main loop as a
    // hidden `<task-notification>` for the next request.
    if let Some(slot) = state.task_store.get(&id) {
        if let Ok(t) = slot.lock() {
            if let Some(note) = agent::TaskNotification::from_task(&t) {
                state.notifications.push(agent::Attachment::for_main(
                    agent::AttachmentKind::TaskNotification(note),
                ));
            }
        }
    }
    Ok(())
}

#[derive(Debug, Serialize)]
pub(crate) struct AgentSummary {
    pub(crate) agent_type: String,
    pub(crate) when_to_use: String,
    pub(crate) background: bool,
    pub(crate) tools: Vec<String>,
    pub(crate) disallowed_tools: Vec<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct UserContextSummary {
    pub(crate) file_count: usize,
    pub(crate) rendered_chars: usize,
    pub(crate) files: Vec<UserContextFile>,
}

#[derive(Debug, Serialize)]
pub(crate) struct UserContextFile {
    pub(crate) ty: String,
    pub(crate) path: String,
    pub(crate) conditional: bool,
    pub(crate) path_globs: Option<Vec<String>>,
}

#[tauri::command]
pub fn refresh_user_context(
    state: tauri::State<Arc<AppState>>,
) -> Result<UserContextSummary, AppError> {
    use crate::ai::agent::memory::UserContextLoader;
    state.user_context.invalidate();
    let ctx = state.user_context.load()?;
    Ok(UserContextSummary {
        file_count: ctx.memory_files.len(),
        rendered_chars: ctx.rendered.chars().count(),
        files: ctx
            .memory_files
            .iter()
            .map(|mf| UserContextFile {
                ty: format!("{:?}", mf.ty).to_lowercase(),
                path: mf.path.to_string_lossy().into_owned(),
                conditional: mf.conditional,
                path_globs: mf.path_globs.clone(),
            })
            .collect(),
    })
}

#[tauri::command]
pub fn set_mcp_servers(
    state: tauri::State<Arc<AppState>>,
    servers: Vec<String>,
) -> Result<(), AppError> {
    state.mcp.set(servers);
    Ok(())
}

#[derive(Debug, Serialize)]
pub(crate) struct SessionMemoryInfo {
    pub(crate) session_id: String,
    pub(crate) summary_path: String,
    pub(crate) total_tokens: i64,
}

#[tauri::command]
pub fn list_agent_tools(state: tauri::State<Arc<AppState>>) -> Result<Vec<String>, AppError> {
    let mut names: Vec<String> = state
        .tools
        .all()
        .into_iter()
        .map(|t| t.spec().name.clone())
        .collect();
    names.sort();
    Ok(names)
}

#[tauri::command]
pub fn extract_session_memory(
    state: tauri::State<Arc<AppState>>,
    app: AppHandle,
    session_id: String,
) -> Result<SessionMemoryInfo, AppError> {
    let dir = paths::session_dir(&app, &session_id)?;

    // Use the most recent completed task for this agent_type/session as
    // the source. If nothing matches we still write the default template.
    let latest_task = state
        .task_store
        .list()
        .into_iter()
        .filter(|t| !matches!(t.state, TaskState::Pending | TaskState::Running))
        .max_by_key(|t| t.ended_at_ms.unwrap_or(t.started_at_ms));

    let sm = state
        .session_memory
        .extract_now(&session_id, &dir, latest_task.as_ref())?;

    Ok(SessionMemoryInfo {
        session_id: sm.session_id,
        summary_path: sm.summary_path.to_string_lossy().into_owned(),
        total_tokens: sm.last_usage.total_tokens.unwrap_or(0),
    })
}

#[tauri::command]
pub fn list_agents(state: tauri::State<Arc<AppState>>) -> Result<Vec<AgentSummary>, AppError> {
    let mut out: Vec<AgentSummary> = state
        .agent_registry
        .active()
        .into_values()
        .map(|d| AgentSummary {
            agent_type: d.agent_type,
            when_to_use: d.when_to_use,
            background: d.background,
            tools: d.tools,
            disallowed_tools: d.disallowed_tools,
        })
        .collect();
    out.sort_by(|a, b| a.agent_type.cmp(&b.agent_type));
    Ok(out)
}

/// Full resolved configuration for one agent type, used to pre-fill the
/// per-node config editor with the agent's default values. Resolves built-ins
/// from the registry and custom agents from the DB.
#[derive(Debug, Serialize)]
pub(crate) struct AgentDefinitionInfo {
    pub(crate) agent_type: String,
    pub(crate) when_to_use: String,
    pub(crate) system_prompt: String,
    pub(crate) model: Option<String>,
    pub(crate) tools: Vec<String>,
    pub(crate) disallowed_tools: Vec<String>,
    pub(crate) background: bool,
    pub(crate) passthrough_output: bool,
}

#[tauri::command]
pub fn get_agent_definition(
    state: tauri::State<Arc<AppState>>,
    agent_type: String,
) -> Result<AgentDefinitionInfo, AppError> {
    let d = resolve_generation_definition(&state, &agent_type)?;
    Ok(AgentDefinitionInfo {
        agent_type: d.agent_type,
        when_to_use: d.when_to_use,
        system_prompt: d.system_prompt,
        model: d.model,
        tools: d.tools,
        disallowed_tools: d.disallowed_tools,
        background: d.background,
        passthrough_output: d.passthrough_output,
    })
}

#[tauri::command]
pub fn list_custom_agents(
    state: tauri::State<Arc<AppState>>,
) -> Result<Vec<custom_agents::CustomAgent>, AppError> {
    let conn = state.conn()?;
    custom_agents::list(&conn)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CreateCustomAgentArgs {
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) when_to_use: String,
    #[serde(default)]
    pub(crate) system_prompt: String,
    #[serde(default)]
    pub(crate) model: Option<String>,
    #[serde(default)]
    pub(crate) tools: Vec<String>,
}

#[tauri::command]
pub fn create_custom_agent(
    state: tauri::State<Arc<AppState>>,
    args: CreateCustomAgentArgs,
) -> Result<custom_agents::CustomAgent, AppError> {
    let conn = state.conn()?;
    custom_agents::create(
        &conn,
        &args.name,
        &args.when_to_use,
        &args.system_prompt,
        args.model.as_deref(),
        &args.tools,
    )
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpdateCustomAgentArgs {
    pub(crate) agent_type: String,
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) when_to_use: String,
    #[serde(default)]
    pub(crate) system_prompt: String,
    #[serde(default)]
    pub(crate) model: Option<String>,
    #[serde(default)]
    pub(crate) tools: Vec<String>,
}

#[tauri::command]
pub fn update_custom_agent(
    state: tauri::State<Arc<AppState>>,
    args: UpdateCustomAgentArgs,
) -> Result<custom_agents::CustomAgent, AppError> {
    let conn = state.conn()?;
    custom_agents::update(
        &conn,
        &args.agent_type,
        &args.name,
        &args.when_to_use,
        &args.system_prompt,
        args.model.as_deref(),
        &args.tools,
    )
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DeleteCustomAgentArgs {
    pub(crate) agent_type: String,
}

#[tauri::command]
pub fn delete_custom_agent(
    state: tauri::State<Arc<AppState>>,
    args: DeleteCustomAgentArgs,
) -> Result<(), AppError> {
    let conn = state.conn()?;
    custom_agents::delete(&conn, &args.agent_type)
}
