use reqwest::StatusCode;
use serde_json::{json, Value};

use crate::ai::chat::{ChatRequest, TextDeltaCallback};
use crate::ai::providers::OPENAI_RESPONSES_SDK;
use crate::ai::{tokens, tokens::TokenUsage};

use super::chat::body::append_openai_assistant_text_turn;
use super::common::{
    finalize_pending_tool_calls, merge_usage, parse_tool_call_arguments, set_streaming,
    sse_event_name_and_data, upstream_rejects_streaming, without_streaming,
};
use super::responses::body::build_responses_body;
use super::responses::cache::responses_object_url;
use super::responses::parse::{
    extract_responses_reasoning, extract_responses_text, extract_responses_tool_calls,
};
use super::responses::stream::{
    ensure_responses_event_type, merge_responses_tool_events,
    responses_stream_reasoning_committed, responses_stream_reasoning_delta,
    responses_stream_text_committed, responses_stream_text_delta,
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
            responses_object_url("https://ark.cn-beijing.volces.com/api/v3/responses", "resp_1")
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
        assert_eq!(name.as_deref(), Some("response.reasoning_summary_text.delta"));
        let mut v: Value = serde_json::from_str(&data).unwrap();
        ensure_responses_event_type(&mut v, name.as_deref());
        assert_eq!(
            responses_stream_reasoning_delta(&v).as_deref(),
            Some("and")
        );
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
                seen_cb
                    .lock()
                    .unwrap()
                    .push((tc.id, tc.name, tc.arguments));
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
        assert_eq!(messages[0]["content"], Value::Null);
        assert_eq!(messages[0]["reasoning_content"], "只思考");
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
