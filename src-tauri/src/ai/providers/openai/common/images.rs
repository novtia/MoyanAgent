use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use serde_json::Value;

use crate::ai::chat::{AttachmentBytes, ImageResult};

pub(crate) fn data_url(att: &AttachmentBytes) -> String {
    format!("data:{};base64,{}", att.mime, B64.encode(&att.bytes))
}
pub(crate) fn parse_data_url(url: &str) -> Option<ImageResult> {
    let prefix = "data:";
    if !url.starts_with(prefix) {
        return None;
    }
    let rest = &url[prefix.len()..];
    let comma = rest.find(',')?;
    let header = &rest[..comma];
    let payload = &rest[comma + 1..];
    let mut mime = "image/png".to_string();
    let mut is_b64 = false;
    for part in header.split(';') {
        if part == "base64" {
            is_b64 = true;
        } else if part.starts_with("image/") {
            mime = part.to_string();
        }
    }
    if !is_b64 {
        return None;
    }
    match B64.decode(payload.as_bytes()) {
        Ok(bytes) => Some(ImageResult { bytes, mime }),
        Err(_) => None,
    }
}

pub(crate) fn parse_b64_image(payload: &str, mime: Option<&str>) -> Option<ImageResult> {
    B64.decode(payload.as_bytes())
        .ok()
        .map(|bytes| ImageResult {
            bytes,
            mime: mime.unwrap_or("image/png").to_string(),
        })
}

pub(crate) fn image_url_from_value(value: &Value) -> Option<&str> {
    [
        value.pointer("/image_url/url"),
        value.pointer("/imageUrl/url"),
        value.get("url"),
        value.get("image_url"),
        value.get("imageUrl"),
    ]
    .into_iter()
    .flatten()
    .find_map(Value::as_str)
}

pub(crate) fn image_from_value(value: &Value) -> Option<ImageResult> {
    // Gemini generateContent / OpenRouter-normalized multimodal image parts
    if let Some(inline) = value.get("inline_data").or_else(|| value.get("inlineData")) {
        let mime = inline
            .get("mime_type")
            .or_else(|| inline.get("mimeType"))
            .and_then(Value::as_str);
        if let Some(data) = inline.get("data").and_then(Value::as_str) {
            if let Some(r) = parse_b64_image(data, mime) {
                return Some(r);
            }
        }
    }
    if let Some(u) = image_url_from_value(value) {
        if let Some(r) = parse_data_url(u) {
            return Some(r);
        }
    }
    let mime = value
        .get("mime_type")
        .or_else(|| value.get("mimeType"))
        .and_then(Value::as_str);
    value
        .get("b64_json")
        .or_else(|| value.get("base64"))
        .or_else(|| value.get("data"))
        .or_else(|| value.get("result"))
        .and_then(Value::as_str)
        .and_then(|payload| parse_b64_image(payload, mime))
}

pub(crate) fn extract_images(v: &Value) -> Vec<ImageResult> {
    let mut out = Vec::new();
    let msg = match v.pointer("/choices/0/message") {
        Some(x) => x,
        None => return out,
    };

    if let Some(arr) = msg.get("images").and_then(Value::as_array) {
        for it in arr {
            if let Some(r) = image_from_value(it) {
                out.push(r);
            }
        }
        if !out.is_empty() {
            return out;
        }
    }

    if let Some(arr) = msg.get("content").and_then(Value::as_array) {
        for part in arr {
            if let Some(r) = image_from_value(part) {
                out.push(r);
            }
        }
        if !out.is_empty() {
            return out;
        }
    }

    if let Some(s) = msg.get("content").and_then(Value::as_str) {
        collect_inline_data_urls(s, &mut out);
    }
    out
}

pub(crate) fn collect_response_images(v: &Value, out: &mut Vec<ImageResult>) {
    match v {
        Value::Array(items) => {
            for item in items {
                collect_response_images(item, out);
            }
        }
        Value::Object(map) => {
            if let Some(image) = image_from_value(v) {
                out.push(image);
                return;
            }
            if let Some(s) = map.get("text").and_then(Value::as_str) {
                collect_inline_data_urls(s, out);
            }
            for key in ["output", "content", "images"] {
                if let Some(value) = map.get(key) {
                    collect_response_images(value, out);
                }
            }
        }
        Value::String(s) => collect_inline_data_urls(s, out),
        _ => {}
    }
}

pub(crate) fn collect_inline_data_urls(s: &str, out: &mut Vec<ImageResult>) {
    let needle = "data:image/";
    let mut i = 0;
    while let Some(start) = s[i..].find(needle).map(|p| p + i) {
        let tail = &s[start..];
        let end_rel = tail
            .find(|c: char| c.is_whitespace() || c == '"' || c == ')' || c == '\'')
            .unwrap_or(tail.len());
        let url = &tail[..end_rel];
        if let Some(r) = parse_data_url(url) {
            out.push(r);
        }
        i = start + end_rel.max(1);
        if i >= s.len() {
            break;
        }
    }
}
