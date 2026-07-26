use std::sync::Arc;

use serde::Deserialize;

use crate::data::{llm_catalog, settings};
use crate::error::AppError;

use super::state::AppState;

#[tauri::command]
pub fn get_settings(state: tauri::State<Arc<AppState>>) -> Result<settings::Settings, AppError> {
    let conn = state.conn()?;
    settings::read(&conn)
}

#[tauri::command]
pub fn update_settings(
    state: tauri::State<Arc<AppState>>,
    patch: settings::SettingsPatch,
) -> Result<settings::Settings, AppError> {
    let conn = state.conn()?;
    settings::apply_patch(&conn, patch)
}

#[tauri::command]
pub fn get_llm_model_catalog(
    state: tauri::State<Arc<AppState>>,
) -> Result<llm_catalog::LlmModelCatalogDto, AppError> {
    let conn = state.conn()?;
    llm_catalog::fetch_for_frontend(&conn)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FetchProviderModelsArgs {
    pub(crate) sdk: String,
    pub(crate) endpoint: String,
    #[serde(default)]
    pub(crate) api_key: String,
}

/// Pull the live model catalog a provider advertises via its `/models`
/// endpoint so the settings dialog can browse and import models (with
/// context / pricing / capability metadata when the upstream provides it).
#[tauri::command]
pub async fn fetch_provider_models(
    args: FetchProviderModelsArgs,
) -> Result<Vec<crate::ai::providers::model_list::RemoteModelInfo>, AppError> {
    crate::ai::providers::model_list::fetch_models(&args.sdk, &args.endpoint, &args.api_key).await
}

/// Run a web search through the configured backend. Used by the manual search
/// UI; the agent uses the `WebSearch` tool which shares the same engine.
#[tauri::command]
pub async fn web_search(
    state: tauri::State<'_, Arc<AppState>>,
    query: String,
    max_results: Option<i64>,
) -> Result<crate::ai::search::SearchOutcome, AppError> {
    let config = {
        let conn = state.conn()?;
        settings::read_web_search_config(&conn)?
    };
    let max_results = crate::ai::search::clamp_max_results(max_results, config.max_results);
    crate::ai::search::run_search(
        &config,
        crate::ai::search::SearchQuery { query, max_results },
    )
    .await
}
