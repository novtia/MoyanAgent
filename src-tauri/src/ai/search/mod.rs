//! Web search subsystem.
//!
//! Provides a small, pluggable search abstraction with two flavours of
//! backend:
//!
//! - **local** ([`local`]): the Rust backend scrapes a public search engine
//!   (DuckDuckGo HTML, Bing HTML) directly and parses the result list. No API
//!   key required.
//! - **API providers** ([`tavily`], [`serper`], [`bing`]): call a paid search
//!   API using a configured key.
//!
//! Adding a new provider is intentionally cheap: implement [`SearchBackend`]
//! in a new file and add one arm to [`resolve_backend`].
//!
//! Both the agent tools (`WebSearch` / `WebFetch`) and the manual UI command
//! (`web_search`) funnel through [`run_search`].

use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};

pub mod fetch;
pub mod local;

mod bing;
mod serper;
mod tavily;

/// Total request budget for a single search backend call. Unlike chat
/// providers (which may stream for a long time), a search must be bounded.
const SEARCH_TIMEOUT_SECS: u64 = 20;
const CONNECT_TIMEOUT_SECS: u64 = 15;

/// Browser-like UA so HTML search engines return the normal results page
/// instead of a bot/challenge page.
pub(crate) const USER_AGENT: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
     (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36";

pub const DEFAULT_MAX_RESULTS: i64 = 5;
pub const MAX_RESULTS_CAP: i64 = 20;

/// Configuration for a single API search provider, persisted in settings as a
/// JSON array under `web_search_providers`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebSearchProviderConfig {
    /// Stable id (equals `kind` for the built-in providers).
    pub id: String,
    /// Backend implementation to use: `tavily` | `serper` | `bing`.
    pub kind: String,
    #[serde(default)]
    pub api_key: String,
    /// Optional endpoint override. Empty ⇒ provider default.
    #[serde(default)]
    pub endpoint: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

/// Resolved web-search configuration read from settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebSearchConfig {
    /// Master switch. When false the tools/command refuse to run.
    pub enabled: bool,
    /// Active backend: `local` or an API provider kind (`tavily`/`serper`/`bing`).
    pub backend: String,
    /// Which engine the local scraper uses: `duckduckgo` (default) or `bing`.
    pub local_engine: String,
    /// Default number of hits to return.
    pub max_results: i64,
    /// API provider credentials.
    pub providers: Vec<WebSearchProviderConfig>,
}

impl Default for WebSearchConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            backend: "local".into(),
            local_engine: "duckduckgo".into(),
            max_results: DEFAULT_MAX_RESULTS,
            providers: Vec::new(),
        }
    }
}

impl WebSearchConfig {
    fn provider(&self, kind: &str) -> Option<&WebSearchProviderConfig> {
        self.providers
            .iter()
            .find(|p| p.enabled && (p.kind == kind || p.id == kind))
    }
}

/// A single search request.
#[derive(Debug, Clone)]
pub struct SearchQuery {
    pub query: String,
    pub max_results: usize,
}

/// One result row.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchHit {
    pub title: String,
    pub url: String,
    #[serde(default)]
    pub snippet: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub published: Option<String>,
    /// Which backend produced this hit (`duckduckgo`, `tavily`, ...).
    pub source: String,
}

/// Full outcome of a search call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchOutcome {
    /// The backend that actually served the request.
    pub backend: String,
    pub query: String,
    pub hits: Vec<SearchHit>,
}

pub type SearchFuture<'a> = Pin<Box<dyn Future<Output = AppResult<Vec<SearchHit>>> + Send + 'a>>;

/// A pluggable search backend. Implementations must be `Send + Sync`.
pub trait SearchBackend: Send + Sync {
    /// Short id used as `SearchHit::source` and in the outcome.
    fn name(&self) -> &str;
    fn search<'a>(
        &'a self,
        client: &'a reqwest::Client,
        query: &'a SearchQuery,
    ) -> SearchFuture<'a>;
}

/// Shared HTTP client for search. Bounded (total + connect timeout) and sends
/// a browser UA so HTML engines behave.
pub(crate) fn build_search_client() -> reqwest::Result<reqwest::Client> {
    reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .connect_timeout(Duration::from_secs(CONNECT_TIMEOUT_SECS))
        .timeout(Duration::from_secs(SEARCH_TIMEOUT_SECS))
        .build()
}

/// Pick the concrete backend for a config. Returns an error when the selected
/// API provider is missing/disabled or has no API key.
pub fn resolve_backend(config: &WebSearchConfig) -> AppResult<Box<dyn SearchBackend>> {
    let backend = config.backend.trim().to_ascii_lowercase();
    match backend.as_str() {
        "" | "local" => Ok(Box::new(local::LocalBackend::new(&config.local_engine))),
        kind @ ("tavily" | "serper" | "bing") => {
            let provider = config.provider(kind).ok_or_else(|| {
                AppError::Config(format!(
                    "web search backend `{kind}` is selected but not configured / enabled"
                ))
            })?;
            if provider.api_key.trim().is_empty() {
                return Err(AppError::Config(format!(
                    "web search provider `{kind}` has no API key"
                )));
            }
            let endpoint = provider.endpoint.trim().to_string();
            let api_key = provider.api_key.trim().to_string();
            Ok(match kind {
                "tavily" => Box::new(tavily::TavilyBackend::new(api_key, endpoint)),
                "serper" => Box::new(serper::SerperBackend::new(api_key, endpoint)),
                _ => Box::new(bing::BingBackend::new(api_key, endpoint)),
            })
        }
        other => Err(AppError::Config(format!(
            "unknown web search backend: {other}"
        ))),
    }
}

/// Run a search end-to-end: build the client, resolve the backend, execute and
/// clamp the result count.
pub async fn run_search(config: &WebSearchConfig, query: SearchQuery) -> AppResult<SearchOutcome> {
    if !config.enabled {
        return Err(AppError::Config("web search is disabled in settings".into()));
    }
    let q = query.query.trim();
    if q.is_empty() {
        return Err(AppError::Invalid("query must not be empty".into()));
    }
    let backend = resolve_backend(config)?;
    let client = build_search_client()?;
    let mut hits = backend.search(&client, &query).await?;
    let cap = query.max_results.max(1);
    hits.truncate(cap);
    // Prefer the engine that actually produced results (the local backend may
    // fall back to the other engine), falling back to the configured name.
    let effective_backend = hits
        .first()
        .map(|h| h.source.clone())
        .unwrap_or_else(|| backend.name().to_string());
    Ok(SearchOutcome {
        backend: effective_backend,
        query: query.query,
        hits,
    })
}

/// Clamp a requested result count into `[1, MAX_RESULTS_CAP]`, falling back to
/// the configured default when unset.
pub fn clamp_max_results(requested: Option<i64>, default: i64) -> usize {
    let base = requested.unwrap_or(default);
    base.clamp(1, MAX_RESULTS_CAP) as usize
}

/// Percent-decode a URL-encoded component (used to unwrap DuckDuckGo redirect
/// links). Best-effort: invalid escapes are passed through verbatim.
pub(crate) fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                let hi = hex_val(bytes[i + 1]);
                let lo = hex_val(bytes[i + 2]);
                if let (Some(hi), Some(lo)) = (hi, lo) {
                    out.push(hi << 4 | lo);
                    i += 3;
                    continue;
                }
                out.push(b'%');
                i += 1;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// Collapse runs of whitespace into single spaces and trim. Used to clean
/// snippets/titles harvested from HTML.
pub(crate) fn clean_text(raw: &str) -> String {
    raw.split_whitespace().collect::<Vec<_>>().join(" ")
}
