use std::sync::Arc;

use serde::Deserialize;

use crate::error::AppError;

use super::state::AppState;

#[derive(Debug, Deserialize)]
pub(crate) struct TokenUsageSummaryArgs {
    #[serde(default)]
    pub(crate) from_ms: Option<i64>,
    #[serde(default)]
    pub(crate) to_ms: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ListTokenUsageEventsArgs {
    #[serde(default)]
    pub(crate) session_id: Option<String>,
    #[serde(default)]
    pub(crate) model: Option<String>,
    #[serde(default)]
    pub(crate) event_kind: Option<String>,
    #[serde(default)]
    pub(crate) from_ms: Option<i64>,
    #[serde(default)]
    pub(crate) to_ms: Option<i64>,
    #[serde(default)]
    pub(crate) limit: Option<i64>,
    #[serde(default)]
    pub(crate) offset: Option<i64>,
}

#[tauri::command]
pub fn get_token_usage_summary(
    state: tauri::State<Arc<AppState>>,
    args: TokenUsageSummaryArgs,
) -> Result<crate::data::token_log::TokenUsageSummary, AppError> {
    let conn = state.conn()?;
    crate::data::token_log::query_summary(&conn, args.from_ms, args.to_ms)
}

#[tauri::command]
pub fn get_token_usage_daily(
    state: tauri::State<Arc<AppState>>,
    args: TokenUsageSummaryArgs,
) -> Result<Vec<crate::data::token_log::DailyUsageRow>, AppError> {
    let conn = state.conn()?;
    crate::data::token_log::query_daily(&conn, args.from_ms, args.to_ms)
}

#[tauri::command]
pub fn get_token_usage_by_tool(
    state: tauri::State<Arc<AppState>>,
    args: TokenUsageSummaryArgs,
) -> Result<Vec<crate::data::token_log::ToolUsageRow>, AppError> {
    let conn = state.conn()?;
    crate::data::token_log::query_by_tool(&conn, args.from_ms, args.to_ms)
}

#[tauri::command]
pub fn list_token_usage_events(
    state: tauri::State<Arc<AppState>>,
    args: ListTokenUsageEventsArgs,
) -> Result<Vec<crate::data::token_log::TokenUsageEvent>, AppError> {
    let conn = state.conn()?;
    crate::data::token_log::list_events(
        &conn,
        &crate::data::token_log::TokenUsageListFilter {
            session_id: args.session_id,
            model: args.model,
            event_kind: args.event_kind,
            from_ms: args.from_ms,
            to_ms: args.to_ms,
            limit: args.limit.unwrap_or(100),
            offset: args.offset.unwrap_or(0),
        },
    )
}
