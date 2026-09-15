use reqwest::StatusCode;
use serde_json::{json, Value};

use crate::ai::chat::{ChatRequest, TextDeltaCallback};
use crate::ai::providers::OPENAI_RESPONSES_SDK;
use crate::ai::{tokens, tokens::TokenUsage};
use crate::error::AppError;

use super::chat::body::{append_openai_assistant_text_turn, build_chat_body};
use super::common::{
    finalize_pending_tool_calls, is_retryable_upstream_message, merge_usage,
    parse_tool_call_arguments, retryable_error_in_json_body, set_streaming,
    should_retry_failed_stream_attempt, sse_event_name_and_data, top_level_error_message,
    upstream_rejects_streaming, without_streaming,
};
use super::openrouter::{
    apply_openrouter_gemini_arg_streaming, is_gemini_model, prepare_openrouter_retry,
    relax_openrouter_provider_routing, step_down_openrouter_reasoning,
    strip_gemini_stream_function_call_config, upstream_rejects_tools,
};
use super::responses::body::build_responses_body;
use super::responses::cache::responses_object_url;
use super::responses::parse::{
    extract_responses_reasoning, extract_responses_text, extract_responses_tool_calls,
};
use super::responses::stream::{
    ensure_responses_event_type, merge_responses_tool_events, responses_stream_reasoning_committed,
    responses_stream_reasoning_delta, responses_stream_text_committed, responses_stream_text_delta,
};

#[test]
fn parse_tool_args_valid_json_passes_through() {
    let v = parse_tool_call_arguments("CreateDoc", r#"{"title":"第二章","content":"正文"}"#);
    assert_eq!(v["title"], "第二章");
    assert_eq!(v["content"], "正文");
}

#[test]
fn parse_tool_args_empty_becomes_object() {
    assert_eq!(parse_tool_call_arguments("CreateDoc", "   "), json!({}));
}

#[test]
fn repair_recovers_raw_newlines_in_strings() {
    let raw = "{\"title\":\"第二章\",\"content\":\"第一行\n\t第二行\"}";
    let v = parse_tool_call_arguments("CreateDoc", raw);
    assert_eq!(v["title"], "第二章");
    assert_eq!(v["content"], "第一行\n\t第二行");
}

#[test]
fn responses_object_url_from_responses_endpoint() {
    assert_eq!(
        responses_object_url(
            "https://ark.cn-beijing.volces.com/api/v3/responses",
            "resp_1"
        )
        .as_deref(),
        Some("https://ark.cn-beijing.volces.com/api/v3/responses/resp_1")
    );
}

#[test]
fn responses_cache_continue_omits_tools_and_instructions() {
    let mut request = ChatRequest {
        provider: crate::ai::chat::ProviderConfig {
            id: "p".into(),
            name: "p".into(),
            sdk: OPENAI_RESPONSES_SDK.into(),
            endpoint: "https://ark.cn-beijing.volces.com/api/v3/responses".into(),
            api_key: "k".into(),
            context_cache_enabled: true,
            safety_threshold: None,
        },
        model: "doubao-seed".into(),
        prompt: "follow up".into(),
        attachments: Vec::new(),
        system_prompt: "you are helpful".into(),
        history: Vec::new(),
        parameters: crate::ai::parameters::factory().build(
            "auto".into(),
            "auto".into(),
            crate::data::settings::ModelParamSettings {
                thinking_enabled: Some(true),
                thinking_effort: Some("high".into()),
                ..Default::default()
            },
        ),
        tools: vec![crate::ai::chat::ToolDefinition {
            name: "Bash".into(),
            description: "run".into(),
            schema: json!({ "type": "object" }),
        }],
        tool_chain: Vec::new(),
        tool_results: Vec::new(),
        pending_assistant_turn: None,
        previous_response_id: Some("resp_prev".into()),
        context_cache_enabled: true,
        context_window: None,
        todo_snapshot: None,
        route_providers: Vec::new(),
    };
    let body = build_responses_body(&request);
    assert_eq!(body["previous_response_id"], "resp_prev");
    assert!(body.get("tools").is_none());
    assert!(body.get("instructions").is_none());
    assert_eq!(body["caching"]["type"], "enabled");
    assert_eq!(body["input"][0]["role"], "user");
    // Ark Responses: thinking.type + reasoning.effort (not Chat Completions fields).
    assert!(body.get("reasoning_effort").is_none());
    assert_eq!(body["thinking"]["type"], "enabled");
    assert_eq!(body["reasoning"]["effort"], "high");
    assert!(body["reasoning"].get("summary").is_none());

    request.previous_response_id = None;
    let head = build_responses_body(&request);
    assert!(head.get("previous_response_id").is_none());
    assert!(head.get("tools").is_some());
    assert!(head.get("instructions").is_none()); // cache head uses system message
    assert_eq!(head["input"][0]["role"], "system");
    assert!(head.get("reasoning_effort").is_none());
    assert_eq!(head["thinking"]["type"], "enabled");
    assert_eq!(head["reasoning"]["effort"], "high");
    assert!(head["reasoning"].get("summary").is_none());
}

#[test]
fn todo_snapshot_is_the_last_input_item() {
    let mut request = ChatRequest {
        provider: crate::ai::chat::ProviderConfig {
            id: "p".into(),
            name: "p".into(),
            sdk: OPENAI_RESPONSES_SDK.into(),
            endpoint: "https://ark.cn-beijing.volces.com/api/v3/responses".into(),
            api_key: "k".into(),
            context_cache_enabled: false,
            safety_threshold: None,
        },
        model: "doubao-seed".into(),
        prompt: "continue".into(),
        attachments: Vec::new(),
        system_prompt: "sys".into(),
        history: Vec::new(),
        parameters: crate::ai::parameters::factory().build(
            "auto".into(),
            "auto".into(),
            crate::data::settings::ModelParamSettings::default(),
        ),
        tools: Vec::new(),
        tool_chain: Vec::new(),
        tool_results: Vec::new(),
        pending_assistant_turn: None,
        previous_response_id: None,
        context_cache_enabled: false,
        context_window: None,
        todo_snapshot: Some("<todolist>\n✔ #1 a [done]\n☐ #2 b [pending]\n</todolist>".into()),
        route_providers: Vec::new(),
    };
    let chat = build_chat_body(&request, false);
    let messages = chat["messages"].as_array().expect("messages");
    let last = messages.last().expect("last");
    assert_eq!(last["role"], "user");
    assert!(last["content"].as_str().unwrap().contains("☐ #2 b"));

    let responses = build_responses_body(&request);
    let input = responses["input"].as_array().expect("input");
    let last = input.last().expect("last");
    assert_eq!(last["role"], "user");
    let text = last["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("✔ #1 a"));

    request.previous_response_id = Some("resp_prev".into());
    request.context_cache_enabled = true;
    request.provider.context_cache_enabled = true;
    let delta = build_responses_body(&request);
    let input = delta["input"].as_array().expect("delta input");
    let last = input.last().expect("last");
    assert_eq!(last["role"], "user");
    let text = last["content"][0]["text"].as_str().unwrap();
    assert!(
        text.contains("☐ #2 b"),
        "cache delta must still carry the live list"
    );
}

#[test]
fn openrouter_route_providers_become_provider_only() {
    let request = ChatRequest {
        provider: crate::ai::chat::ProviderConfig {
            id: "openrouter".into(),
            name: "OpenRouter".into(),
            sdk: "openai".into(),
            endpoint: "https://openrouter.ai/api/v1/chat/completions".into(),
            api_key: "k".into(),
            context_cache_enabled: false,
            safety_threshold: None,
        },
        model: "qwen/qwen3.7-max".into(),
        prompt: "hi".into(),
        attachments: Vec::new(),
        system_prompt: String::new(),
        history: Vec::new(),
        parameters: crate::ai::parameters::factory().build(
            "auto".into(),
            "auto".into(),
            crate::data::settings::ModelParamSettings::default(),
        ),
        tools: Vec::new(),
        tool_chain: Vec::new(),
        tool_results: Vec::new(),
        pending_assistant_turn: None,
        previous_response_id: None,
        context_cache_enabled: false,
        context_window: None,
        todo_snapshot: None,
        route_providers: vec!["alibaba".into(), "together".into()],
    };
    let body = build_chat_body(&request, false);
    assert_eq!(body["provider"]["only"], json!(["alibaba", "together"]));
}

#[test]
fn empty_route_providers_omit_openrouter_provider_field() {
    let request = ChatRequest {
        provider: crate::ai::chat::ProviderConfig {
            id: "openrouter".into(),
            name: "OpenRouter".into(),
            sdk: "openai".into(),
            endpoint: "https://openrouter.ai/api/v1/chat/completions".into(),
            api_key: "k".into(),
            context_cache_enabled: false,
            safety_threshold: None,
        },
        model: "qwen/qwen3.7-max".into(),
        prompt: "hi".into(),
        attachments: Vec::new(),
        system_prompt: String::new(),
        history: Vec::new(),
        parameters: crate::ai::parameters::factory().build(
            "auto".into(),
            "auto".into(),
            crate::data::settings::ModelParamSettings::default(),
        ),
        tools: Vec::new(),
        tool_chain: Vec::new(),
        tool_results: Vec::new(),
        pending_assistant_turn: None,
        previous_response_id: None,
        context_cache_enabled: false,
        context_window: None,
        todo_snapshot: None,
        route_providers: Vec::new(),
    };
    let body = build_chat_body(&request, false);
    assert!(body.get("provider").is_none());
}

#[test]
fn top_level_error_formats_openrouter_idle_timeout() {
    let v = json!({
        "error": {
            "code": 504,
            "message": "Upstream idle timeout exceeded"
        }
    });
    assert_eq!(
        top_level_error_message(&v).as_deref(),
        Some("upstream error 504: Upstream idle timeout exceeded")
    );
    assert!(is_retryable_upstream_message(
        "upstream error 504: Upstream idle timeout exceeded"
    ));
    assert!(retryable_error_in_json_body(&v.to_string()));
}

#[test]
fn choice_error_idle_timeout_is_retryable() {
    let v = json!({
        "choices": [{
            "error": {
                "code": 504,
                "message": "Upstream idle timeout exceeded"
            }
        }]
    });
    assert_eq!(
        top_level_error_message(&v).as_deref(),
        Some("upstream error 504: Upstream idle timeout exceeded")
    );
    assert!(retryable_error_in_json_body(&v.to_string()));
}

#[test]
fn client_errors_are_not_retryable_upstream_messages() {
    let v = json!({ "error": { "code": 400, "message": "bad request" } });
    assert!(!is_retryable_upstream_message(
        "upstream error 400: bad request"
    ));
    assert!(!retryable_error_in_json_body(&v.to_string()));
}

#[test]
fn relax_openrouter_routing_drops_provider_only() {
    let mut body = json!({
        "model": "qwen/qwen3.7-max",
        "provider": { "only": ["alibaba"] }
    });
    relax_openrouter_provider_routing(&mut body);
    assert!(body.get("provider").is_none());
    assert_eq!(body["model"], "qwen/qwen3.7-max");
}

#[test]
fn idle_timeout_retry_steps_high_reasoning_down_to_low() {
    let mut body = json!({
        "model": "google/gemini-3.8-flash",
        "provider": { "only": ["google"] },
        "reasoning": { "effort": "high", "enabled": true }
    });
    prepare_openrouter_retry(
        &mut body,
        "upstream error 504: Upstream idle timeout exceeded",
    );
    assert!(body.get("provider").is_none());
    assert_eq!(body["reasoning"]["effort"], "low");
    assert_eq!(body["reasoning"]["exclude"], false);
}

#[test]
fn idle_timeout_retry_disables_reasoning_when_already_low() {
    let mut body = json!({
        "model": "google/gemini-3.8-flash",
        "reasoning": { "effort": "low", "enabled": true, "exclude": false }
    });
    step_down_openrouter_reasoning(&mut body);
    assert_eq!(body["reasoning"]["effort"], "none");
    assert_eq!(body["reasoning"]["enabled"], false);
}

#[test]
fn idle_timeout_retry_disables_silent_default_thinking() {
    let mut body = json!({ "model": "google/gemini-3.8-flash" });
    prepare_openrouter_retry(&mut body, "Upstream idle timeout exceeded");
    assert_eq!(body["reasoning"]["effort"], "none");
    assert_eq!(body["reasoning"]["enabled"], false);
}

#[test]
fn non_idle_retry_keeps_reasoning_effort() {
    let mut body = json!({
        "reasoning": { "effort": "high", "enabled": true },
        "provider": { "only": ["google"] }
    });
    prepare_openrouter_retry(&mut body, "upstream error 429: too many requests");
    assert!(body.get("provider").is_none());
    assert_eq!(body["reasoning"]["effort"], "high");
}

#[test]
fn idle_timeout_stream_error_retries_before_any_delta() {
    let err = AppError::Upstream("upstream error 504: Upstream idle timeout exceeded".into());
    assert!(should_retry_failed_stream_attempt(&err, 1, false));
    assert!(!should_retry_failed_stream_attempt(&err, 1, true));
    assert!(!should_retry_failed_stream_attempt(&err, 3, false));
}

#[test]
fn route_providers_ignored_off_openrouter() {
    let request = ChatRequest {
        provider: crate::ai::chat::ProviderConfig {
            id: "openai".into(),
            name: "OpenAI".into(),
            sdk: "openai".into(),
            endpoint: "https://api.openai.com/v1/chat/completions".into(),
            api_key: "k".into(),
            context_cache_enabled: false,
            safety_threshold: None,
        },
        model: "gpt-4o".into(),
        prompt: "hi".into(),
        attachments: Vec::new(),
        system_prompt: String::new(),
        history: Vec::new(),
        parameters: crate::ai::parameters::factory().build(
            "auto".into(),
            "auto".into(),
            crate::data::settings::ModelParamSettings::default(),
        ),
        tools: Vec::new(),
        tool_chain: Vec::new(),
        tool_results: Vec::new(),
        pending_assistant_turn: None,
        previous_response_id: None,
        context_cache_enabled: false,
        context_window: None,
        todo_snapshot: None,
        route_providers: vec!["openai".into()],
    };
    let body = build_chat_body(&request, false);
    assert!(body.get("provider").is_none());
}

#[test]
fn responses_reasoning_summary_delta_is_thinking() {
    let v = json!({
        "type": "response.reasoning_summary_text.delta",
        "delta": "先判断意图"
    });
    assert_eq!(
        responses_stream_reasoning_delta(&v).as_deref(),
        Some("先判断意图")
    );
    assert!(responses_stream_text_delta(&v).is_none());
}

#[test]
fn responses_output_text_delta_and_done_are_body() {
    let delta = json!({
        "type": "response.output_text.delta",
        "delta": "你好"
    });
    assert_eq!(responses_stream_text_delta(&delta).as_deref(), Some("你好"));
    assert!(responses_stream_reasoning_delta(&delta).is_none());

    let done = json!({
        "type": "response.output_text.done",
        "text": "你好，世界"
    });
    assert_eq!(
        responses_stream_text_committed(&done).as_deref(),
        Some("你好，世界")
    );

    let item_done = json!({
        "type": "response.output_item.done",
        "item": {
            "type": "message",
            "role": "assistant",
            "content": [{ "type": "output_text", "text": "完整正文" }]
        }
    });
    assert_eq!(
        responses_stream_text_committed(&item_done).as_deref(),
        Some("完整正文")
    );
}

#[test]
fn responses_reasoning_output_item_done_is_committed() {
    let v = json!({
        "type": "response.output_item.done",
        "item": {
            "type": "reasoning",
            "summary": [{ "type": "summary_text", "text": "推理摘要" }]
        }
    });
    assert_eq!(
        responses_stream_reasoning_committed(&v).as_deref(),
        Some("推理摘要")
    );
}

#[test]
fn sse_event_line_fills_missing_type() {
    let raw = "event: response.reasoning_summary_text.delta\ndata: {\"delta\":\"and\"}\n\n";
    let (name, data) = sse_event_name_and_data(raw).unwrap();
    assert_eq!(
        name.as_deref(),
        Some("response.reasoning_summary_text.delta")
    );
    let mut v: Value = serde_json::from_str(&data).unwrap();
    ensure_responses_event_type(&mut v, name.as_deref());
    assert_eq!(responses_stream_reasoning_delta(&v).as_deref(), Some("and"));
}

#[test]
fn responses_extract_keeps_summary_out_of_body() {
    let v = json!({
        "output": [
            {
                "type": "reasoning",
                "summary": [{ "type": "summary_text", "text": "这是思考" }]
            },
            {
                "type": "message",
                "role": "assistant",
                "content": [{ "type": "output_text", "text": "这是正文" }]
            }
        ]
    });
    assert_eq!(extract_responses_reasoning(&v).as_deref(), Some("这是思考"));
    assert_eq!(extract_responses_text(&v).as_deref(), Some("这是正文"));
}

#[test]
fn responses_function_call_arguments_stream_live() {
    use std::sync::{Arc, Mutex};
    let seen: Arc<Mutex<Vec<(String, String, String)>>> = Arc::new(Mutex::new(Vec::new()));
    let seen_cb = seen.clone();
    let cb: TextDeltaCallback = Arc::new(move |d| {
        if let Some(tc) = d.tool_call {
            seen_cb.lock().unwrap().push((tc.id, tc.name, tc.arguments));
        }
    });
    let mut pending = Vec::new();

    merge_responses_tool_events(
        &json!({
            "type": "response.output_item.added",
            "output_index": 0,
            "item": {
                "id": "fc_1",
                "type": "function_call",
                "call_id": "call_1",
                "name": "CreateDoc",
                "arguments": "",
                "status": "in_progress"
            }
        }),
        &mut pending,
        &cb,
    );
    merge_responses_tool_events(
        &json!({
            "type": "response.function_call_arguments.delta",
            "item_id": "fc_1",
            "output_index": 0,
            "delta": "{\"title\":\""
        }),
        &mut pending,
        &cb,
    );
    merge_responses_tool_events(
        &json!({
            "type": "response.function_call_arguments.delta",
            "item_id": "fc_1",
            "output_index": 0,
            "delta": "第二章\"}"
        }),
        &mut pending,
        &cb,
    );
    merge_responses_tool_events(
        &json!({
            "type": "response.function_call_arguments.done",
            "item_id": "fc_1",
            "output_index": 0,
            "arguments": "{\"title\":\"第二章\"}"
        }),
        &mut pending,
        &cb,
    );

    let calls = finalize_pending_tool_calls(pending);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].id, "call_1");
    assert_eq!(calls[0].name, "CreateDoc");
    assert_eq!(calls[0].arguments["title"], "第二章");

    let events = seen.lock().unwrap();
    assert!(events.iter().any(|e| e.0 == "call_1" && e.1 == "CreateDoc"));
    let streamed: String = events
        .iter()
        .filter(|e| e.0 == "call_1")
        .map(|e| e.2.as_str())
        .collect();
    assert!(streamed.contains("第二章"));
}

#[test]
fn extract_responses_function_calls() {
    let v = json!({
        "id": "resp_x",
        "output": [
            {
                "type": "function_call",
                "call_id": "call_1",
                "name": "Bash",
                "arguments": "{\"command\":\"ls\"}"
            }
        ]
    });
    let calls = extract_responses_tool_calls(&v);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].id, "call_1");
    assert_eq!(calls[0].name, "Bash");
    assert_eq!(calls[0].arguments["command"], "ls");
}

#[test]
fn repair_recovers_duplicated_payload() {
    let raw = r#"{"title":"a","content":"b"}{"title":"a","content":"b"}"#;
    let v = parse_tool_call_arguments("CreateDoc", raw);
    assert_eq!(v["title"], "a");
    assert_eq!(v["content"], "b");
}

#[test]
fn repair_recovers_truncated_string_value() {
    let raw = r#"{"title":"第二章","content":"写到一半突然断"#;
    let v = parse_tool_call_arguments("CreateDoc", raw);
    assert_eq!(v["title"], "第二章");
    assert_eq!(v["content"], "写到一半突然断");
}

#[test]
fn repair_recovers_cut_after_colon_and_comma() {
    let after_colon = r#"{"title":"a","content":"#;
    let v = parse_tool_call_arguments("CreateDoc", after_colon);
    assert_eq!(v["title"], "a");
    assert_eq!(v["content"], Value::Null);

    let after_comma = r#"{"title":"a","content":"b","#;
    let v = parse_tool_call_arguments("CreateDoc", after_comma);
    assert_eq!(v["title"], "a");
    assert_eq!(v["content"], "b");
}

#[test]
fn repair_recovers_truncated_mid_key() {
    let raw = r#"{"title":"a","cont"#;
    let v = parse_tool_call_arguments("CreateDoc", raw);
    assert_eq!(v["title"], "a");
}

#[test]
fn repair_neutralizes_invalid_escapes() {
    let raw = r#"{"title":"a\x1","content":"b\u12"}"#;
    let v = parse_tool_call_arguments("CreateDoc", raw);
    assert_eq!(v["title"], "a\\x1");
    assert_eq!(v["content"], "b\\u12");
}

#[test]
fn unrepairable_garbage_falls_back_to_null() {
    assert_eq!(
        parse_tool_call_arguments("CreateDoc", "not json at all"),
        Value::Null
    );
}

#[test]
fn assistant_text_turn_echoes_reasoning_content() {
    let mut messages = Vec::new();
    append_openai_assistant_text_turn(&mut messages, "最终答复", Some("先推理"));
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0]["role"], "assistant");
    assert_eq!(messages[0]["content"], "最终答复");
    assert_eq!(messages[0]["reasoning_content"], "先推理");
}

#[test]
fn assistant_text_turn_thinking_only() {
    let mut messages = Vec::new();
    append_openai_assistant_text_turn(&mut messages, "", Some("只思考"));
    assert_eq!(messages.len(), 1);
    // Empty string (not null): DeepSeek requires content or tool_calls.
    assert_eq!(messages[0]["content"], "");
    assert_eq!(messages[0]["reasoning_content"], "只思考");
}

#[test]
fn assistant_tool_turn_empty_content_uses_empty_string() {
    use super::chat::body::append_openai_assistant_tool_turn;
    use crate::ai::chat::{PendingAssistantTurn, ProviderToolCall};

    let mut messages = Vec::new();
    append_openai_assistant_tool_turn(
        &mut messages,
        &PendingAssistantTurn {
            text: None,
            thinking_content: Some("先推理再调工具".into()),
            tool_calls: vec![ProviderToolCall::new("call_1", "Read", json!({}))],
        },
    );
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0]["content"], "");
    assert!(messages[0]["tool_calls"].as_array().unwrap().len() == 1);
    assert_eq!(messages[0]["reasoning_content"], "先推理再调工具");
}

#[test]
fn assistant_tool_turn_skips_blank_thinking_without_payload() {
    use super::chat::body::append_openai_assistant_tool_turn;
    use crate::ai::chat::PendingAssistantTurn;

    let mut messages = Vec::new();
    append_openai_assistant_tool_turn(
        &mut messages,
        &PendingAssistantTurn {
            text: Some("   ".into()),
            thinking_content: Some("  ".into()),
            tool_calls: vec![],
        },
    );
    assert!(messages.is_empty());
}

/// A turn stopped mid-tool-loop has reasoning but never got to a prose reply.
/// Dropping it for want of visible text erased the whole turn from the next
/// request, so the model could not remember what it had just been doing.
#[test]
fn thinking_only_assistant_history_turn_is_kept() {
    use super::chat::body::history_turn_to_chat_message;
    use crate::ai::chat::HistoryTurn;

    let turn = HistoryTurn {
        role: "assistant".into(),
        text: None,
        images: Vec::new(),
        thinking_content: Some("被中断前的推理".into()),
        timeline: Vec::new(),
    };
    let msg = history_turn_to_chat_message(&turn, true).expect("turn must survive");
    assert_eq!(msg["role"], "assistant");
    // Empty string (not null): DeepSeek requires content or tool_calls.
    assert_eq!(msg["content"], "");
    assert_eq!(msg["reasoning_content"], "被中断前的推理");
}

#[test]
fn blank_assistant_history_turn_without_thinking_is_still_dropped() {
    use super::chat::body::history_turn_to_chat_message;
    use crate::ai::chat::HistoryTurn;

    let turn = HistoryTurn {
        role: "assistant".into(),
        text: Some("   ".into()),
        images: Vec::new(),
        thinking_content: None,
        timeline: Vec::new(),
    };
    assert!(history_turn_to_chat_message(&turn, true).is_none());
}

#[test]
fn set_streaming_requests_include_usage_for_chat() {
    let mut body = json!({ "model": "doubao", "messages": [] });
    set_streaming(&mut body, true);
    assert_eq!(body["stream"], true);
    assert_eq!(body["stream_options"]["include_usage"], true);
}

#[test]
fn set_streaming_can_omit_stream_options() {
    let mut body = json!({ "model": "gpt", "input": [] });
    set_streaming(&mut body, false);
    assert_eq!(body["stream"], true);
    assert!(body.get("stream_options").is_none());
}

#[test]
fn responses_streaming_omits_stream_options() {
    let mut body = json!({ "model": "doubao", "input": [] });
    // Responses path must keep usage via `response.completed`, not chat
    // `stream_options` (rejected by Ark / OpenAI Responses).
    set_streaming(&mut body, false);
    assert_eq!(body["stream"], true);
    assert!(body.get("stream_options").is_none());
}

#[test]
fn upstream_rejects_streaming_ignores_stream_options_param_errors() {
    assert!(!upstream_rejects_streaming(
        StatusCode::BAD_REQUEST,
        "Unknown parameter: 'stream_options'",
    ));
    assert!(upstream_rejects_streaming(
        StatusCode::BAD_REQUEST,
        "streaming is not supported for this model",
    ));
}

#[test]
fn without_streaming_strips_stream_options() {
    let body = json!({
        "stream": true,
        "stream_options": { "include_usage": true },
        "model": "x",
    });
    let cleaned = without_streaming(&body);
    assert!(cleaned.get("stream").is_none());
    assert!(cleaned.get("stream_options").is_none());
    assert_eq!(cleaned["model"], "x");
}

#[test]
fn merge_usage_from_final_empty_choices_chunk() {
    let mut usage = TokenUsage::default();
    // Intermediate chunk with usage: null — no overwrite.
    merge_usage(
        &mut usage,
        tokens::extract_usage(&json!({
            "choices": [{ "delta": { "content": "hi" } }],
            "usage": null,
        })),
    );
    assert!(usage.prompt_tokens.is_none());

    // Final Ark/OpenAI usage chunk: empty choices + usage object.
    merge_usage(
        &mut usage,
        tokens::extract_usage(&json!({
            "choices": [],
            "usage": {
                "prompt_tokens": 12,
                "completion_tokens": 34,
                "total_tokens": 46
            }
        })),
    );
    assert_eq!(usage.prompt_tokens, Some(12));
    assert_eq!(usage.completion_tokens, Some(34));
    assert_eq!(usage.total_tokens, Some(46));
}

fn openrouter_chat_request(model: &str, with_tools: bool) -> ChatRequest {
    ChatRequest {
        provider: crate::ai::chat::ProviderConfig {
            id: "openrouter".into(),
            name: "OpenRouter".into(),
            sdk: "openai".into(),
            endpoint: "https://openrouter.ai/api/v1/chat/completions".into(),
            api_key: "k".into(),
            context_cache_enabled: false,
            safety_threshold: None,
        },
        model: model.into(),
        prompt: "hi".into(),
        attachments: Vec::new(),
        system_prompt: String::new(),
        history: Vec::new(),
        parameters: crate::ai::parameters::factory().build(
            "auto".into(),
            "auto".into(),
            crate::data::settings::ModelParamSettings::default(),
        ),
        tools: if with_tools {
            vec![crate::ai::chat::ToolDefinition {
                name: "CreateDoc".into(),
                description: "create".into(),
                schema: json!({ "type": "object" }),
            }]
        } else {
            Vec::new()
        },
        tool_chain: Vec::new(),
        tool_results: Vec::new(),
        pending_assistant_turn: None,
        previous_response_id: None,
        context_cache_enabled: false,
        context_window: None,
        todo_snapshot: None,
        route_providers: Vec::new(),
    }
}

#[test]
fn gemini_model_slug_detection() {
    assert!(is_gemini_model("google/gemini-3.8-flash"));
    assert!(is_gemini_model("Gemini-2.5-pro"));
    assert!(!is_gemini_model("anthropic/claude-sonnet-4.6"));
}

#[test]
fn openrouter_gemini_tools_request_stream_function_call_arguments() {
    let request = openrouter_chat_request("google/gemini-3.8-flash", true);
    let mut body = build_chat_body(&request, false);
    set_streaming(&mut body, true);
    apply_openrouter_gemini_arg_streaming(&mut body, &request);
    assert_eq!(
        body["toolConfig"]["functionCallingConfig"]["streamFunctionCallArguments"],
        true
    );
    strip_gemini_stream_function_call_config(&mut body);
    assert!(body.get("toolConfig").is_none());
    assert!(body.get("tools").is_some());
}

#[test]
fn openrouter_non_gemini_or_no_tools_skip_arg_streaming_flag() {
    let gemini_no_tools = openrouter_chat_request("google/gemini-3.8-flash", false);
    let mut body = build_chat_body(&gemini_no_tools, false);
    apply_openrouter_gemini_arg_streaming(&mut body, &gemini_no_tools);
    assert!(body.get("toolConfig").is_none());

    let claude = openrouter_chat_request("anthropic/claude-sonnet-4.6", true);
    let mut body = build_chat_body(&claude, false);
    apply_openrouter_gemini_arg_streaming(&mut body, &claude);
    assert!(body.get("toolConfig").is_none());
}

#[test]
fn stream_function_call_arguments_reject_does_not_strip_tools() {
    let msg = "Unknown name toolConfig.functionCallingConfig.streamFunctionCallArguments";
    assert!(!upstream_rejects_tools(StatusCode::BAD_REQUEST, msg));
    assert!(super::common::message_rejects_gemini_arg_streaming(msg));
    assert!(super::common::upstream_rejects_gemini_arg_streaming(
        StatusCode::BAD_REQUEST,
        msg
    ));
}
