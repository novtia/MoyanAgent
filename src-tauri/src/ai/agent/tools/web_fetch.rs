//! `WebFetch` — download a web page and return its readable text.
//!
//! Companion to `WebSearch`: after finding a result URL, use this to read the
//! page content. Strips scripts/styles and collapses to plain text. Read-only
//! and concurrency-safe.

use serde_json::{json, Value};

use crate::ai::agent::tools::{Tool, ToolFuture, ToolInvocation, ToolResult, ToolSpec};
use crate::ai::search::fetch;
use crate::error::{AppError, AppResult};

const TOOL_NAME: &str = "WebFetch";

pub struct WebFetchTool {
    spec: ToolSpec,
}

impl Default for WebFetchTool {
    fn default() -> Self {
        Self::new()
    }
}

impl WebFetchTool {
    pub fn new() -> Self {
        Self {
            spec: ToolSpec {
                name: TOOL_NAME.to_string(),
                description: "Fetch a web page by URL and return its readable text \
                    content (scripts, styles and markup stripped). Use after \
                    `WebSearch` to read a specific result, or on any absolute \
                    http(s) URL. Long pages are truncated."
                    .to_string(),
                schema: json!({
                    "type": "object",
                    "properties": {
                        "url": {
                            "type": "string",
                            "description": "Absolute http(s) URL of the page to fetch."
                        }
                    },
                    "required": ["url"]
                }),
                read_only: true,
                concurrency_safe: true,
            },
        }
    }
}

impl Tool for WebFetchTool {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    fn validate(&self, input: &Value) -> AppResult<()> {
        let url = input
            .get("url")
            .and_then(Value::as_str)
            .ok_or_else(|| AppError::Invalid(format!("{TOOL_NAME}: `url` must be a string")))?;
        if url.trim().is_empty() {
            return Err(AppError::Invalid(format!(
                "{TOOL_NAME}: `url` must be non-empty"
            )));
        }
        Ok(())
    }

    fn execute<'a>(&'a self, invocation: ToolInvocation<'a>) -> ToolFuture<'a> {
        Box::pin(async move {
            let url = invocation
                .input
                .get("url")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();

            let page = match fetch::fetch_page(&url).await {
                Ok(p) => p,
                Err(e) => return Ok(ToolResult::error(e.to_string())),
            };

            let content = json!({
                "url": page.url,
                "title": page.title,
                "text": page.text,
                "truncated": page.truncated,
            });
            let mut result = ToolResult::ok(content);
            result.metadata = Some(json!({ "kind": "web_fetch", "url": url }));
            Ok(result)
        })
    }
}
