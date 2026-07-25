use std::sync::{Arc, Mutex};

use tauri::{AppHandle, Emitter};

use crate::ai::agent::exec::query::ToolEventCallback;
use crate::ai::agent::types::MessageEvent;
use crate::ai::agent::FileSnapshotStore;
use crate::ai::chat;
use crate::data::{db, session};
use crate::error::AppResult;

use crate::app::history::concat_block_text;

pub(crate) type StreamBlocks = Arc<Mutex<Vec<serde_json::Value>>>;

pub(crate) fn new_stream_blocks() -> StreamBlocks {
    Arc::new(Mutex::new(Vec::new()))
}

pub(crate) fn snapshot_stream_blocks(blocks: &StreamBlocks) -> Vec<serde_json::Value> {
    blocks.lock().ok().map(|g| g.clone()).unwrap_or_default()
}

/// Persist streamed assistant output before the session is reloaded.
///
/// On upstream failure the UI has already rendered deltas from `gen://stream`;
/// without this write, a DB reload would drop the in-flight assistant bubble and
/// leave only the separate `error` row.
pub(crate) fn persist_streamed_assistant_snapshot(
    conn: &db::DbConn,
    session_id: &str,
    blocks: &[serde_json::Value],
    fallback_text: Option<&str>,
    fallback_thinking: Option<&str>,
    mut params: serde_json::Value,
    file_snapshots: &FileSnapshotStore,
    request_message_id: Option<&str>,
) -> AppResult<()> {
    use crate::ai::stream_split::strip_leaked_host_tool_log;

    session::get(conn, session_id)?;

    // Scrub any leaked host tool-log lines out of persisted text blocks so
    // an interrupted / errored partial snapshot can't re-teach the model to
    // echo them on the next turn.
    let cleaned_blocks: Vec<serde_json::Value> = blocks
        .iter()
        .map(|b| {
            if b.get("type").and_then(|v| v.as_str()) == Some("text") {
                if let Some(t) = b.get("content").and_then(|c| c.as_str()) {
                    let mut nb = b.clone();
                    if let Some(obj) = nb.as_object_mut() {
                        obj.insert(
                            "content".into(),
                            serde_json::Value::String(strip_leaked_host_tool_log(t)),
                        );
                    }
                    return nb;
                }
            }
            b.clone()
        })
        .collect();

    let mut text = concat_block_text(&cleaned_blocks, "text");
    if text.trim().is_empty() {
        text = fallback_text
            .map(strip_leaked_host_tool_log)
            .unwrap_or_default()
            .trim()
            .to_string();
    } else {
        text = text.trim().to_string();
    }
    let mut thinking = concat_block_text(&cleaned_blocks, "thinking");
    if thinking.trim().is_empty() {
        thinking = fallback_thinking.unwrap_or("").trim().to_string();
    } else {
        thinking = thinking.trim().to_string();
    }

    let has_blocks = !cleaned_blocks.is_empty();
    if text.is_empty() && thinking.is_empty() && !has_blocks {
        return Ok(());
    }

    if let Some(obj) = params.as_object_mut() {
        if !thinking.is_empty() {
            obj.insert(
                "thinking_content".into(),
                serde_json::Value::String(thinking.clone()),
            );
        }
        if has_blocks {
            let timeline = crate::ai::block_timeline::restore_timeline_from_blocks(&cleaned_blocks);
            if !timeline.is_empty() {
                if let Ok(tv) = serde_json::to_value(&timeline) {
                    obj.insert("timeline".into(), tv);
                }
            }
            obj.insert(
                "blocks".into(),
                serde_json::Value::Array(cleaned_blocks.clone()),
            );
        }
    }
    let params_json = params.to_string();
    let text_opt = if text.is_empty() {
        None
    } else {
        Some(text.as_str())
    };
    let assistant =
        session::insert_message(conn, session_id, "assistant", text_opt, Some(&params_json))?;

    // Bind any file mutations captured before the interrupt / error to this
    // partial message so they roll back if the message is deleted.
    let file_changes = file_snapshots.take(session_id);
    if !file_changes.is_empty() {
        if let Err(e) = crate::data::file_snapshot::save_changes(
            conn,
            session_id,
            &assistant.id,
            &file_changes,
        ) {
            eprintln!(
                "persist_streamed: save_changes failed for session {session_id}: {e}"
            );
        }
    }
    if let Some(req_id) = request_message_id {
        if let Err(e) =
            crate::data::pending_diff::bind_message(conn, session_id, req_id, &assistant.id)
        {
            eprintln!("persist_streamed: bind_message failed for session {session_id}: {e}");
        }
    } else if let Err(e) =
        crate::data::pending_diff::bind_unbound(conn, session_id, &assistant.id)
    {
        eprintln!("persist_streamed: bind_unbound failed for session {session_id}: {e}");
    }

    session::recompute_context_window_used(conn, session_id)?;
    Ok(())
}

/// Append a text delta to the ordered block list, merging with the
/// trailing block when it is also a `text` block.
pub(crate) fn append_text_delta_block(blocks: &mut Vec<serde_json::Value>, delta: &str) {
    if delta.is_empty() {
        return;
    }
    if let Some(last) = blocks.last_mut() {
        if last.get("type").and_then(|v| v.as_str()) == Some("text") {
            if let Some(content) = last.get_mut("content").and_then(|c| c.as_str()) {
                let merged = format!("{content}{delta}");
                if let Some(obj) = last.as_object_mut() {
                    obj.insert("content".into(), serde_json::Value::String(merged));
                }
                return;
            }
        }
    }
    blocks.push(serde_json::json!({ "type": "text", "content": delta }));
}

/// Same as [`append_text_delta_block`] but for `thinking` blocks.
pub(crate) fn append_thinking_delta_block(blocks: &mut Vec<serde_json::Value>, delta: &str) {
    if delta.is_empty() {
        return;
    }
    if let Some(last) = blocks.last_mut() {
        if last.get("type").and_then(|v| v.as_str()) == Some("thinking") {
            if let Some(content) = last.get_mut("content").and_then(|c| c.as_str()) {
                let merged = format!("{content}{delta}");
                if let Some(obj) = last.as_object_mut() {
                    obj.insert("content".into(), serde_json::Value::String(merged));
                }
                return;
            }
        }
    }
    blocks.push(serde_json::json!({ "type": "thinking", "content": delta }));
}

/// Push a new `tool_use` block in `pending` state.
pub(crate) fn record_tool_use_block(
    blocks: &mut Vec<serde_json::Value>,
    id: &str,
    tool: &str,
    input: &serde_json::Value,
) {
    blocks.push(serde_json::json!({
        "type": "tool_use",
        "id": id,
        "tool": tool,
        "input": input.clone(),
        "status": "pending",
    }));
}

/// Mutate the matching `tool_use` block in place with the tool result.
/// No-op if the matching id can't be found (defensive against duplicated
/// or out-of-order events).
pub(crate) fn record_tool_result_block(
    blocks: &mut Vec<serde_json::Value>,
    id: &str,
    output: &serde_json::Value,
    is_error: bool,
) {
    for b in blocks.iter_mut().rev() {
        if b.get("type").and_then(|v| v.as_str()) != Some("tool_use") {
            continue;
        }
        if b.get("id").and_then(|v| v.as_str()) != Some(id) {
            continue;
        }
        if let Some(obj) = b.as_object_mut() {
            obj.insert(
                "status".into(),
                serde_json::Value::String(if is_error { "error" } else { "success" }.into()),
            );
            obj.insert("output".into(), output.clone());
            if is_error {
                obj.insert("is_error".into(), serde_json::Value::Bool(true));
            }
        }
        return;
    }
}

pub(crate) fn stream_text_callback(
    app: AppHandle,
    session_id: String,
    request_message_id: String,
    blocks: StreamBlocks,
) -> chat::TextDeltaCallback {
    // Per-request stateful cleaner: strips any host tool-transcript lines
    // (`[已调用工具 ...]`, `[阶段: ...]`) a model might echo, holding back
    // only a trailing fragment that could still become such a marker so
    // normal prose streams unimpeded. Shared (cloned) across chain stages.
    let splitter = Arc::new(std::sync::Mutex::new(
        crate::ai::stream_split::StreamContentSplitter::default(),
    ));
    Arc::new(move |delta| {
        // Route visible text through the marker cleaner before it reaches
        // either the persisted block buffer or the live UI stream.
        let cleaned_text = delta.text.as_deref().map(|t| {
            splitter
                .lock()
                .map(|mut s| s.push(t))
                .unwrap_or_else(|_| t.to_string())
        });
        if let Ok(mut g) = blocks.lock() {
            if let Some(t) = cleaned_text.as_deref() {
                append_text_delta_block(&mut g, t);
            }
            if let Some(t) = delta.thinking.as_deref() {
                append_thinking_delta_block(&mut g, t);
            }
        }
        // Live tool-call argument fragments are renderer-only: the shared
        // `blocks` buffer (used for persistence) is populated later by the
        // engine's terminal `ToolUse` event via `record_tool_use_block`,
        // so we deliberately don't write the partial input here.
        let tool_call_delta = delta.tool_call.as_ref().map(|tc| {
            serde_json::json!({
                "id": tc.id,
                "name": tc.name,
                "arguments": tc.arguments,
            })
        });
        // Suppress an empty text_delta emitted purely because the cleaner
        // held everything back this chunk (avoids a no-op UI event).
        let emit_text = match cleaned_text.as_deref() {
            Some("") => None,
            other => other.map(|s| s.to_string()),
        };
        let _ = app.emit(
            "gen://stream",
            serde_json::json!({
                "session_id": &session_id,
                "request_message_id": &request_message_id,
                "text_delta": emit_text,
                "thinking_delta": delta.thinking,
                "tool_call_delta": tool_call_delta,
            }),
        );
    })
}

/// Build the `gen://tool` callback. Mirrors [`stream_text_callback`]:
/// updates the shared block buffer first, then forwards a structured
/// payload to the renderer so the UI can render the tool card inline
/// the moment the engine fires the event.
pub(crate) fn tool_event_callback(
    app: AppHandle,
    session_id: String,
    request_message_id: String,
    blocks: StreamBlocks,
) -> ToolEventCallback {
    Arc::new(move |event| match event {
        MessageEvent::ToolUse { id, tool, input } => {
            if let Ok(mut g) = blocks.lock() {
                record_tool_use_block(&mut g, id.as_str(), tool, input);
            }
            let _ = app.emit(
                "gen://tool",
                serde_json::json!({
                    "session_id": &session_id,
                    "request_message_id": &request_message_id,
                    "type": "tool_use",
                    "id": id.as_str(),
                    "tool": tool,
                    "input": input,
                }),
            );
        }
        MessageEvent::ToolResult {
            id,
            tool,
            output,
            is_error,
        } => {
            if let Ok(mut g) = blocks.lock() {
                record_tool_result_block(&mut g, id.as_str(), output, *is_error);
            }
            let _ = app.emit(
                "gen://tool",
                serde_json::json!({
                    "session_id": &session_id,
                    "request_message_id": &request_message_id,
                    "type": "tool_result",
                    "id": id.as_str(),
                    "tool": tool,
                    "output": output,
                    "is_error": is_error,
                }),
            );
        }
        // Other variants (Assistant text, User, Progress, CompactBoundary)
        // aren't structural tool events - ignore them here.
        _ => {}
    })
}
