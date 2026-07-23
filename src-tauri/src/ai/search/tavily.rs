//! Tavily search API backend (`https://api.tavily.com/search`).
//!
//! Auth: API key in the JSON body (`api_key`). Returns a `results` array of
//! `{ title, url, content }`.

use serde_json::{json, Value};

use crate::ai::search::{clean_text, SearchBackend, SearchFuture, SearchHit, SearchQuery};
use crate::error::AppError;

const DEFAULT_ENDPOINT: &str = "https://api.tavily.com/search";

pub struct TavilyBackend {
    api_key: String,
    endpoint: String,
}

impl TavilyBackend {
    pub fn new(api_key: String, endpoint: String) -> Self {
        let endpoint = if endpoint.is_empty() {
            DEFAULT_ENDPOINT.to_string()
        } else {
            endpoint
        };
        Self { api_key, endpoint }
    }
}

impl SearchBackend for TavilyBackend {
    fn name(&self) -> &str {
        "tavily"
    }

    fn search<'a>(
        &'a self,
        client: &'a reqwest::Client,
        query: &'a SearchQuery,
    ) -> SearchFuture<'a> {
        Box::pin(async move {
            let body = json!({
                "api_key": self.api_key,
                "query": query.query,
                "max_results": query.max_results,
                "search_depth": "basic",
            });
            let resp = client
                .post(&self.endpoint)
                .header("Authorization", format!("Bearer {}", self.api_key))
                .json(&body)
                .send()
                .await?;
            let status = resp.status();
            let payload: Value = resp.json().await.map_err(|e| {
                AppError::Http(format!("tavily: failed to decode response: {e}"))
            })?;
            if !status.is_success() {
                let msg = payload
                    .get("error")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown error");
                return Err(AppError::Http(format!(
                    "tavily returned HTTP {}: {msg}",
                    status.as_u16()
                )));
            }
            let mut hits = Vec::new();
            if let Some(items) = payload.get("results").and_then(Value::as_array) {
                for item in items {
                    let title = item
                        .get("title")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    let url = item
                        .get("url")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    if url.is_empty() {
                        continue;
                    }
                    let snippet = item
                        .get("content")
                        .and_then(Value::as_str)
                        .map(clean_text)
                        .unwrap_or_default();
                    let published = item
                        .get("published_date")
                        .and_then(Value::as_str)
                        .map(|s| s.to_string());
                    hits.push(SearchHit {
                        title,
                        url,
                        snippet,
                        published,
                        source: "tavily".into(),
                    });
                }
            }
            Ok(hits)
        })
    }
}
