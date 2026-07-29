use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::ai::agent::exec::engine::ProviderQueryEngine;
use crate::ai::agent::tools::agent_tool::{AgentTool, ChatRequestFactory, SubagentSessionHost};
use crate::ai::agent::{
    self, AgentRegistry, ConsultRolesTool, FileReadTool, FileSnapshotStore,
    FsSessionMemoryExtractor, FsUserContextLoader, NotificationQueue, ProviderEngine,
    RoleStateStore, RoleStateTool, StaticMcpRegistry, TaskStore, ToolPool, UserContextConfig,
};
use crate::ai::{session_log, token_log};
use tauri::Manager;

use crate::data::{db, paths};

mod agents;
mod attachments;
mod backup;
mod clipboard;
mod dto;
mod generation;
mod history;
mod media_cmds;
mod messages;
mod project_fs;
mod project_io;
mod project_rules;
mod projects;
mod reader_paths;
mod role_state;
mod sessions;
mod settings;
mod shell;
mod skills;
mod state;
mod subagent;
mod tokens;
mod transfer;

pub use state::AppState;

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .setup(|app| {
            let handle = app.handle();
            let db_path = paths::db_path(handle)?;
            let pool = db::open_pool(&db_path)?;

            // Build the shared services first.
            let registry = Arc::new(AgentRegistry::with_builtins());
            let task_store = Arc::new(TaskStore::new());
            let mcp = Arc::new(StaticMcpRegistry::new());
            let provider_engine = Arc::new(ProviderEngine::new());
            let user_context = Arc::new(FsUserContextLoader::new(UserContextConfig::from_env()));

            // Pool starts with the built-in filesystem tools. Wrap in
            // `Arc` immediately so we can register `AgentTool` self-
            // referentially below.
            let tools: Arc<ToolPool> = Arc::new(ToolPool::new());
            let file_snapshots = Arc::new(FileSnapshotStore::new());
            tools.register(FileReadTool::new());
            tools.register(crate::ai::agent::tools::list_files::ListFilesTool::new());
            tools.register(crate::ai::agent::tools::grep::GrepTool::new());
            tools.register(crate::ai::agent::tools::edit::FileWriteTool::new(
                file_snapshots.clone(),
            ));
            tools.register(
                crate::ai::agent::tools::edit::FileEditTool::new(file_snapshots.clone())
                    .with_pool(Arc::new(pool.clone())),
            );
            tools.register(crate::ai::agent::tools::create_doc::CreateDocTool::new(
                file_snapshots.clone(),
            ));
            tools.register(crate::ai::agent::tools::delete::DeleteTool::new(
                file_snapshots.clone(),
            ));
            tools.register(crate::ai::agent::tools::bash::BashTool::new());
            tools.register_todo_list(crate::ai::agent::tools::todo::TodoListTool::new());
            let role_states = Arc::new(RoleStateStore::new());
            tools.register(RoleStateTool::new(role_states.clone()));
            let prompt_registry =
                Arc::new(crate::ai::agent::tools::prompt_registry::PromptRegistry::new());
            tools.register(crate::ai::agent::tools::ask_user::AskUserTool::new(
                prompt_registry.clone(),
            ));
            tools.register(crate::ai::agent::tools::web_search::WebSearchTool::new(
                Arc::new(pool.clone()),
            ));
            tools.register(crate::ai::agent::tools::web_fetch::WebFetchTool::new());

            // Build the agent-callable `Agent` tool. The chat factory
            // lets it materialise a sub-agent `ChatRequest` from the
            // current settings on demand and (when
            // `definition.omit_claude_md == false`) attaches CLAUDE.md
            // as a system-reminder.
            let chat_factory: Arc<dyn ChatRequestFactory> =
                Arc::new(subagent::SettingsChatFactory::new(pool.clone(), user_context.clone()));
            // TRPG ConsultRoles needs the same factory + provider engine
            // (ProviderEngine is not on ToolUseContext).
            tools.register(ConsultRolesTool::new(
                provider_engine.clone(),
                role_states.clone(),
                chat_factory.clone(),
                registry.clone(),
                pool.clone(),
            ));
            // Plan-mode aware resolver: in Plan-mode any write tool /
            // mutating Bash invocation is denied at the executor before
            // hitting the tool itself.
            let permission_resolver: Arc<dyn agent::PermissionResolver> = Arc::new(
                crate::ai::agent::core::permission::PlanModeResolver::new(agent::AllowAllResolver),
            );
            let query_engine: Arc<dyn agent::QueryEngine> = Arc::new(ProviderQueryEngine::new(
                provider_engine.clone(),
                permission_resolver,
            ));
            let logs_dir = paths::token_logs_dir()?;
            let token_stats = Arc::new(token_log::TokenStatsRecorder::new(pool.clone()));
            let session_logger = Arc::new(session_log::SessionLogger::new(logs_dir));
            let session_host: Arc<dyn SubagentSessionHost> = Arc::new(subagent::TauriSubagentHost::new(
                app.handle().clone(),
                pool.clone(),
                role_states.clone(),
                file_snapshots.clone(),
                token_stats.clone(),
                session_logger.clone(),
            ));
            let agent_tool = AgentTool::new(
                registry.clone(),
                tools.clone(),
                task_store.clone(),
                query_engine.clone(),
                mcp.clone(),
            )
            .with_chat_factory(chat_factory)
            .with_session_host(session_host);
            tools.register(agent_tool);

            app.manage(Arc::new(AppState {
                pool,
                generation_abort: Mutex::new(HashMap::new()),
                agent_registry: registry,
                task_store,
                notifications: Arc::new(NotificationQueue::new()),
                engine: provider_engine,
                query_engine,
                user_context,
                mcp,
                tools,
                session_memory: Arc::new(FsSessionMemoryExtractor::new()),
                role_states,
                prompt_registry,
                file_snapshots,
                token_stats,
                session_logger,
            }));

            let state = app.state::<Arc<AppState>>().inner().clone();
            backup::spawn_scheduler(app.handle().clone(), state);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            settings::get_settings,
            settings::update_settings,
            settings::get_llm_model_catalog,
            settings::fetch_provider_models,
            settings::web_search,
            shell::get_app_info,
            shell::open_path,
            shell::open_url,
            shell::toggle_devtools,
            clipboard::clipboard_write_text,
            sessions::list_sessions,
            sessions::search_sessions,
            sessions::search_session_hits,
            sessions::create_session,
            sessions::rename_session,
            sessions::update_session_config,
            sessions::set_session_model,
            sessions::set_session_agent_type,
            sessions::set_session_agent_chain,
            projects::set_project_agent_chain,
            sessions::delete_session,
            sessions::load_session,
            sessions::list_message_outline,
            sessions::list_messages_window,
            sessions::load_session_window,
            sessions::list_session_media,
            projects::list_projects,
            projects::create_project,
            projects::rename_project,
            projects::update_project_path,
            projects::delete_project,
            projects::reorder_projects,
            projects::assign_session_to_project,
            projects::update_project_config,
            messages::delete_message,
            messages::update_message_text,
            messages::update_message_images,
            attachments::quote_message_as_attachments,
            attachments::add_attachment_from_path,
            attachments::add_attachment_from_bytes,
            attachments::add_url_attachment,
            attachments::remove_attachment_draft,
            media_cmds::get_image_abs_path,
            generation::commands::cancel_generation,
            agents::answer_ask_user,
            agents::list_agent_tasks,
            agents::cancel_agent_task,
            agents::list_agents,
            agents::get_agent_definition,
            agents::list_custom_agents,
            agents::create_custom_agent,
            agents::update_custom_agent,
            agents::delete_custom_agent,
            agents::refresh_user_context,
            agents::set_mcp_servers,
            agents::list_agent_tools,
            role_state::get_role_states,
            role_state::update_role_state,
            role_state::reorder_role_states,
            role_state::delete_role_state,
            agents::extract_session_memory,
            tokens::get_token_usage_summary,
            tokens::get_token_usage_daily,
            tokens::get_token_usage_by_tool,
            tokens::list_token_usage_events,
            generation::commands::generate_image,
            generation::commands::regenerate_image,
            generation::commands::save_cancelled_message,
            media_cmds::edit_image,
            media_cmds::export_image,
            media_cmds::export_media,
            media_cmds::export_media_zip,
            media_cmds::delete_media,
            transfer::export_projects_archive,
            transfer::export_session_archive,
            transfer::import_archive,
            backup::create_backup,
            backup::restore_backup,
            backup::list_backups,
            backup::get_backup_status,
            project_io::write_project_file,
            project_io::read_project_file,
            project_io::list_pending_diffs,
            project_io::confirm_pending_diff,
            project_io::confirm_all_pending_diffs,
            project_fs::list_project_dir,
            project_fs::create_project_dir,
            project_fs::create_project_file,
            project_fs::rename_project_path,
            project_fs::copy_project_path,
            project_fs::import_external_path_to_project,
            project_fs::write_project_file_bytes,
            project_fs::delete_project_path,
            project_rules::list_project_rules,
            project_rules::set_project_rule_enabled,
            skills::list_skills,
            skills::get_skill,
            skills::set_skill_enabled,
            skills::import_skill,
            skills::uninstall_skill,
            skills::get_skills_dir,
            skills::list_enabled_skills,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
