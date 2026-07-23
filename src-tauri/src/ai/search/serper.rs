//! Serper.dev search API backend (`https://google.serper.dev/search`).
//!
//! Auth: `X-API-KEY` header. Returns an `organic` array of
//! `{ title, link, snippet, date }`.

use serde_json::{json, Value};

use crate::ai::search::{clean_text, SearchBackend, SearchFuture, SearchHit, SearchQuery};
use crate::error::AppError;

const DEFAULT_ENDPOINT: &str = "https://google.serper.dev/search";

pub struct SerperBackend {
    api_key: String,
    endpoint: String,
}

impl SerperBackend {
    pub fn new(api_key: String, endpoint: String) -> Self {
        let endpoint = if endpoint.is_empty() {
            DEFAULT_ENDPOINT.to_string()
        } else {
            endpoint
        };
        Self { api_key, endpoint }
    }
}

impl SearchBackend for SerperBackend {
    fn name(&self) -> &str {
        "serper"
    }

    fn search<'a>(
        &'a self,
        client: &'a reqwest::Client,
        query: &'a SearchQuery,
    ) -> SearchFuture<'a> {
        Box::pin(async move {
            let body = json!({
                "q": query.query,
                "num": query.max_results,
            });
            let resp = client
                .post(&self.endpoint)
                .header("X-API-KEY", &self.api_key)
                .json(&body)
                .send()
                .await?;
            let status = resp.status();
            let payload: Value = resp.json().await.map_err(|e| {
                AppError::Http(format!("serper: failed to decode response: {e}"))
            })?;
            if !status.is_success() {
                let msg = payload
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown error");
                return Err(AppError::Http(format!(
                    "serper returned HTTP {}: {msg}",
                    status.as_u16()
                )));
            }
            let mut hits = Vec::new();
            if let Some(items) = payload.get("organic").and_then(Value::as_array) {
                for item in items {
                    let url = item
                        .get("link")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    if url.is_empty() {
                        continue;
                    }
                    let title = item
                        .get("title")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    let snippet = item
                        .get("snippet")
                        .and_then(Value::as_str)
                        .map(clean_text)
                        .unwrap_or_default();
                    let published = item
                        .get("date")
                        .and_then(Value::as_str)
                        .map(|s| s.to_string());
                    hits.push(SearchHit {
                        title,
                        url,
                        snippet,
                        published,
                        source: "serper".into(),
                    });
                }
            }
            Ok(hits)
        })
    }
}
