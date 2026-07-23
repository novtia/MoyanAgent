//! Local (keyless) search backend.
//!
//! Scrapes a public HTML search endpoint and parses the result list with
//! `scraper`. Two engines are supported:
//!
//! - `duckduckgo` (default) — `https://html.duckduckgo.com/html/`
//! - `bing`                  — `https://www.bing.com/search`
//!
//! No API key is required. This is best-effort: HTML layouts change and the
//! engines may rate-limit; the API providers exist for reliability.

use scraper::{Html, Selector};

use crate::ai::search::{clean_text, percent_decode, SearchBackend, SearchFuture, SearchHit, SearchQuery};
use crate::error::{AppError, AppResult};

const DDG_ENDPOINT: &str = "https://html.duckduckgo.com/html/";
const BING_ENDPOINT: &str = "https://www.bing.com/search";

pub struct LocalBackend {
    engine: LocalEngine,
}

#[derive(Clone, Copy, PartialEq)]
enum LocalEngine {
    DuckDuckGo,
    Bing,
}

impl LocalEngine {
    fn endpoint(self) -> &'static str {
        match self {
            LocalEngine::DuckDuckGo => DDG_ENDPOINT,
            LocalEngine::Bing => BING_ENDPOINT,
        }
    }

    fn source(self) -> &'static str {
        match self {
            LocalEngine::DuckDuckGo => "duckduckgo",
            LocalEngine::Bing => "bing",
        }
    }

    /// The other engine, used as an automatic fallback when the selected one
    /// yields nothing (e.g. Bing serving a CAPTCHA/challenge page).
    fn fallback(self) -> LocalEngine {
        match self {
            LocalEngine::DuckDuckGo => LocalEngine::Bing,
            LocalEngine::Bing => LocalEngine::DuckDuckGo,
        }
    }
}

impl LocalBackend {
    pub fn new(engine: &str) -> Self {
        let engine = match engine.trim().to_ascii_lowercase().as_str() {
            "bing" => LocalEngine::Bing,
            _ => LocalEngine::DuckDuckGo,
        };
        Self { engine }
    }
}

impl SearchBackend for LocalBackend {
    fn name(&self) -> &str {
        match self.engine {
            LocalEngine::DuckDuckGo => "duckduckgo",
            LocalEngine::Bing => "bing",
        }
    }

    fn search<'a>(
        &'a self,
        client: &'a reqwest::Client,
        query: &'a SearchQuery,
    ) -> SearchFuture<'a> {
        Box::pin(async move {
            let primary = self.engine;
            let fallback = primary.fallback();
            match fetch_and_parse(client, primary, query).await {
                // Primary succeeded with real hits.
                Ok(hits) if !hits.is_empty() => Ok(hits),
                // Primary reachable but empty (e.g. Bing CAPTCHA page) —
                // try the other engine before giving up.
                Ok(_) => Ok(fetch_and_parse(client, fallback, query)
                    .await
                    .unwrap_or_default()),
                // Primary failed outright — try the fallback, else surface
                // the original error.
                Err(primary_err) => match fetch_and_parse(client, fallback, query).await {
                    Ok(hits) if !hits.is_empty() => Ok(hits),
                    _ => Err(primary_err),
                },
            }
        })
    }
}

/// Fetch one engine's HTML and parse it into hits. Never falls back — the
/// caller orchestrates cross-engine fallback.
async fn fetch_and_parse(
    client: &reqwest::Client,
    engine: LocalEngine,
    query: &SearchQuery,
) -> AppResult<Vec<SearchHit>> {
    let resp = client
        .get(engine.endpoint())
        .query(&[("q", query.query.as_str())])
        .header("Accept-Language", "en-US,en;q=0.9")
        .send()
        .await?;
    if !resp.status().is_success() {
        return Err(AppError::Http(format!(
            "{} returned HTTP {}",
            engine.source(),
            resp.status().as_u16()
        )));
    }
    let body = resp.text().await?;
    Ok(match engine {
        LocalEngine::DuckDuckGo => parse_duckduckgo(&body, query.max_results),
        LocalEngine::Bing => parse_bing(&body, query.max_results),
    })
}

fn parse_duckduckgo(html: &str, limit: usize) -> Vec<SearchHit> {
    let doc = Html::parse_document(html);
    let result_sel = Selector::parse("div.result, div.web-result").unwrap();
    let link_sel = Selector::parse("a.result__a").unwrap();
    let snippet_sel = Selector::parse("a.result__snippet, div.result__snippet").unwrap();

    let mut out = Vec::new();
    for node in doc.select(&result_sel) {
        if out.len() >= limit {
            break;
        }
        let Some(link) = node.select(&link_sel).next() else {
            continue;
        };
        let title = clean_text(&link.text().collect::<String>());
        let raw_href = link.value().attr("href").unwrap_or_default();
        let url = normalize_ddg_href(raw_href);
        if url.is_empty() || title.is_empty() {
            continue;
        }
        let snippet = node
            .select(&snippet_sel)
            .next()
            .map(|s| clean_text(&s.text().collect::<String>()))
            .unwrap_or_default();
        out.push(SearchHit {
            title,
            url,
            snippet,
            published: None,
            source: "duckduckgo".into(),
        });
    }
    out
}

/// DuckDuckGo wraps result links in a redirect like
/// `//duckduckgo.com/l/?uddg=<percent-encoded target>&rut=...`. Unwrap the
/// real URL from the `uddg` parameter when present.
fn normalize_ddg_href(href: &str) -> String {
    let href = href.trim();
    if href.is_empty() {
        return String::new();
    }
    let with_scheme = if let Some(stripped) = href.strip_prefix("//") {
        format!("https://{stripped}")
    } else {
        href.to_string()
    };
    if let Some(idx) = with_scheme.find("uddg=") {
        let rest = &with_scheme[idx + "uddg=".len()..];
        let encoded = rest.split('&').next().unwrap_or(rest);
        let decoded = percent_decode(encoded);
        if decoded.starts_with("http") {
            return decoded;
        }
    }
    with_scheme
}

fn parse_bing(html: &str, limit: usize) -> Vec<SearchHit> {
    let doc = Html::parse_document(html);
    let result_sel = Selector::parse("li.b_algo").unwrap();
    let title_sel = Selector::parse("h2 a").unwrap();
    let snippet_sel = Selector::parse("div.b_caption p, p.b_algoSlug, div.b_caption").unwrap();

    let mut out = Vec::new();
    for node in doc.select(&result_sel) {
        if out.len() >= limit {
            break;
        }
        let Some(link) = node.select(&title_sel).next() else {
            continue;
        };
        let title = clean_text(&link.text().collect::<String>());
        let url = link.value().attr("href").unwrap_or_default().to_string();
        if url.is_empty() || title.is_empty() {
            continue;
        }
        let snippet = node
            .select(&snippet_sel)
            .next()
            .map(|s| clean_text(&s.text().collect::<String>()))
            .unwrap_or_default();
        out.push(SearchHit {
            title,
            url,
            snippet,
            published: None,
            source: "bing".into(),
        });
    }
    out
}
