use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use serde_json::{json, Map, Value};

use crate::ai::chat::{AttachmentBytes, ChatRequest, GenerateResponse, HistoryTurn, ImageResult};
use crate::ai::providers::{ChatProvider, ProviderFuture, GEMINI_SDK};
use crate::ai::tokens::TokenUsage;
use crate::error::{AppError, AppResult};

const UPSTREAM_TIMEOUT_SECS: u64 = 15 * 60;

pub struct GeminiProvider;

impl GeminiProvider {
    pub fn new() -> Self {
        Self
    }
}

impl ChatProvider for GeminiProvider {
    fn sdk(&self) -> &'static str {
        GEMINI_SDK
    }

    fn chat<'a>(&'a self, request: ChatRequest) -> ProviderFuture<'a> {
        Box::pin(async move { generate(request).await })
    }
}

async fn generate(request: ChatRequest) -> AppResult<GenerateResponse> {
    let url = gemini_url(&request.provider.endpoint, &request.model);
    let body = build_body(&request);
    let provider_label = provider_label(&request);

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(UPSTREAM_TIMEOUT_SECS))
        .build()?;

    let resp = client
        .post(url)
        .header("Content-Type", "application/json")
        .header("x-goog-api-key", &request.provider.api_key)
        .json(&body)
        .send()
        .await?;

    let status = resp.status();
    let txt = resp.text().await?;
    if !status.is_success() {
        return Err(AppError::Upstream(format!(
            "{} HTTP {}: {}",
            provider_label,
            status,
            upstream_error_message(&txt)
        )));
    }
    parse_response(&txt)
}

fn provider_label(request: &ChatRequest) -> String {
    if request.provider.name.trim().is_empty() {
        request.provider.id.clone()
    } else {
        format!("{} ({})", request.provider.name, request.provider.id)
    }
}

fn gemini_url(endpoint: &str, model: &str) -> String {
    let endpoint = endpoint.trim();
    if endpoint.contains("{model}") {
        return endpoint.replace("{model}", model.trim());
    }
    if endpoint.ends_with(":generateContent") {
        return endpoint.to_string();
    }
    format!(
        "{}/models/{}:generateContent",
        endpoint.trim_end_matches('/'),
        model.trim()
    )
}

fn build_body(request: &ChatRequest) -> Value {
    let mut contents: Vec<Value> = Vec::new();
    for turn in &request.history {
        if let Some(content) = history_turn_to_content(turn) {
            contents.push(content);
        }
    }
    contents.push(content_from_parts(
        "user",
        Some(&request.prompt),
        &request.attachments,
    ));

    // Thread the prior model functionCall + functionResponse pair so
    // Gemini sees a complete call/response chain.
    if let Some(pending) = &request.pending_assistant_turn {
        let mut parts: Vec<Value> = Vec::new();
        if let Some(text) = pending.text.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            parts.push(json!({ "text": text }));
        }
        for tc in &pending.tool_calls {
            parts.push(json!({
                "functionCall": { "name": tc.name, "args": tc.arguments }
            }));
        }
        if !parts.is_empty() {
            contents.push(json!({ "role": "model", "parts": parts }));
        }
    }
    if !request.tool_results.is_empty() {
        let parts: Vec<Value> = request
            .tool_results
            .iter()
            .map(|tr| {
                // Gemini wants the tool name (not the id) on functionResponse.
                // The id-→name mapping lives in the just-emitted pending turn.
                let name = request
                    .pending_assistant_turn
                    .as_ref()
                    .and_then(|p| p.tool_calls.iter().find(|c| c.id == tr.tool_call_id))
                    .map(|c| c.name.clone())
                    .unwrap_or_else(|| tr.tool_call_id.clone());
                let response = match &tr.content {
                    Value::Object(_) | Value::Array(_) => tr.content.clone(),
                    other => json!({ "result": other }),
                };
                json!({
                    "functionResponse": { "name": name, "response": response }
                })
            })
            .collect();
        contents.push(json!({ "role": "user", "parts": parts }));
    }

    let mut body = json!({ "contents": contents });
    let map = body.as_object_mut().unwrap();

    let sys = request.system_prompt.trim();
    if !sys.is_empty() {
        map.insert(
            "system_instruction".into(),
            json!({ "parts": [{ "text": sys }] }),
        );
    }

    if !request.tools.is_empty() {
        let function_declarations: Vec<Value> = request
            .tools
            .iter()
            .map(|t| {
                json!({
                    "name": t.name,
                    "description": t.description,
                    "parameters": t.schema,
                })
            })
            .collect();
        map.insert(
            "tools".into(),
            json!([{ "functionDeclarations": function_declarations }]),
        );
    }

    let generation_config = generation_config(&request);
    if !generation_config.is_empty() {
        map.insert("generationConfig".into(), Value::Object(generation_config));
    }
    body
}

fn history_turn_to_content(turn: &HistoryTurn) -> Option<Value> {
    let text = turn.text.as_deref();
    if text.map(|s| s.trim().is_empty()).unwrap_or(true) && turn.images.is_empty() {
        return None;
    }
    let role = if turn.role == "assistant" {
        "model"
    } else {
        "user"
    };
    let images: &[AttachmentBytes] = if role == "user" { &turn.images } else { &[] };
    Some(content_from_parts(role, text, images))
}

fn content_from_parts(role: &str, text: Option<&str>, attachments: &[AttachmentBytes]) -> Value {
    let mut parts: Vec<Value> = Vec::new();
    if let Some(text) = text {
        if !text.trim().is_empty() {
            parts.push(json!({ "text": text }));
        }
    }
    for attachment in attachments {
        parts.push(json!({
            "inline_data": {
                "mime_type": attachment.mime.as_str(),
                "data": B64.encode(&attachment.bytes),
            }
        }));
    }
    json!({ "role": role, "parts": parts })
}

fn generation_config(request: &ChatRequest) -> Map<String, Value> {
    let mut out = Map::new();
    if request.model.to_ascii_lowercase().contains("image") {
        out.insert("responseModalities".into(), json!(["TEXT", "IMAGE"]));
        if request.parameters.aspect_ratio != "auto" {
            out.insert(
                "imageConfig".into(),
                json!({ "aspectRatio": request.parameters.aspect_ratio.as_str() }),
            );
        }
    }
    if let Some(v) = request.parameters.model.temperature {
        out.insert("temperature".into(), json!(v));
    }
    if let Some(v) = request.parameters.model.top_p {
        out.insert("topP".into(), json!(v));
    }
    if let Some(v) = request.parameters.model.max_tokens {
        out.insert("maxOutputTokens".into(), json!(v));
    }
    out
}

fn parse_response(txt: &str) -> AppResult<GenerateResponse> {
    if txt.is_empty() {
        return Err(AppError::Upstream(
            "upstream returned an empty response body".into(),
        ));
    }
    let v: Value = serde_json::from_str(txt).map_err(|err| {
        AppError::Upstream(format!(
            "failed to parse upstream JSON response: {}; body_bytes={}",
            err,
            txt.len()
        ))
    })?;
    if let Some(msg) = v.pointer("/error/message").and_then(Value::as_str) {
        return Err(AppError::Upstream(msg.to_string()));
    }

    let mut texts = Vec::new();
    let mut images = Vec::new();
    let mut tool_calls: Vec<crate::ai::chat::ProviderToolCall> = Vec::new();
    let mut counter: u32 = 0;
    if let Some(parts) = v
        .pointer("/candidates/0/content/parts")
        .and_then(Value::as_array)
    {
        for part in parts {
            if let Some(text) = part.get("text").and_then(Value::as_str) {
                let trimmed = text.trim();
                if !trimmed.is_empty() {
                    texts.push(trimmed.to_string());
                }
            }
            if let Some(image) = image_from_part(part) {
                images.push(image);
            }
            if let Some(fc) = part.get("functionCall") {
                let name = fc.get("name").and_then(Value::as_str).unwrap_or("").to_string();
                if name.is_empty() {
                    continue;
                }
                let args = fc.get("args").cloned().unwrap_or(Value::Null);
                // Gemini doesn't supply call ids; synthesise stable ones
                // per-response so tool_result can correlate.
                counter += 1;
                tool_calls.push(crate::ai::chat::ProviderToolCall {
                    id: format!("gemini-{counter}"),
                    name,
                    arguments: args,
                });
            }
        }
    }

    if texts.is_empty() && images.is_empty() && tool_calls.is_empty() {
        return Err(AppError::Upstream(format!(
            "upstream response did not contain generated image, text or tool calls. {}",
            empty_response_details(&v)
        )));
    }

    Ok(GenerateResponse {
        images,
        text: if texts.is_empty() {
            None
        } else {
            Some(texts.join("\n\n"))
        },
        usage: usage(&v),
        tool_calls,
    })
}

fn image_from_part(part: &Value) -> Option<ImageResult> {
    let inline = part.get("inline_data").or_else(|| part.get("inlineData"))?;
    let mime = inline
        .get("mime_type")
        .or_else(|| inline.get("mimeType"))
        .and_then(Value::as_str)
        .unwrap_or("image/png");
    let data = inline.get("data").and_then(Value::as_str)?;
    B64.decode(data.as_bytes()).ok().map(|bytes| ImageResult {
        bytes,
        mime: mime.to_string(),
    })
}

fn usage(v: &Value) -> TokenUsage {
    let usage = v.get("usageMetadata").unwrap_or(&Value::Null);
    TokenUsage {
        prompt_tokens: usage.get("promptTokenCount").and_then(Value::as_i64),
        completion_tokens: usage.get("candidatesTokenCount").and_then(Value::as_i64),
        total_tokens: usage.get("totalTokenCount").and_then(Value::as_i64),
    }
}

fn upstream_error_message(txt: &str) -> String {
    match serde_json::from_str::<Value>(txt) {
        Ok(v) => v
            .pointer("/error/message")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| txt.to_string()),
        Err(_) => txt.to_string(),
    }
}

fn empty_response_details(v: &Value) -> String {
    let mut details = Vec::new();
    if let Some(reason) = v
        .pointer("/candidates/0/finishReason")
        .and_then(Value::as_str)
    {
        details.push(format!("finishReason={reason}"));
    }
    if v.pointer("/candidates/0/content/parts").is_none() {
        details.push("missing candidates[0].content.parts".to_string());
    }
    if details.is_empty() {
        String::new()
    } else {
        format!("details: {}", details.join("; "))
    }
}
