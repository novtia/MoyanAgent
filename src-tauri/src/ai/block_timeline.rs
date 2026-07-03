//! Reconstruct an ordered timeline from persisted assistant `blocks`
//! (shared by LLM history replay and, potentially, frontend rendering).
//!
//! Our persisted block schema differs slightly from a call/response
//! stream: a tool's result is merged back INTO its `tool_use` block
//! (`output` + `status`/`is_error`) rather than stored as a separate
//! `tool_result` block. This module folds those blocks into
//! [`TimelineSegment`]s: consecutive `tool_use` blocks collapse into one
//! [`TimelineSegment::ToolRound`]; `text` blocks become
//! [`TimelineSegment::Text`]; `agent_stage` markers become
//! [`TimelineSegment::AgentStage`].

use serde_json::{json, Value};

use crate::ai::chat::{TimelineSegment, TimelineToolCall, TimelineToolResult};
use crate::ai::stream_split::strip_leaked_host_tool_log;

pub fn restore_timeline_from_blocks(blocks: &[Value]) -> Vec<TimelineSegment> {
    let mut segments: Vec<TimelineSegment> = Vec::new();
    let mut pending_prefix: Option<String> = None;
    let mut batch_calls: Vec<TimelineToolCall> = Vec::new();
    let mut batch_results: Vec<TimelineToolResult> = Vec::new();

    let flush_tool_batch = |segments: &mut Vec<TimelineSegment>,
                            pending_prefix: &mut Option<String>,
                            batch_calls: &mut Vec<TimelineToolCall>,
                            batch_results: &mut Vec<TimelineToolResult>| {
        if batch_calls.is_empty() {
            return;
        }
        segments.push(TimelineSegment::ToolRound {
            assistant_text: pending_prefix.take(),
            thinking_content: None,
            calls: std::mem::take(batch_calls),
            results: std::mem::take(batch_results),
        });
    };

    for block in blocks {
        match block.get("type").and_then(Value::as_str) {
            Some("agent_stage") => {
                flush_tool_batch(
                    &mut segments,
                    &mut pending_prefix,
                    &mut batch_calls,
                    &mut batch_results,
                );
                let agent_type = block
                    .get("agent_type")
                    .or_else(|| block.get("agentType"))
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let index = block.get("index").and_then(Value::as_u64).unwrap_or(0) as u32;
                segments.push(TimelineSegment::AgentStage { agent_type, index });
            }
            Some("text") => {
                let text = block
                    .get("content")
                    .or_else(|| block.get("text"))
                    .and_then(Value::as_str)
                    .map(strip_leaked_host_tool_log)
                    .unwrap_or_default();
                if text.trim().is_empty() {
                    continue;
                }
                flush_tool_batch(
                    &mut segments,
                    &mut pending_prefix,
                    &mut batch_calls,
                    &mut batch_results,
                );
                pending_prefix = Some(text);
            }
            Some("tool_use") => {
                let id = block
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim()
                    .to_string();
                if id.is_empty() {
                    continue;
                }
                batch_calls.push(TimelineToolCall {
                    id: id.clone(),
                    name: block
                        .get("tool")
                        .and_then(Value::as_str)
                        .unwrap_or("?")
                        .to_string(),
                    arguments: block.get("input").cloned().unwrap_or_else(|| json!({})),
                });
                // Our schema merges the result back into the tool_use block,
                // so read `output`/`is_error` directly. A separate
                // `tool_result` block (legacy / other producers) is used as
                // a fallback if `output` is absent.
                let inline_output = block.get("output").cloned();
                let inline_error = block.get("is_error").and_then(Value::as_bool);
                if let Some(output) = inline_output {
                    batch_results.push(TimelineToolResult {
                        tool_call_id: id,
                        content: output,
                        is_error: inline_error.unwrap_or(false),
                    });
                } else if let Some(result_block) = blocks.iter().find(|b| {
                    b.get("type").and_then(Value::as_str) == Some("tool_result")
                        && b.get("id").and_then(Value::as_str) == Some(id.as_str())
                }) {
                    batch_results.push(TimelineToolResult {
                        tool_call_id: id,
                        content: result_block.get("output").cloned().unwrap_or(Value::Null),
                        is_error: result_block
                            .get("is_error")
                            .and_then(Value::as_bool)
                            .unwrap_or(false),
                    });
                } else {
                    batch_results.push(TimelineToolResult {
                        tool_call_id: id,
                        content: Value::String("（无结果记录）".into()),
                        is_error: false,
                    });
                }
            }
            Some("tool_result") | Some("thinking") => {}
            _ => {}
        }
    }

    flush_tool_batch(
        &mut segments,
        &mut pending_prefix,
        &mut batch_calls,
        &mut batch_results,
    );
    if let Some(text) = pending_prefix.take() {
        let text = strip_leaked_host_tool_log(&text);
        if !text.trim().is_empty() {
            segments.push(TimelineSegment::Text { text });
        }
    }
    segments
}

/// The user-visible reply for a persisted assistant turn: the last
/// `Text` segment of the timeline, cleaned of any leaked host tool log.
pub fn timeline_summary_text(segments: &[TimelineSegment]) -> String {
    segments
        .iter()
        .rev()
        .find_map(|s| match s {
            TimelineSegment::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .map(strip_leaked_host_tool_log)
        .unwrap_or_default()
        .trim()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restores_prefix_tools_then_summary() {
        let blocks = vec![
            json!({"type":"text","content":"先查目录"}),
            json!({"type":"tool_use","id":"c1","tool":"Read","input":{},"output":{},"status":"success"}),
            json!({"type":"text","content":"已完成"}),
        ];
        let segs = restore_timeline_from_blocks(&blocks);
        assert_eq!(segs.len(), 2);
        assert!(matches!(&segs[0], TimelineSegment::ToolRound { .. }));
        assert!(matches!(&segs[1], TimelineSegment::Text { .. }));
    }

    #[test]
    fn tool_use_result_merged_inline() {
        let blocks = vec![
            json!({"type":"tool_use","id":"c1","tool":"Edit","input":{"path":"a.md"},"output":"ok","status":"success"}),
        ];
        let segs = restore_timeline_from_blocks(&blocks);
        assert_eq!(segs.len(), 1);
        match &segs[0] {
            TimelineSegment::ToolRound { calls, results, .. } => {
                assert_eq!(calls.len(), 1);
                assert_eq!(results.len(), 1);
                assert_eq!(results[0].content, json!("ok"));
                assert!(!results[0].is_error);
            }
            _ => panic!("expected ToolRound"),
        }
    }

    #[test]
    fn agent_stage_splits_and_is_preserved() {
        let blocks = vec![
            json!({"type":"agent_stage","agent_type":"general-purpose","name":"general-purpose","index":0}),
            json!({"type":"text","content":"正文"}),
        ];
        let segs = restore_timeline_from_blocks(&blocks);
        assert_eq!(segs.len(), 2);
        assert!(matches!(&segs[0], TimelineSegment::AgentStage { .. }));
    }

    #[test]
    fn summary_takes_last_text() {
        let segs = vec![
            TimelineSegment::Text { text: "第一段".into() },
            TimelineSegment::Text { text: "最终答复".into() },
        ];
        assert_eq!(timeline_summary_text(&segs), "最终答复");
    }
}
