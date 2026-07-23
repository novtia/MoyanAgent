//! Bing Web Search API backend (Azure Cognitive Services).
//!
//! Auth: `Ocp-Apim-Subscription-Key` header. Query via `q`/`count`. Returns
//! `webPages.value[]` of `{ name, url, snippet, dateLastCrawled }`.

use serde_json::Value;

use crate::ai::search::{clean_text, SearchBackend, SearchFuture, SearchHit, SearchQuery};
use crate::error::AppError;

const DEFAULT_ENDPOINT: &str = "https://api.bing.microsoft.com/v7.0/search";

pub struct BingBackend {
    api_key: String,
    endpoint: String,
}

impl BingBackend {
    pub fn new(api_key: String, endpoint: String) -> Self {
        let endpoint = if endpoint.is_empty() {
            DEFAULT_ENDPOINT.to_string()
        } else {
            endpoint
        };
        Self { api_key, endpoint }
    }
}

impl SearchBackend for BingBackend {
    fn name(&self) -> &str {
        "bing-api"
    }

    fn search<'a>(
        &'a self,
        client: &'a reqwest::Client,
        query: &'a SearchQuery,
    ) -> SearchFuture<'a> {
        Box::pin(async move {
            let count = query.max_results.to_string();
            let resp = client
                .get(&self.endpoint)
                .header("Ocp-Apim-Subscription-Key", &self.api_key)
                .query(&[
                    ("q", query.query.as_str()),
                    ("count", count.as_str()),
                    ("textDecorations", "false"),
                    ("responseFilter", "Webpages"),
                ])
                .send()
                .await?;
            let status = resp.status();
            let payload: Value = resp
                .json()
                .await
                .map_err(|e| AppError::Http(format!("bing: failed to decode response: {e}")))?;
            if !status.is_success() {
                let msg = payload
                    .pointer("/error/message")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown error");
                return Err(AppError::Http(format!(
                    "bing returned HTTP {}: {msg}",
                    status.as_u16()
                )));
            }
            let mut hits = Vec::new();
            if let Some(items) = payload
                .pointer("/webPages/value")
                .and_then(Value::as_array)
            {
                for item in items {
                    let url = item
                        .get("url")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    if url.is_empty() {
                        continue;
                    }
                    let title = item
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    let snippet = item
                        .get("snippet")
                        .and_then(Value::as_str)
                        .map(clean_text)
                        .unwrap_or_default();
                    let published = item
                        .get("dateLastCrawled")
                        .and_then(Value::as_str)
                        .map(|s| s.to_string());
                    hits.push(SearchHit {
                        title,
                        url,
                        snippet,
                        published,
                        source: "bing-api".into(),
                    });
                }
            }
            Ok(hits)
        })
    }
}
