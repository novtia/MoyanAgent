//! In-session TodoList tool.
//!
//! Gives the model a lightweight, ephemeral task-list it can use to plan
//! and track multi-step work within a single agent run. The list lives
//! only in memory for the duration of the session; it is never written
//! to disk and is not shared across concurrent sub-agents.
//!
//! Supported operations (the `action` field):
//!
//! | action   | required fields         | description                              |
//! |----------|-------------------------|------------------------------------------|
//! | `add`    | `content`               | Append one or more items (array or str)  |
//! | `remove` | `id`                    | Delete an item by id                     |
//! | `list`   | —                       | Return all current items                 |
//! | `clear`  | —                       | Delete all items                         |
//!
//! Task completion is handled by other tools via `todo_done_id`; this tool
//! only creates and manages the list itself.
//!
//! Each item has:
//! - `id`      – stable u64 counter, assigned on insert.
//! - `content` – plain-text description.
//! - `status`  – one of `"pending"`, `"in_progress"`, `"done"`, `"cancelled"`.

use std::sync::{Arc, Mutex};

use serde_json::{json, Value};

use crate::ai::agent::tools::{Tool, ToolFuture, ToolInvocation, ToolResult, ToolSpec};
use crate::error::{AppError, AppResult};

pub const TOOL_NAME: &str = "TodoList";

#[derive(Debug, Clone)]
struct TodoItem {
    id: u64,
    content: String,
    status: String,
}

impl TodoItem {
    fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "content": self.content,
            "status": self.status,
        })
    }
}

#[derive(Debug, Default)]
struct TodoStore {
    items: Vec<TodoItem>,
    next_id: u64,
}

impl TodoStore {
    fn add(&mut self, content: String) -> &TodoItem {
        let id = self.next_id;
        self.next_id += 1;
        self.items.push(TodoItem {
            id,
            content,
            status: "pending".into(),
        });
        self.items.last().unwrap()
    }

    fn get_mut(&mut self, id: u64) -> Option<&mut TodoItem> {
        self.items.iter_mut().find(|t| t.id == id)
    }

    fn remove(&mut self, id: u64) -> bool {
        let before = self.items.len();
        self.items.retain(|t| t.id != id);
        self.items.len() < before
    }

    fn list(&self) -> Vec<Value> {
        self.items.iter().map(TodoItem::to_json).collect()
    }

    fn clear(&mut self) -> usize {
        let n = self.items.len();
        self.items.clear();
        n
    }
}

/// The TodoList tool. Each instance owns its own isolated store, so
/// concurrent sub-agents don't share each other's lists.
#[derive(Clone)]
pub struct TodoListTool {
    spec: ToolSpec,
    store: Arc<Mutex<TodoStore>>,
}

impl Default for TodoListTool {
    fn default() -> Self {
        Self::new()
    }
}

impl TodoListTool {
    /// Returns a nudge message when pending / in-progress items remain.
    /// Used by the query engine to continue the loop instead of stopping early.
    pub fn incomplete_nudge_message(&self) -> Option<String> {
        let store = self.store.lock().ok()?;
        let incomplete: Vec<_> = store
            .items
            .iter()
            .filter(|t| t.status == "pending" || t.status == "in_progress")
            .collect();
        if incomplete.is_empty() {
            return None;
        }
        let lines: Vec<String> = incomplete
            .iter()
            .map(|t| format!("- [#{}] ({}) {}", t.id, t.status, t.content))
            .collect();
        Some(format!(
            "[SYSTEM] Your task list is NOT complete. {} item(s) remain:\n{}\n\
             Continue working NOW: finish each remaining task using the appropriate \
             tool and pass `todo_done_id` with the item id when that step succeeds. \
             Do NOT call TodoList to change status. Do NOT stop with a summary or \
             final reply until every item is `done` or `cancelled`.",
            incomplete.len(),
            lines.join("\n")
        ))
    }

    /// Mark a todo item as done. Called by [`super::ToolPool`] when another
    /// tool completes with `todo_done_id`.
    pub fn mark_done(&self, id: u64) -> AppResult<Value> {
        let mut store = self
            .store
            .lock()
            .map_err(|_| AppError::Other("TodoList: store lock poisoned".into()))?;
        let item = store.get_mut(id).ok_or_else(|| {
            AppError::Invalid(format!("TodoList: item id={id} not found"))
        })?;
        item.status = "done".into();
        Ok(item.to_json())
    }

    pub fn new() -> Self {
        Self {
            store: Arc::new(Mutex::new(TodoStore::default())),
            spec: ToolSpec {
                name: TOOL_NAME.to_string(),
                description: "\
Manage an in-session task list. Follow ALL of these rules strictly.\n\n\
━━━ WHEN TO USE ━━━\n\
Only create a TodoList when the user's request involves MULTIPLE DISTINCT PHASES \
that each require a separate tool call or verification step. \
NEVER use TodoList for a task that is naturally done in one shot — just do it.\n\n\
Examples of tasks that do NOT need a TodoList:\n\
• Write a novel / story / article (one continuous output — just write it)\n\
• Answer a question\n\
• Generate a single file\n\n\
Examples that DO need a TodoList:\n\
• Multi-file refactor across many files\n\
• Research → draft → verify → publish pipeline\n\
• Any task where the user explicitly asks for a breakdown\n\n\
━━━ TASK GRANULARITY ━━━\n\
Each task must represent a meaningful, independently verifiable unit of output. \
NEVER split a single continuous action into multiple tasks. \
BAD: '写第一章', '写第二章', '写第三章' — these are all 'write content', one task.\n\
GOOD: '撰写正文内容（目标 3 万字）', '自查字数与质量是否达标' — two tasks.\n\
Keep the list as short as possible: 2–5 tasks is typical. \
If you find yourself writing 6+ tasks, reconsider — you are almost certainly \
splitting one action into meaningless micro-tasks.\n\n\
━━━ WORKFLOW (two phases only) ━━━\n\
1. PLAN once: call `add` with ALL tasks as a JSON array at the very start, e.g. \
   content: [\"撰写正文\", \"自查字数\"]. Each array element is exactly ONE task. \
   NEVER pass the whole list as one string like \"[\\\"a\\\", \\\"b\\\"]\". \
   Do NOT add more tasks later.\n\
2. EXECUTE: for each task, call the appropriate working tool and pass \
   `todo_done_id` with that task's id. On success the runtime marks it done \
   automatically. Do NOT call TodoList to change status. \
   The task title is fixed at creation time.\n\
   Never add, remove, or clear items just because you started or finished a step.\n\n\
━━━ DO NOT STOP EARLY ━━━\n\
If ANY item is still `pending` or `in_progress`, you MUST keep calling tools. \
Never end your turn with only a text summary while tasks remain — the runtime \
will reject premature completion and ask you to continue.\n\n\
Actions:\n\
• add    – ONE-TIME initialisation only.\n\
• list   – Check ids when needed.\n\
• remove – Only if the user explicitly asks to delete a task.\n\
• clear  – Only if the user explicitly asks to wipe everything.\n\n\
Status lifecycle: pending → done (via `todo_done_id` on working tools) | cancelled"
                    .to_string(),
                schema: json!({
                    "type": "object",
                    "properties": {
                        "action": {
                            "type": "string",
                            "enum": ["add", "remove", "list", "clear"],
                            "description": "The operation to perform."
                        },
                        "content": {
                            "description": "For 'add': pass multiple tasks as a JSON array of strings, e.g. \
                                [\"task one\", \"task two\"] — each element is ONE task. \
                                Do NOT put the whole array inside a single string.",
                            "oneOf": [
                                { "type": "string" },
                                { "type": "array", "items": { "type": "string" } }
                            ]
                        },
                        "id": {
                            "type": "integer",
                            "description": "Item id (required for 'remove')."
                        }
                    },
                    "required": ["action"]
                }),
                read_only: false,
                concurrency_safe: false,
            },
        }
    }
}

/// Extract optional `todo_done_id` from a tool input object.
pub fn extract_todo_done_id(input: &Value) -> Option<u64> {
    input.get("todo_done_id").and_then(Value::as_u64)
}

/// Return a copy of `input` with `todo_done_id` removed so individual tools
/// do not see an unknown field during validation.
pub fn strip_todo_done_id(input: &Value) -> Value {
    let mut out = input.clone();
    if let Some(obj) = out.as_object_mut() {
        obj.remove("todo_done_id");
    }
    out
}

/// Inject the shared `todo_done_id` property into a tool JSON schema.
pub fn inject_todo_done_schema(schema: &mut Value) {
    let Some(props) = schema.get_mut("properties").and_then(|p| p.as_object_mut()) else {
        return;
    };
    props.insert(
        "todo_done_id".into(),
        json!({
            "type": "integer",
            "description": "When this tool call completes a TodoList step, pass the item id here. \
                On success the runtime marks it done automatically. Do NOT call TodoList to change status."
        }),
    );
}

/// Expand `add` content into individual task strings.
///
/// Returns `(tasks, split_from_string)` where `split_from_string` is true when
/// a single string that looked like a JSON array was auto-split.
fn expand_add_contents(content_val: Value) -> AppResult<(Vec<String>, bool)> {
    match content_val {
        Value::String(s) => {
            let texts = parse_task_text(&s);
            let split = texts.len() > 1 && looks_like_json_array_string(&s);
            Ok((texts, split))
        }
        Value::Array(arr) => {
            let mut texts = Vec::new();
            let mut split_from_string = false;
            for v in arr {
                match v {
                    Value::String(s) => {
                        let parts = parse_task_text(&s);
                        if parts.len() > 1 && looks_like_json_array_string(&s) {
                            split_from_string = true;
                        }
                        texts.extend(parts);
                    }
                    _ => {}
                }
            }
            Ok((texts, split_from_string))
        }
        _ => Err(AppError::Invalid(
            "TodoList add: `content` must be a string or array of strings".into(),
        )),
    }
}

fn looks_like_json_array_string(s: &str) -> bool {
    let trimmed = s.trim();
    trimmed.starts_with('[') && trimmed.ends_with(']')
}

/// Parse one task string; auto-split when the model stringifies a JSON array.
fn parse_task_text(s: &str) -> Vec<String> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }

    if looks_like_json_array_string(trimmed) {
        if let Ok(Value::Array(arr)) = serde_json::from_str(trimmed) {
            let mut out = Vec::new();
            for v in arr {
                if let Value::String(text) = v {
                    let t = text.trim();
                    if !t.is_empty() {
                        out.push(t.to_string());
                    }
                }
            }
            if out.len() > 1 {
                return out;
            }
        }
    }

    vec![trimmed.to_string()]
}

impl Tool for TodoListTool {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    fn validate(&self, input: &Value) -> AppResult<()> {
        let action = input
            .get("action")
            .and_then(Value::as_str)
            .ok_or_else(|| AppError::Invalid("TodoList: `action` must be a string".into()))?;

        match action {
            "add" => {
                if input.get("content").is_none() {
                    return Err(AppError::Invalid(
                        "TodoList add: `content` is required".into(),
                    ));
                }
            }
            "remove" => {
                if input.get("id").and_then(Value::as_u64).is_none() {
                    return Err(AppError::Invalid(
                        "TodoList remove: `id` must be a non-negative integer".into(),
                    ));
                }
            }
            "list" | "clear" => {}
            other => {
                return Err(AppError::Invalid(format!(
                    "TodoList: unknown action {:?}; must be one of add|remove|list|clear",
                    other
                )));
            }
        }
        Ok(())
    }

    fn execute<'a>(&'a self, invocation: ToolInvocation<'a>) -> ToolFuture<'a> {
        let store = self.store.clone();
        Box::pin(async move {
            let action = invocation
                .input
                .get("action")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();

            let mut store = store
                .lock()
                .map_err(|_| AppError::Other("TodoList: store lock poisoned".into()))?;

            match action.as_str() {
                "add" => {
                    let content_val = invocation.input.get("content").cloned().unwrap_or(Value::Null);
                    let (texts, split_from_string) = expand_add_contents(content_val)?;
                    if texts.is_empty() {
                        return Ok(ToolResult::error("TodoList add: no content items provided"));
                    }
                    let mut added = Vec::with_capacity(texts.len());
                    for text in texts {
                        let item = store.add(text);
                        added.push(item.to_json());
                    }
                    let mut result = json!({
                        "added": added,
                        "total": store.items.len()
                    });
                    if split_from_string {
                        result["note"] = json!(
                            "Detected a JSON array written as one string; split into separate tasks. \
                             Pass `content` as a JSON array of strings, not a stringified array."
                        );
                    }
                    Ok(ToolResult::ok(result))
                }

                "remove" => {
                    let id = invocation.input.get("id").and_then(Value::as_u64).unwrap();
                    if store.remove(id) {
                        Ok(ToolResult::ok(json!({
                            "removed_id": id,
                            "total": store.items.len()
                        })))
                    } else {
                        Ok(ToolResult::error(format!(
                            "TodoList remove: item id={id} not found"
                        )))
                    }
                }

                "list" => {
                    let items = store.list();
                    let total = items.len();
                    Ok(ToolResult::ok(json!({
                        "items": items,
                        "total": total
                    })))
                }

                "clear" => {
                    let removed = store.clear();
                    Ok(ToolResult::ok(json!({ "cleared": removed })))
                }

                other => Ok(ToolResult::error(format!(
                    "TodoList: unknown action {other:?}"
                ))),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn incomplete_nudge_when_items_open() {
        let tool = TodoListTool::new();
        {
            let mut store = tool.store.lock().unwrap();
            store.add("task a".into());
            store.add("task b".into());
            store.items[0].status = "done".into();
        }
        let msg = tool.incomplete_nudge_message().expect("nudge");
        assert!(msg.contains("NOT complete"));
        assert!(msg.contains("task b"));
        assert!(msg.contains("todo_done_id"));
    }

    #[test]
    fn no_nudge_when_all_done() {
        let tool = TodoListTool::new();
        {
            let mut store = tool.store.lock().unwrap();
            let id = store.add("task".into()).id;
            store.get_mut(id).unwrap().status = "done".into();
        }
        assert!(tool.incomplete_nudge_message().is_none());
    }

    #[test]
    fn mark_done_preserves_title() {
        let tool = TodoListTool::new();
        let id = {
            let mut store = tool.store.lock().unwrap();
            store.add("重写 P013".into()).id
        };
        let updated = tool.mark_done(id).unwrap();
        assert_eq!(updated["content"], "重写 P013");
        assert_eq!(updated["status"], "done");
    }

    #[test]
    fn mark_done_unknown_id_errors() {
        let tool = TodoListTool::new();
        assert!(tool.mark_done(999).is_err());
    }

    #[test]
    fn expand_splits_stringified_json_array() {
        let raw = r#"["阅读并解析原第九章", "撰写第九章正文", "对正文进行字数估算"]"#;
        let (texts, split) = expand_add_contents(Value::String(raw.into())).unwrap();
        assert!(split);
        assert_eq!(texts.len(), 3);
        assert_eq!(texts[0], "阅读并解析原第九章");
    }

    #[test]
    fn expand_keeps_single_task_string() {
        let (texts, split) =
            expand_add_contents(Value::String("撰写第九章正文".into())).unwrap();
        assert!(!split);
        assert_eq!(texts, vec!["撰写第九章正文".to_string()]);
    }

    #[test]
    fn expand_splits_array_element_that_is_stringified_json_array() {
        let raw = json!([
            r#"["任务一", "任务二", "任务三"]"#
        ]);
        let (texts, split) = expand_add_contents(raw).unwrap();
        assert!(split);
        assert_eq!(texts.len(), 3);
    }

    #[test]
    fn strip_todo_done_id_removes_field() {
        let input = json!({ "path": "foo.txt", "todo_done_id": 1 });
        let stripped = strip_todo_done_id(&input);
        assert!(stripped.get("todo_done_id").is_none());
        assert_eq!(stripped["path"], "foo.txt");
    }

    #[test]
    fn extract_todo_done_id_reads_field() {
        let input = json!({ "todo_done_id": 42 });
        assert_eq!(extract_todo_done_id(&input), Some(42));
    }
}
