use serde_json::{json, Map, Value};

use crate::ai::chat::{AttachmentBytes, ChatRequest, HistoryTurn, PendingAssistantTurn};

use super::super::common::data_url;

pub(crate) fn responses_cache_active(request: &ChatRequest) -> bool {
    (request.context_cache_enabled || request.provider.context_cache_enabled)
        && crate::ai::parameters::is_volcengine_endpoint(&request.provider.endpoint)
}

pub(crate) fn build_responses_body(request: &ChatRequest) -> Value {
    let cache = responses_cache_active(request);
    let continuing = cache
        && request
            .previous_response_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .is_some();

    let input = if continuing {
        build_responses_delta_input(request)
    } else {
        build_responses_full_input(request, cache)
    };

    let mut body = json!({
        "model": request.model,
        "input": input,
    });
    let map = body.as_object_mut().unwrap();

    if continuing {
        if let Some(prev) = request
            .previous_response_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            map.insert("previous_response_id".into(), Value::String(prev.to_string()));
        }
    } else if !cache {
        // Non-cache mode keeps system prompt in `instructions`.
        let sys = request.system_prompt.trim();
        if !sys.is_empty() {
            map.insert("instructions".into(), Value::String(sys.to_string()));
        }
    }

    // Tools only on chain head (Volcengine Session cache constraint).
    if !continuing && !request.tools.is_empty() {
        let tools: Vec<Value> = request
            .tools
            .iter()
            .map(|t| {
                json!({
                    "type": "function",
                    "name": t.name,
                    "description": t.description,
                    "parameters": t.schema,
                })
            })
            .collect();
        map.insert("tools".into(), Value::Array(tools));
    }

    if cache {
        map.insert("store".into(), Value::Bool(true));
        map.insert("caching".into(), json!({ "type": "enabled" }));
        let expire_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64 + 86_400)
            .unwrap_or(0);
        if expire_at > 0 {
            map.insert("expire_at".into(), json!(expire_at));
        }
    }

    apply_responses_params(map, request);
    body
}

/// Full request input: system (cache head) + history + user + in-turn tool rounds.
pub(crate) fn build_responses_full_input(request: &ChatRequest, cache_head: bool) -> Vec<Value> {
    let mut input: Vec<Value> = Vec::new();
    if cache_head {
        let sys = request.system_prompt.trim();
        if !sys.is_empty() {
            input.push(responses_message("system", Some(sys), &[]));
        }
    }
    for turn in &request.history {
        append_responses_history_turn(&mut input, turn);
    }
    input.push(responses_message(
        "user",
        Some(&request.prompt),
        &request.attachments,
    ));
    for round in &request.tool_chain {
        append_responses_tool_round(&mut input, round);
    }
    if let Some(pending) = &request.pending_assistant_turn {
        append_responses_pending_assistant(&mut input, pending);
    }
    append_responses_tool_results(&mut input, &request.tool_results);
    input
}

/// Cache continuation: only the latest tool outputs, or the new user turn.
pub(crate) fn build_responses_delta_input(request: &ChatRequest) -> Vec<Value> {
    let mut input: Vec<Value> = Vec::new();
    if let Some(pending) = &request.pending_assistant_turn {
        // Should be rare (engine commits before the next call); keep for safety.
        append_responses_tool_results(&mut input, &request.tool_results);
        if input.is_empty() && !pending.tool_calls.is_empty() {
            append_responses_pending_assistant(&mut input, pending);
            append_responses_tool_results(&mut input, &request.tool_results);
        }
        if !input.is_empty() {
            return input;
        }
    }
    if let Some(round) = request.tool_chain.last() {
        append_responses_tool_results(&mut input, &round.results);
        if !input.is_empty() {
            return input;
        }
    }
    // New user turn continuing a prior session cache chain.
    input.push(responses_message(
        "user",
        Some(&request.prompt),
        &request.attachments,
    ));
    input
}

pub(crate) fn append_responses_history_turn(input: &mut Vec<Value>, turn: &HistoryTurn) {
    if turn.role == "assistant" && !turn.timeline.is_empty() {
        for seg in &turn.timeline {
            match seg {
                crate::ai::chat::TimelineSegment::Text { text, .. } => {
                    if !text.trim().is_empty() {
                        input.push(responses_message("assistant", Some(text), &[]));
                    }
                }
                crate::ai::chat::TimelineSegment::ToolRound { .. } => {
                    if let Some(round) = seg.to_tool_round() {
                        append_responses_tool_round(input, &round);
                    }
                }
                crate::ai::chat::TimelineSegment::AgentStage { .. } => {}
            }
        }
        return;
    }
    if let Some(message) = history_turn_to_responses_message(turn) {
        input.push(message);
    }
}

pub(crate) fn append_responses_tool_round(input: &mut Vec<Value>, round: &crate::ai::chat::ToolChainRound) {
    append_responses_pending_assistant(input, &round.assistant);
    append_responses_tool_results(input, &round.results);
}

pub(crate) fn append_responses_pending_assistant(input: &mut Vec<Value>, pending: &PendingAssistantTurn) {
    if let Some(text) = pending.text.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        input.push(responses_message("assistant", Some(text), &[]));
    }
    for call in &pending.tool_calls {
        input.push(json!({
            "type": "function_call",
            "call_id": call.id,
            "name": call.name,
            "arguments": serde_json::to_string(&call.arguments).unwrap_or_else(|_| "{}".into()),
        }));
    }
}

pub(crate) fn append_responses_tool_results(
    input: &mut Vec<Value>,
    tool_results: &[crate::ai::chat::ToolResultMessage],
) {
    for tr in tool_results {
        let output = match &tr.content {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        };
        input.push(json!({
            "type": "function_call_output",
            "call_id": tr.tool_call_id,
            "output": output,
        }));
    }
}
pub(crate) fn history_turn_to_responses_message(turn: &HistoryTurn) -> Option<Value> {
    let text = turn.text.as_deref();
    if text.map(|s| s.trim().is_empty()).unwrap_or(true) && turn.images.is_empty() {
        return None;
    }
    let role = match turn.role.as_str() {
        "assistant" => "assistant",
        "system" => "system",
        _ => "user",
    };
    Some(responses_message(role, text, &turn.images))
}

pub(crate) fn responses_message(role: &str, text: Option<&str>, attachments: &[AttachmentBytes]) -> Value {
    let mut content: Vec<Value> = Vec::new();
    if let Some(text) = text {
        if !text.trim().is_empty() {
            content.push(json!({ "type": "input_text", "text": text }));
        }
    }
    for attachment in attachments {
        content.push(json!({
            "type": "input_image",
            "image_url": data_url(attachment),
            "detail": "auto"
        }));
    }
    json!({
        "type": "message",
        "role": role,
        "content": content,
    })
}

pub(crate) fn apply_responses_params(body: &mut Map<String, Value>, request: &ChatRequest) {
    let params = &request.parameters;
    if let Some(v) = params.model.temperature {
        body.insert("temperature".into(), json!(v));
    }
    if let Some(v) = params.model.top_p {
        body.insert("top_p".into(), json!(v));
    }
    if let Some(v) = params.model.max_tokens {
        body.insert("max_output_tokens".into(), json!(v));
    }
    // Volcengine Ark Responses uses `reasoning.effort` (minimal/low/medium/
    // high/max). Chat Completions fields `thinking` / `reasoning_effort`
    // are rejected on this path.
    if crate::ai::parameters::is_volcengine_endpoint(&request.provider.endpoint) {
        params.apply_volcengine_responses_reasoning(body);
    } else {
        params.apply_thinking_params(body, &request.provider.endpoint);
    }
}
