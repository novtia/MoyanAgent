//! `FixToolCall` — amend any earlier failed tool call without resending it.
//!
//! The model submits only the fields that should change. The agent loop
//! deep-merges those fields onto the failed call's arguments and runs the
//! original tool again. This module resolves that merge; it does not execute
//! the original tool. [`Tool::execute`] is a fallback for a call that reached
//! the pool without being intercepted.

use serde_json::{json, Map, Value};

use crate::ai::agent::tools::{Tool, ToolFuture, ToolInvocation, ToolResult, ToolSpec};

pub const TOOL_NAME: &str = "FixToolCall";

/// Strings longer than this are summarized by character count so a fix hint
/// does not copy the body back into the tool result.
const LONG_TEXT_CHARS: usize = 200;

/// Arrays longer than this are summarized by length instead of element-wise.
const LONG_ARRAY_LEN: usize = 20;

#[derive(Clone)]
pub struct FixToolCallTool {
    spec: ToolSpec,
}

impl FixToolCallTool {
    pub fn new() -> Self {
        Self {
            spec: ToolSpec {
                name: TOOL_NAME.to_string(),
                description: "Amend any earlier failed tool call. `set` is deep-merged onto \
                    that call's arguments and the original tool runs again. Omit `call_id` to \
                    amend the latest failure. Do not resend long text listed under `kept`, and \
                    do not use this to start a new action."
                    .to_string(),
                schema: json!({
                    "type": "object",
                    "properties": {
                        "set": {
                            "type": "object",
                            "description": "Fields to add or replace. Nested objects merge; \
                                arrays and scalars replace. Omit or pass {} to rerun the failed \
                                call unchanged. Do not include long text listed under kept."
                        },
                        "call_id": {
                            "type": "string",
                            "description": "Id of the failed call to amend. Omit to amend the \
                                latest failed tool call in this conversation."
                        }
                    }
                }),
                // The amendment re-runs the original tool, which may write.
                read_only: false,
                concurrency_safe: false,
            },
        }
    }
}

impl Default for FixToolCallTool {
    fn default() -> Self {
        Self::new()
    }
}

impl Tool for FixToolCallTool {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    fn execute<'a>(&'a self, _invocation: ToolInvocation<'a>) -> ToolFuture<'a> {
        Box::pin(async move {
            Ok(ToolResult::error(
                "FixToolCall: the agent loop resolves this call before execution. \
                 Retry the amendment.",
            ))
        })
    }
}

/// One finished tool call the amendment resolver can see.
#[derive(Debug, Clone)]
pub struct PriorToolCall {
    pub id: String,
    pub name: String,
    pub arguments: Value,
    pub is_error: bool,
    pub result: Value,
}

/// Original tool plus the arguments to run, after prior patches and `set`.
#[derive(Debug, Clone)]
pub struct ResolvedAmendment {
    pub tool_name: String,
    pub call_id: String,
    pub input: Value,
}

/// Merge `patch` onto `base`. Objects merge recursively. Arrays and scalars
/// replace. JSON `null` replaces; it does not delete the key.
pub fn deep_merge(base: &Value, patch: &Value) -> Value {
    match (base, patch) {
        (Value::Object(base_map), Value::Object(patch_map)) => {
            let mut out = base_map.clone();
            for (key, value) in patch_map {
                let merged = match out.get(key) {
                    Some(existing) => deep_merge(existing, value),
                    None => value.clone(),
                };
                out.insert(key.clone(), merged);
            }
            Value::Object(out)
        }
        _ => patch.clone(),
    }
}

/// Locate the failed call described by `fix_input` and produce the merged
/// arguments. `calls` is chronological (oldest first) across history, this
/// turn's earlier rounds, and calls already finished in the current round.
pub fn resolve_amendment(
    calls: &[PriorToolCall],
    fix_input: &Value,
) -> Result<ResolvedAmendment, String> {
    let set = match fix_input.get("set") {
        None | Some(Value::Null) => json!({}),
        Some(value) if value.is_object() => value.clone(),
        Some(_) => return Err("FixToolCall: `set` must be an object".into()),
    };
    let requested_id = match fix_input.get("call_id") {
        None | Some(Value::Null) => None,
        Some(Value::String(id)) => {
            let trimmed = id.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        }
        Some(_) => return Err("FixToolCall: `call_id` must be a string".into()),
    };

    let target_index = if let Some(id) = requested_id.as_deref() {
        calls.iter().position(|call| call.id == id).ok_or_else(|| {
            format!("FixToolCall: no tool call with id `{id}` in this conversation")
        })?
    } else {
        let mut chosen = None;
        for index in (0..calls.len()).rev() {
            let name = calls[index].name.clone();
            let id = calls[index].id.clone();
            let is_error = calls[index].is_error;
            if name == TOOL_NAME || !is_error || successfully_amended(calls, &id) {
                continue;
            }
            chosen = Some(index);
            break;
        }
        chosen.ok_or_else(|| {
            "FixToolCall: no failed tool call to amend in this conversation".to_string()
        })?
    };

    let target_id = calls[target_index].id.clone();
    let target_name = calls[target_index].name.clone();
    let target_is_error = calls[target_index].is_error;
    let target_result = calls[target_index].result.clone();
    let mut merged = calls[target_index].arguments.clone();

    if target_name == TOOL_NAME {
        return Err(
            "FixToolCall: cannot amend another FixToolCall. Omit call_id to amend the latest \
             failed tool, or pass that tool's call_id."
                .into(),
        );
    }
    if !target_is_error {
        return Err(format!(
            "FixToolCall: call `{target_id}` ({target_name}) succeeded. Only failed calls can be amended."
        ));
    }
    if is_unamendable_permission(&target_result) {
        return Err(format!(
            "FixToolCall: call `{target_id}` ({target_name}) was denied or is waiting for user \
             approval. Do not amend it. Pass call_id to amend an older failure, or ask the user \
             to approve the action."
        ));
    }
    if successfully_amended(calls, &target_id) {
        return Err(format!(
            "FixToolCall: call `{target_id}` ({target_name}) was already amended successfully. \
             Do not amend it again."
        ));
    }
    if !merged.is_object() {
        return Err(format!(
            "FixToolCall: arguments for {target_name} call `{target_id}` were not recovered. \
             Call {target_name} again with full arguments."
        ));
    }

    for prior in calls.iter().skip(target_index + 1) {
        if prior.name != TOOL_NAME {
            continue;
        }
        let aimed = prior.result.get("target_call_id").and_then(Value::as_str);
        if aimed != Some(target_id.as_str()) {
            continue;
        }
        if let Some(prev_set) = prior.arguments.get("set").filter(|value| value.is_object()) {
            merged = deep_merge(&merged, prev_set);
        }
    }
    merged = deep_merge(&merged, &set);

    Ok(ResolvedAmendment {
        tool_name: target_name,
        call_id: target_id,
        input: merged,
    })
}

fn successfully_amended(calls: &[PriorToolCall], target_id: &str) -> bool {
    calls.iter().any(|call| {
        call.name == TOOL_NAME
            && !call.is_error
            && call.result.get("target_call_id").and_then(Value::as_str) == Some(target_id)
    })
}

fn is_unamendable_permission(result: &Value) -> bool {
    let message = result.get("error").and_then(Value::as_str).unwrap_or("");
    message.starts_with("denied:") || message.starts_with("awaiting user approval")
}

fn is_cancelled(result: &Value) -> bool {
    result.get("error").and_then(Value::as_str) == Some("generation cancelled")
}

/// Record which failed call a `FixToolCall` result belongs to.
pub fn stamp_amendment(content: &mut Value, tool_name: &str, target_call_id: &str) {
    let obj = object_content(content);
    obj.insert("amended_tool".into(), Value::String(tool_name.to_string()));
    obj.insert(
        "target_call_id".into(),
        Value::String(target_call_id.to_string()),
    );
}

/// Attach `fix` / `kept` / `invalid` / `call_id` / `tool` to a reusable
/// failure. Permission denials, approval waits, and cancellation are left
/// unchanged. A payload that is not a JSON object gets a note to call the
/// original tool again, and no `fix` hint.
pub fn annotate_failure(
    content: &mut Value,
    tool_name: &str,
    call_id: &str,
    input: &Value,
    schema: Option<&Value>,
) {
    if tool_name == TOOL_NAME || is_unamendable_permission(content) || is_cancelled(content) {
        return;
    }
    if !input.is_object() {
        let obj = object_content(content);
        let note = format!(
            " Arguments for this {tool_name} call were not recovered because they were not a \
             JSON object. Call {tool_name} again with full arguments. FixToolCall cannot amend \
             this call."
        );
        match obj.get_mut("error") {
            Some(Value::String(message)) if !message.contains("were not recovered") => {
                message.push_str(&note);
            }
            Some(_) => {}
            None => {
                obj.insert("error".into(), Value::String(note.trim().to_string()));
            }
        }
        return;
    }

    let invalid = invalid_fields(input, schema);
    let kept = kept_summary(input, &invalid);
    let fix = format!(
        "Do not call {tool_name} again. Do not resend long text listed in `kept`. \
         Call FixToolCall and put only the fields to add or replace in `set` \
         (nested objects merge; arrays and scalars replace). \
         Omit `set` or pass {{}} to rerun with the same arguments. \
         Omit `call_id` to amend this failure. Pass `call_id` \"{call_id}\" only when \
         amending an older failure."
    );
    let obj = object_content(content);
    obj.insert("call_id".into(), Value::String(call_id.to_string()));
    obj.insert("tool".into(), Value::String(tool_name.to_string()));
    if !invalid.is_empty() {
        obj.insert(
            "invalid".into(),
            Value::Array(invalid.into_iter().map(Value::String).collect()),
        );
    }
    obj.insert("kept".into(), kept);
    obj.insert("fix".into(), Value::String(fix));
}

fn object_content(content: &mut Value) -> &mut Map<String, Value> {
    if !content.is_object() {
        let previous = std::mem::replace(content, Value::Null);
        let message = match previous {
            Value::String(text) => text,
            other => other.to_string(),
        };
        *content = json!({ "error": message });
    }
    content
        .as_object_mut()
        .expect("content was just made an object")
}

fn invalid_fields(input: &Value, schema: Option<&Value>) -> Vec<String> {
    let Some(input) = input.as_object() else {
        return Vec::new();
    };
    let Some(schema) = schema.and_then(Value::as_object) else {
        return Vec::new();
    };
    let properties = schema.get("properties").and_then(Value::as_object);
    let required = schema
        .get("required")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let mut invalid = Vec::new();
    for key in &required {
        match input.get(key) {
            None => invalid.push(key.clone()),
            Some(value) if !field_ok(properties.and_then(|props| props.get(key)), value) => {
                invalid.push(key.clone());
            }
            _ => {}
        }
    }
    if let Some(properties) = properties {
        for (key, value) in input {
            if required.iter().any(|required_key| required_key == key) {
                continue;
            }
            if !field_ok(properties.get(key), value) {
                invalid.push(key.clone());
            }
        }
    }
    invalid
}

fn field_ok(property_schema: Option<&Value>, value: &Value) -> bool {
    let Some(schema) = property_schema.and_then(Value::as_object) else {
        return true;
    };
    if let Some(type_name) = schema.get("type") {
        if !type_matches(type_name, value) {
            return false;
        }
    }
    if let Some(enum_values) = schema.get("enum").and_then(Value::as_array) {
        if !enum_values.iter().any(|candidate| candidate == value) {
            return false;
        }
    }
    true
}

fn type_matches(schema_type: &Value, value: &Value) -> bool {
    let names: Vec<&str> = match schema_type {
        Value::String(name) => vec![name.as_str()],
        Value::Array(items) => items.iter().filter_map(Value::as_str).collect(),
        _ => return true,
    };
    if names.is_empty() {
        return true;
    }
    names.iter().any(|name| json_type_is(name, value))
}

fn json_type_is(type_name: &str, value: &Value) -> bool {
    match type_name {
        "string" => value.is_string(),
        "number" => value.is_number(),
        "integer" => value.as_i64().is_some() || value.as_u64().is_some(),
        "boolean" => value.is_boolean(),
        "object" => value.is_object(),
        "array" => value.is_array(),
        "null" => value.is_null(),
        _ => true,
    }
}

fn kept_summary(input: &Value, invalid: &[String]) -> Value {
    let Some(input) = input.as_object() else {
        return json!({});
    };
    let mut kept = Map::new();
    for (key, value) in input {
        if invalid.iter().any(|invalid_key| invalid_key == key) {
            continue;
        }
        kept.insert(key.clone(), summarize_value(value));
    }
    Value::Object(kept)
}

fn summarize_value(value: &Value) -> Value {
    match value {
        Value::String(text) if text.chars().count() > LONG_TEXT_CHARS => json!({
            "type": "string",
            "chars": text.chars().count()
        }),
        Value::Array(items) if items.len() > LONG_ARRAY_LEN => json!({
            "type": "array",
            "len": items.len()
        }),
        Value::Array(items) => Value::Array(items.iter().map(summarize_value).collect()),
        Value::Object(map) => {
            let mut summarized = Map::new();
            for (key, child) in map {
                summarized.insert(key.clone(), summarize_value(child));
            }
            Value::Object(summarized)
        }
        other => other.clone(),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use serde_json::json;

    use super::*;
    use crate::ai::agent::core::file_snapshot::FileSnapshotStore;
    use crate::ai::agent::tools::create_doc::CreateDocTool;
    use crate::ai::agent::tools::Tool;

    fn failed(id: &str, name: &str, arguments: Value) -> PriorToolCall {
        PriorToolCall {
            id: id.into(),
            name: name.into(),
            arguments,
            is_error: true,
            result: json!({ "error": "failed" }),
        }
    }

    fn prior_fix(id: &str, target: &str, set: Value, is_error: bool) -> PriorToolCall {
        PriorToolCall {
            id: id.into(),
            name: TOOL_NAME.into(),
            arguments: json!({ "set": set }),
            is_error,
            result: json!({
                "target_call_id": target,
                "amended_tool": "CreateDoc"
            }),
        }
    }

    #[test]
    fn create_doc_title_patch_reuses_the_body() {
        let content = "章".repeat(80);
        let calls = vec![failed(
            "c1",
            "CreateDoc",
            json!({ "content": content, "doc_type": "md" }),
        )];
        let resolved = resolve_amendment(&calls, &json!({ "set": { "title": "番外一" } })).unwrap();
        assert_eq!(resolved.tool_name, "CreateDoc");
        assert_eq!(resolved.call_id, "c1");
        assert_eq!(resolved.input["content"], content);
        assert_eq!(resolved.input["doc_type"], "md");
        let tool = CreateDocTool::new(Arc::new(FileSnapshotStore::new()));
        tool.validate(&resolved.input).unwrap();
    }

    #[test]
    fn nested_object_patch_keeps_sibling_fields() {
        let calls = vec![failed(
            "r1",
            "RoleState",
            json!({
                "action": "update",
                "state": { "name": "A", "bio": "long bio" }
            }),
        )];
        let resolved =
            resolve_amendment(&calls, &json!({ "set": { "state": { "name": "B" } } })).unwrap();
        assert_eq!(resolved.input["action"], "update");
        assert_eq!(resolved.input["state"]["name"], "B");
        assert_eq!(resolved.input["state"]["bio"], "long bio");
    }

    #[test]
    fn array_patch_replaces_the_array() {
        let calls = vec![failed(
            "t1",
            "TodoList",
            json!({ "action": "write", "tasks": [{ "id": "1" }, { "id": "2" }] }),
        )];
        let resolved =
            resolve_amendment(&calls, &json!({ "set": { "tasks": [{ "id": "9" }] } })).unwrap();
        assert_eq!(resolved.input["action"], "write");
        assert_eq!(resolved.input["tasks"], json!([{ "id": "9" }]));
    }

    #[test]
    fn second_patch_keeps_the_first_patch() {
        let content = "正文".repeat(40);
        let calls = vec![
            failed(
                "c1",
                "CreateDoc",
                json!({ "content": content, "doc_type": "md" }),
            ),
            prior_fix("f1", "c1", json!({ "title": "番外" }), true),
        ];
        let resolved = resolve_amendment(&calls, &json!({ "set": { "folder": "番外" } })).unwrap();
        assert_eq!(resolved.call_id, "c1");
        assert_eq!(resolved.input["title"], "番外");
        assert_eq!(resolved.input["folder"], "番外");
        assert_eq!(resolved.input["content"], content);
        assert_eq!(resolved.input["doc_type"], "md");
    }

    #[test]
    fn empty_set_still_applies_an_earlier_patch() {
        let calls = vec![
            failed(
                "c1",
                "CreateDoc",
                json!({ "content": "hi", "doc_type": "md" }),
            ),
            prior_fix("f1", "c1", json!({ "title": "番外" }), true),
        ];
        let resolved = resolve_amendment(&calls, &json!({})).unwrap();
        assert_eq!(resolved.call_id, "c1");
        assert_eq!(resolved.input["title"], "番外");
        assert_eq!(resolved.input["content"], "hi");
    }

    #[test]
    fn call_id_selects_an_older_failure() {
        let calls = vec![
            failed("old", "Bash", json!({ "command": "echo hi" })),
            failed("new", "Read", json!({ "path": "a.txt" })),
        ];
        let older = resolve_amendment(&calls, &json!({ "call_id": "old", "set": {} })).unwrap();
        assert_eq!(older.tool_name, "Bash");
        assert_eq!(older.input["command"], "echo hi");
        let latest = resolve_amendment(&calls, &json!({})).unwrap();
        assert_eq!(latest.tool_name, "Read");
        assert_eq!(latest.call_id, "new");
    }

    #[test]
    fn latest_fix_tool_call_does_not_hide_the_failed_tool() {
        let calls = vec![
            failed("c1", "Edit", json!({ "path": "a.md", "old_string": "x" })),
            prior_fix("f1", "c1", json!({ "new_string": "y" }), true),
        ];
        let resolved = resolve_amendment(&calls, &json!({})).unwrap();
        assert_eq!(resolved.tool_name, "Edit");
        assert_eq!(resolved.input["new_string"], "y");
        assert_eq!(resolved.input["old_string"], "x");
    }

    #[test]
    fn refuses_a_successful_call() {
        let mut call = failed(
            "c1",
            "CreateDoc",
            json!({ "title": "a", "doc_type": "md", "content": "b" }),
        );
        call.is_error = false;
        call.result = json!({ "path": "a.md" });
        let err = resolve_amendment(&[call], &json!({ "call_id": "c1" })).unwrap_err();
        assert!(err.contains("succeeded"), "{err}");
    }

    #[test]
    fn refuses_a_permission_denial() {
        let mut call = failed("c1", "Delete", json!({ "path": "a" }));
        call.result = json!({ "error": "denied: plan mode" });
        let err = resolve_amendment(&[call], &json!({})).unwrap_err();
        assert!(err.contains("approval") || err.contains("denied"), "{err}");
    }

    #[test]
    fn refuses_arguments_that_were_not_recovered() {
        let call = failed("c1", "CreateDoc", Value::Null);
        let err = resolve_amendment(&[call], &json!({ "set": { "title": "x" } })).unwrap_err();
        assert!(err.contains("not recovered"), "{err}");
    }

    #[test]
    fn refuses_a_call_that_was_already_amended() {
        let calls = vec![
            failed("c1", "CreateDoc", json!({ "content": "hi" })),
            prior_fix("f1", "c1", json!({ "title": "番外" }), false),
        ];
        let err = resolve_amendment(
            &calls,
            &json!({ "call_id": "c1", "set": { "doc_type": "md" } }),
        )
        .unwrap_err();
        assert!(err.contains("already amended"), "{err}");
    }

    #[test]
    fn default_target_skips_a_call_that_was_already_amended() {
        let calls = vec![
            failed("c1", "CreateDoc", json!({ "content": "hi" })),
            prior_fix("f1", "c1", json!({ "title": "番外" }), false),
            failed("r1", "Read", json!({ "path": "a" })),
        ];
        let resolved = resolve_amendment(&calls, &json!({})).unwrap();
        assert_eq!(resolved.call_id, "r1");
        assert_eq!(resolved.tool_name, "Read");
    }

    #[test]
    fn failure_hint_summarizes_long_text_and_names_missing_fields() {
        let tool = CreateDocTool::new(Arc::new(FileSnapshotStore::new()));
        let body = "字".repeat(LONG_TEXT_CHARS + 1);
        let input = json!({ "content": body, "doc_type": "pdf" });
        let mut content = json!({
            "error": "invalid argument: CreateDoc: `title` must be a string"
        });
        annotate_failure(
            &mut content,
            "CreateDoc",
            "call-1",
            &input,
            Some(&tool.spec().schema),
        );
        assert_eq!(content["call_id"], "call-1");
        assert_eq!(content["tool"], "CreateDoc");
        assert_eq!(content["invalid"], json!(["title", "doc_type"]));
        assert_eq!(content["kept"]["content"]["chars"], LONG_TEXT_CHARS + 1);
        let kept = content["kept"].to_string();
        assert!(!kept.contains(&body), "kept leaked the document body");
        assert!(content["fix"].as_str().unwrap().contains("FixToolCall"));
        assert_eq!(
            content["error"],
            "invalid argument: CreateDoc: `title` must be a string"
        );
    }

    #[test]
    fn permission_and_unparsed_failures_do_not_offer_fix() {
        let tool = CreateDocTool::new(Arc::new(FileSnapshotStore::new()));
        let schema = &tool.spec().schema;
        let mut denied = json!({ "error": "awaiting user approval: needs approval. Do not retry" });
        annotate_failure(
            &mut denied,
            "CreateDoc",
            "c",
            &json!({ "title": "a" }),
            Some(schema),
        );
        assert!(denied.get("fix").is_none());

        let mut missing =
            json!({ "error": "invalid argument: CreateDoc: `title` must be a string" });
        annotate_failure(&mut missing, "CreateDoc", "c", &Value::Null, Some(schema));
        assert!(missing.get("fix").is_none());
        assert!(missing["error"].as_str().unwrap().contains("not recovered"));
    }
}
