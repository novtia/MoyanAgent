//! Fetch a single web page and convert it to readable Markdown.
//!
//! Uses `html-to-markdown-rs` with aggressive preprocessing to strip
//! navigation, forms, and other boilerplate before conversion.

use html_to_markdown_rs::{
    convert, ConversionOptions, PreprocessingOptions, PreprocessingPreset,
};

use crate::ai::search::build_search_client;
use crate::error::{AppError, AppResult};

/// Hard cap on extracted text so a huge page can't blow up a tool result.
const MAX_TEXT_CHARS: usize = 24_000;

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
    let (title, mut text) = extract(&body)?;
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

fn extract(html: &str) -> AppResult<(String, String)> {
    let mut options = ConversionOptions::default();
    options.preprocessing = PreprocessingOptions {
        enabled: true,
        preset: PreprocessingPreset::Aggressive,
        remove_navigation: true,
        remove_forms: true,
    };
    options.skip_images = true;
    options.extract_metadata = true;

    let result = convert(html, options).map_err(|e| {
        AppError::Other(format!("html to markdown conversion failed: {e}"))
    })?;

    let title = result
        .metadata
        .document
        .title
        .unwrap_or_default();
    let text = result.content.unwrap_or_default();
    Ok((title, text))
}
