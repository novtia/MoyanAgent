use serde_json::{json, Value};

use crate::ai::chat::{StreamDelta, TextDeltaCallback};

/// Buffered shape used while streaming tool_calls arrive piecewise.
/// OpenAI chat completions stream tool_calls as a sequence of deltas
/// indexed by `index`: the first delta carries `id` / `type` /
/// `function.name`, later deltas append more `function.arguments`
/// fragments. Responses API uses `item_id` / `output_index` instead.
/// We accumulate them here and emit a final
/// [`crate::ai::chat::ProviderToolCall`] list when the stream ends.
#[derive(Debug, Default, Clone)]
pub(crate) struct PendingStreamToolCall {
    /// Call id forwarded to the UI / tool loop (`call_id` on Responses).
    pub(crate) id: String,
    /// Responses output-item id (`item.id`); used to match argument deltas.
    pub(crate) item_id: String,
    pub(crate) name: String,
    pub(crate) arguments: String,
}

/// Merge `choices[*].delta.tool_calls[*]` from one SSE event into the
/// running accumulator. Tool calls are addressed by `index` (OpenAI
/// guarantees stable indices across chunks for the same call).
///
/// Besides buffering for the final [`crate::ai::chat::ProviderToolCall`],
/// each fragment is forwarded via `on_text_delta` as a
/// [`StreamDelta::tool_call`] so the renderer can display the tool input
/// (e.g. a document's `content`) as it streams in, before the turn ends.
pub(crate) fn merge_tool_call_deltas(
    v: &Value,
    out: &mut Vec<PendingStreamToolCall>,
    on_text_delta: &TextDeltaCallback,
) {
    let Some(choices) = v.get("choices").and_then(Value::as_array) else {
        return;
    };
    for choice in choices {
        let Some(arr) = choice
            .pointer("/delta/tool_calls")
            .and_then(Value::as_array)
        else {
            continue;
        };
        for tc in arr {
            let idx = tc
                .get("index")
                .and_then(Value::as_u64)
                .map(|i| i as usize)
                .unwrap_or_else(|| out.len());
            if idx >= out.len() {
                out.resize_with(idx + 1, PendingStreamToolCall::default);
            }
            let slot = &mut out[idx];
            let mut identity_changed = false;
            if let Some(id) = tc.get("id").and_then(Value::as_str) {
                if !id.is_empty() {
                    slot.id = id.to_string();
                    identity_changed = true;
                }
            }
            if let Some(name) = tc.pointer("/function/name").and_then(Value::as_str) {
                if !name.is_empty() {
                    slot.name = name.to_string();
                    identity_changed = true;
                }
            }
            let mut fragment = String::new();
            if let Some(args) = tc.pointer("/function/arguments").and_then(Value::as_str) {
                if !args.is_empty() {
                    slot.arguments.push_str(args);
                    fragment.push_str(args);
                }
            }
            // Forward the live fragment once we know the call id. An empty
            // `fragment` with a fresh id/name still emits, so the UI can
            // create the pending card before any argument bytes arrive.
            if !slot.id.is_empty() && (identity_changed || !fragment.is_empty()) {
                (on_text_delta)(StreamDelta::tool_call(
                    slot.id.clone(),
                    slot.name.clone(),
                    fragment,
                ));
            }
        }
    }
}
/// Typewriter-split tool argument fragments so large CreateDoc/Write payloads
/// don't appear as a one-shot dump when Ark batches them.
pub(crate) fn emit_tool_arg_deltas(cb: &TextDeltaCallback, id: &str, name: &str, chunk: &str) {
    if id.is_empty() {
        return;
    }
    // Card creation with unknown name is deferred until name arrives.
    if name.is_empty() && chunk.is_empty() {
        return;
    }
    if chunk.is_empty() {
        (cb)(StreamDelta::tool_call(
            id.to_string(),
            name.to_string(),
            String::new(),
        ));
        return;
    }
    for ch in chunk.chars() {
        (cb)(StreamDelta::tool_call(
            id.to_string(),
            name.to_string(),
            ch.to_string(),
        ));
    }
}

/// Convert the buffered stream-side tool-call state into the provider-
/// agnostic shape the agent loop expects. Drops entries that never
/// received a name (defensive against malformed streams).
pub(crate) fn finalize_pending_tool_calls(
    pending: Vec<PendingStreamToolCall>,
) -> Vec<crate::ai::chat::ProviderToolCall> {
    pending
        .into_iter()
        .filter(|p| !p.name.is_empty())
        .map(|p| {
            let arguments = parse_tool_call_arguments(&p.name, &p.arguments);
            crate::ai::chat::ProviderToolCall {
                id: p.id,
                name: p.name,
                arguments,
            }
        })
        .collect()
}

/// Parse a tool call's accumulated `function.arguments` string.
///
/// Some upstreams (notably doubao/ark) occasionally emit argument JSON that
/// strict parsing rejects: raw control characters inside string values,
/// output truncated mid-stream, or the whole payload concatenated twice.
/// Dropping the call to `Value::Null` loses the entire generation, so on
/// strict-parse failure we attempt a best-effort repair before giving up.
pub(crate) fn parse_tool_call_arguments(tool_name: &str, raw: &str) -> Value {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return json!({});
    }
    match serde_json::from_str::<Value>(trimmed) {
        Ok(v) => v,
        Err(err) => match repair_tool_call_arguments(trimmed) {
            Some(v) => {
                eprintln!(
                    "[ATELIER_TOOL_ARGS] repaired malformed `{}` arguments ({} bytes; strict parse: {})",
                    tool_name,
                    trimmed.len(),
                    err
                );
                v
            }
            None => {
                eprintln!(
                    "[ATELIER_TOOL_ARGS] unrepairable `{}` arguments ({} bytes; strict parse: {})",
                    tool_name,
                    trimmed.len(),
                    err
                );
                Value::Null
            }
        },
    }
}

/// Best-effort repair of malformed tool-argument JSON. Returns `None` when
/// nothing parseable can be salvaged.
pub(crate) fn repair_tool_call_arguments(raw: &str) -> Option<Value> {
    // Duplicated / concatenated payloads (`{...}{...}`): strict parsing fails
    // with "trailing characters", but the first value alone is valid.
    if let Some(v) = first_json_value(raw) {
        return Some(v);
    }

    // Escape raw control chars and invalid escape sequences inside strings.
    let (clean, open_stack, in_string) = sanitize_json_fragment(raw);
    if let Some(v) = first_json_value(&clean) {
        return Some(v);
    }

    // Truncated output: close whatever is still open. Several closing
    // strategies are tried because the cut may fall mid-key, mid-value or
    // right after a separator.
    let closers: String = open_stack
        .iter()
        .rev()
        .map(|c| if *c == '{' { '}' } else { ']' })
        .collect();
    let mut candidates: Vec<String> = Vec::new();
    if in_string {
        // Cut inside a string value: close the quote.
        candidates.push(format!("{clean}\"{closers}"));
        // Cut inside an object key: close the quote and give it a value.
        candidates.push(format!("{clean}\":null{closers}"));
    } else {
        candidates.push(format!("{clean}{closers}"));
        // Cut right after `:` → the key still needs a value.
        candidates.push(format!("{clean}null{closers}"));
        // Cut right after `,` → drop the dangling separator.
        let stripped = clean.trim_end().trim_end_matches([',', ':']);
        candidates.push(format!("{stripped}{closers}"));
    }
    candidates
        .iter()
        .find_map(|cand| serde_json::from_str::<Value>(cand).ok())
}

/// Extract the first complete JSON value from `raw`, ignoring anything after it.
pub(crate) fn first_json_value(raw: &str) -> Option<Value> {
    let mut iter = serde_json::Deserializer::from_str(raw).into_iter::<Value>();
    match iter.next() {
        Some(Ok(v)) => Some(v),
        _ => None,
    }
}

/// Single pass over a (possibly malformed) JSON fragment:
/// - escapes raw control characters (U+0000..U+001F) inside strings;
/// - neutralizes invalid escape sequences (`\x`, incomplete `\uXX`) by
///   escaping the backslash itself;
/// - records which containers are still open and whether the fragment ends
///   inside a string, so the caller can synthesize closers.
pub(crate) fn sanitize_json_fragment(raw: &str) -> (String, Vec<char>, bool) {
    let chars: Vec<char> = raw.chars().collect();
    let mut out = String::with_capacity(raw.len());
    let mut stack: Vec<char> = Vec::new();
    let mut in_string = false;
    let mut i = 0;
    while i < chars.len() {
        let ch = chars[i];
        if !in_string {
            match ch {
                '"' => {
                    in_string = true;
                    out.push(ch);
                }
                '{' | '[' => {
                    stack.push(ch);
                    out.push(ch);
                }
                '}' => {
                    if stack.last() == Some(&'{') {
                        stack.pop();
                    }
                    out.push(ch);
                }
                ']' => {
                    if stack.last() == Some(&'[') {
                        stack.pop();
                    }
                    out.push(ch);
                }
                _ => out.push(ch),
            }
            i += 1;
            continue;
        }
        match ch {
            '"' => {
                in_string = false;
                out.push(ch);
                i += 1;
            }
            '\\' => match chars.get(i + 1) {
                None => {
                    // Dangling escape at buffer end: drop it.
                    i += 1;
                }
                Some('u') => {
                    let hex_ok = chars.len() >= i + 6
                        && chars[i + 2..i + 6].iter().all(|c| c.is_ascii_hexdigit());
                    if hex_ok {
                        out.push('\\');
                        out.push('u');
                        out.extend(&chars[i + 2..i + 6]);
                        i += 6;
                    } else {
                        out.push_str("\\\\u");
                        i += 2;
                    }
                }
                Some(next @ ('"' | '\\' | '/' | 'b' | 'f' | 'n' | 'r' | 't')) => {
                    out.push('\\');
                    out.push(*next);
                    i += 2;
                }
                Some(_) => {
                    // Invalid escape like `\x`: escape the backslash literally
                    // and reprocess the following char normally.
                    out.push_str("\\\\");
                    i += 1;
                }
            },
            c if (c as u32) < 0x20 => {
                match c {
                    '\n' => out.push_str("\\n"),
                    '\r' => out.push_str("\\r"),
                    '\t' => out.push_str("\\t"),
                    _ => out.push_str(&format!("\\u{:04x}", c as u32)),
                }
                i += 1;
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    (out, stack, in_string)
}
