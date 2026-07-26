use serde_json::{json, Value};

use crate::ai::chat::{
    AttachmentBytes, ChatRequest, HistoryTurn, PendingAssistantTurn, ToolResultMessage,
};

use super::super::common::data_url;
use super::super::openrouter::{is_openrouter_endpoint, openrouter_wants_image_output};

pub(crate) fn build_chat_body(request: &ChatRequest, allow_image_parts: bool) -> Value {
    let user_content = chat_content(
        &request.prompt,
        &request.attachments,
        allow_image_parts,
        true,
    );

    let mut messages: Vec<Value> = Vec::new();
    let sys = request.system_prompt.trim();
    if !sys.is_empty() {
        messages.push(json!({ "role": "system", "content": sys }));
    }
    for turn in &request.history {
        if turn.role == "assistant" && !turn.timeline.is_empty() {
            // Replay the prior assistant turn segment-by-segment so tool
            // history reads as native assistant{tool_calls} + role:"tool"
            // messages rather than a leak-prone plain-text transcript.
            // AgentStage markers are host-only and dropped.
            for seg in &turn.timeline {
                match seg {
                    crate::ai::chat::TimelineSegment::Text {
                        text,
                        thinking_content,
                    } => {
                        append_openai_assistant_text_turn(
                            &mut messages,
                            text,
                            thinking_content.as_deref(),
                        );
                    }
                    crate::ai::chat::TimelineSegment::ToolRound { .. } => {
                        if let Some(round) = seg.to_tool_round() {
                            append_openai_assistant_tool_turn(&mut messages, &round.assistant);
                            append_openai_tool_results(&mut messages, &round.results);
                        }
                    }
                    crate::ai::chat::TimelineSegment::AgentStage { .. } => {}
                }
            }
        } else if let Some(message) = history_turn_to_chat_message(turn, allow_image_parts) {
            messages.push(message);
        }
    }
    messages.push(json!({ "role": "user", "content": user_content }));

    for round in &request.tool_chain {
        append_openai_assistant_tool_turn(&mut messages, &round.assistant);
        append_openai_tool_results(&mut messages, &round.results);
    }
    if let Some(pending) = &request.pending_assistant_turn {
        append_openai_assistant_tool_turn(&mut messages, pending);
    }
    append_openai_tool_results(&mut messages, &request.tool_results);

    let mut body = json!({
        "model": request.model,
        "messages": messages,
    });

    let map = body.as_object_mut().unwrap();
    request.parameters.apply_model_params(map);
    request
        .parameters
        .apply_thinking_params(map, &request.provider.endpoint);
    if is_openrouter_endpoint(&request.provider.endpoint) && openrouter_wants_image_output(request)
    {
        if let Some(image_config) = request.parameters.image_config() {
            map.insert("image_config".into(), image_config);
        }
    }

    // Surface available tools to the model. Image-generation flows
    // leave `tools` empty so the field is omitted.
    if !request.tools.is_empty() {
        let tools: Vec<Value> = request
            .tools
            .iter()
            .map(|t| {
                json!({
                    "type": "function",
                    "function": {
                        "name": t.name,
                        "description": t.description,
                        "parameters": t.schema,
                    }
                })
            })
            .collect();
        map.insert("tools".into(), Value::Array(tools));
    }

    body
}

pub(crate) fn append_openai_assistant_text_turn(
    messages: &mut Vec<Value>,
    text: &str,
    thinking_content: Option<&str>,
) {
    let t = text.trim();
    let thinking = thinking_content.map(str::trim).filter(|s| !s.is_empty());
    if t.is_empty() && thinking.is_none() {
        return;
    }
    let mut msg = json!({ "role": "assistant" });
    let m = msg.as_object_mut().unwrap();
    if !t.is_empty() {
        m.insert("content".into(), Value::String(t.to_string()));
    } else {
        m.insert("content".into(), Value::Null);
    }
    if let Some(thinking) = thinking {
        m.insert("reasoning_content".into(), json!(thinking));
    }
    messages.push(msg);
}

pub(crate) fn append_openai_assistant_tool_turn(messages: &mut Vec<Value>, pending: &PendingAssistantTurn) {
    let text = pending.text.as_deref().unwrap_or("");
    let tool_calls: Vec<Value> = pending
        .tool_calls
        .iter()
        .map(|c| {
            json!({
                "id": c.id,
                "type": "function",
                "function": {
                    "name": c.name,
                    "arguments": serde_json::to_string(&c.arguments).unwrap_or_else(|_| "{}".into()),
                }
            })
        })
        .collect();
    let mut msg = json!({ "role": "assistant" });
    let m = msg.as_object_mut().unwrap();
    if !text.is_empty() {
        m.insert("content".into(), Value::String(text.to_string()));
    } else {
        m.insert("content".into(), Value::Null);
    }
    if !tool_calls.is_empty() {
        m.insert("tool_calls".into(), Value::Array(tool_calls));
    }
    if let Some(t) = pending
        .thinking_content
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        m.insert("reasoning_content".into(), json!(t));
    }
    if !text.is_empty() || !pending.tool_calls.is_empty() || pending.thinking_content.is_some() {
        messages.push(msg);
    }
}

pub(crate) fn append_openai_tool_results(messages: &mut Vec<Value>, tool_results: &[ToolResultMessage]) {
    for tr in tool_results {
        let content = match &tr.content {
            Value::String(s) => Value::String(s.clone()),
            other => Value::String(other.to_string()),
        };
        messages.push(json!({
            "role": "tool",
            "tool_call_id": tr.tool_call_id,
            "content": content,
        }));
    }
}

pub(crate) fn chat_content(
    text: &str,
    attachments: &[AttachmentBytes],
    allow_image_parts: bool,
    include_empty_text: bool,
) -> Value {
    if attachments.is_empty() || !allow_image_parts {
        return Value::String(text.to_string());
    }

    let mut arr: Vec<Value> = Vec::with_capacity(attachments.len() + 1);
    if include_empty_text || !text.trim().is_empty() {
        arr.push(json!({"type":"text","text":text}));
    }
    for attachment in attachments {
        arr.push(json!({
            "type":"image_url",
            "image_url": { "url": data_url(attachment) }
        }));
    }
    Value::Array(arr)
}

pub(crate) fn history_turn_to_chat_message(turn: &HistoryTurn, allow_image_parts: bool) -> Option<Value> {
    let role = turn.role.trim();
    if role.is_empty() {
        return None;
    }

    let text = turn
        .text
        .as_deref()
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    let role_allows_images = role == "user";
    let content = chat_content(
        &text,
        if role_allows_images {
            &turn.images
        } else {
            &[]
        },
        allow_image_parts,
        false,
    );
    if matches!(&content, Value::String(s) if s.trim().is_empty()) {
        return None;
    }

    let mut msg = json!({ "role": role, "content": content });
    // DeepSeek (and compatible providers) require `reasoning_content` to be
    // echoed back in assistant history turns when the original response
    // included it; omitting it causes a 400 error.
    if role == "assistant" {
        if let Some(t) = turn
            .thinking_content
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            msg.as_object_mut()
                .unwrap()
                .insert("reasoning_content".into(), json!(t));
        }
    }
    Some(msg)
}
