//! Fetch a single web page and extract readable text.
//!
//! Deliberately lightweight: strips `script`/`style`/`nav`/`footer` noise and
//! collapses the remaining text. Good enough to feed a model a page's gist
//! without pulling in a full readability engine.

use scraper::{Html, Selector};

use crate::ai::search::{build_search_client, clean_text};
use crate::error::{AppError, AppResult};

/// Hard cap on extracted text so a huge page can't blow up a tool result.
const MAX_TEXT_CHARS: usize = 12_000;

pub struct FetchedPage {
    pub url: String,
    pub title: String,
    pub text: String,
    /// True when the text was truncated to [`MAX_TEXT_CHARS`].
    pub truncated: bool,
}

pub async fn fetch_page(url: &str) -> AppResult<FetchedPage> {
    let url = url.trim();
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err(AppError::Invalid(
            "url must be an absolute http(s) URL".into(),
        ));
    }
    let client = build_search_client()?;
    let resp = client
        .get(url)
        .header("Accept-Language", "en-US,en;q=0.9")
        .send()
        .await?;
    let status = resp.status();
    if !status.is_success() {
        return Err(AppError::Http(format!(
            "fetch returned HTTP {}",
            status.as_u16()
        )));
    }
    let body = resp.text().await?;
    let (title, mut text) = extract(&body);
    let truncated = text.chars().count() > MAX_TEXT_CHARS;
    if truncated {
        text = text.chars().take(MAX_TEXT_CHARS).collect();
    }
    Ok(FetchedPage {
        url: url.to_string(),
        title,
        text,
        truncated,
    })
}

fn extract(html: &str) -> (String, String) {
    let doc = Html::parse_document(html);
    let title = Selector::parse("title")
        .ok()
        .and_then(|sel| doc.select(&sel).next().map(|t| clean_text(&t.text().collect::<String>())))
        .unwrap_or_default();

    // Prefer semantic content containers; fall back to <body>.
    let body_sel = Selector::parse("main, article, body").unwrap();
    let drop_sel = Selector::parse("script, style, noscript, template, svg").unwrap();

    let mut chunks: Vec<String> = Vec::new();
    if let Some(root) = doc.select(&body_sel).next() {
        // Collect text but skip descendants of dropped tags. `scraper` has no
        // node removal, so we gather text from block elements and filter.
        let block_sel = Selector::parse("p, li, h1, h2, h3, h4, h5, h6, td, blockquote, pre").unwrap();
        let drop_ids: std::collections::HashSet<_> =
            root.select(&drop_sel).map(|e| e.id()).collect();
        for el in root.select(&block_sel) {
            // Skip if this element is inside a dropped subtree.
            if el
                .ancestors()
                .any(|a| drop_ids.contains(&a.id()))
            {
                continue;
            }
            let t = clean_text(&el.text().collect::<String>());
            if !t.is_empty() {
                chunks.push(t);
            }
        }
    }
    let text = chunks.join("\n");
    (title, text)
}
