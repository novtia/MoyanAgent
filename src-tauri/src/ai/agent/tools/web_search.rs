//! `WebSearch` — query the web via the configured search backend.
//!
//! Backend selection (local scrape vs. Tavily/Serper/Bing API) is driven by
//! the user's settings, read live on every call so config changes take effect
//! immediately. Read-only and concurrency-safe.

use std::sync::Arc;

use serde_json::{json, Value};

use crate::ai::agent::tools::{Tool, ToolFuture, ToolInvocation, ToolResult, ToolSpec};
use crate::ai::search::{self, SearchQuery};
use crate::data::db::DbPool;
use crate::data::settings;
use crate::error::{AppError, AppResult};

const TOOL_NAME: &str = "WebSearch";

pub struct WebSearchTool {
    spec: ToolSpec,
    pool: Arc<DbPool>,
}

impl WebSearchTool {
    pub fn new(pool: Arc<DbPool>) -> Self {
        Self {
            spec: ToolSpec {
                name: TOOL_NAME.to_string(),
                description: "Search the web and return a ranked list of results \
                    (title, URL, snippet). Use this to find current information, \
                    documentation, news or facts that may not be in your training \
                    data. The backend (local scrape or an API provider) is chosen \
                    from the user's settings. Follow up with `WebFetch` on a result \
                    URL to read the full page."
                    .to_string(),
                schema: json!({
                    "type": "object",
                    "properties": {
                        "query": {
                            "type": "string",
                            "description": "The search query. Be specific; include keywords, versions or dates when relevant."
                        },
                        "max_results": {
                            "type": "integer",
                            "minimum": 1,
                            "maximum": search::MAX_RESULTS_CAP,
                            "description": "Maximum number of results to return. Defaults to the user's configured value."
                        }
                    },
                    "required": ["query"]
                }),
                read_only: true,
                concurrency_safe: true,
            },
            pool,
        }
    }
}

impl Tool for WebSearchTool {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    fn validate(&self, input: &Value) -> AppResult<()> {
        let query = input
            .get("query")
            .and_then(Value::as_str)
            .ok_or_else(|| AppError::Invalid(format!("{TOOL_NAME}: `query` must be a string")))?;
        if query.trim().is_empty() {
            return Err(AppError::Invalid(format!(
                "{TOOL_NAME}: `query` must be non-empty"
            )));
        }
        Ok(())
    }

    fn execute<'a>(&'a self, invocation: ToolInvocation<'a>) -> ToolFuture<'a> {
        Box::pin(async move {
            let query = invocation
                .input
                .get("query")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let requested = invocation
                .input
                .get("max_results")
                .and_then(Value::as_i64);

            let conn = self.pool.get().map_err(AppError::from)?;
            let config = settings::read_web_search_config(&conn)?;
            drop(conn);

            let max_results = search::clamp_max_results(requested, config.max_results);
            let outcome = match search::run_search(
                &config,
                SearchQuery {
                    query: query.clone(),
                    max_results,
                },
            )
            .await
            {
                Ok(o) => o,
                Err(e) => return Ok(ToolResult::error(e.to_string())),
            };

            if outcome.hits.is_empty() {
                return Ok(ToolResult::ok(json!({
                    "backend": outcome.backend,
                    "query": outcome.query,
                    "results": [],
                    "message": "No results found."
                })));
            }

            let results: Vec<Value> = outcome
                .hits
                .iter()
                .enumerate()
                .map(|(i, h)| {
                    json!({
                        "rank": i + 1,
                        "title": h.title,
                        "url": h.url,
                        "snippet": h.snippet,
                        "published": h.published,
                    })
                })
                .collect();

            let content = json!({
                "backend": outcome.backend,
                "query": outcome.query,
                "results": results,
            });
            let mut result = ToolResult::ok(content);
            result.metadata = Some(json!({
                "kind": "web_search",
                "backend": outcome.backend,
                "count": outcome.hits.len(),
            }));
            Ok(result)
        })
    }
}
