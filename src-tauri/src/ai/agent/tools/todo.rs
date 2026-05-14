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
//! | `update` | `id`, `content`/`status`| Edit text or status of an existing item  |
//! | `remove` | `id`                    | Delete an item by id                     |
//! | `list`   | —                       | Return all current items                 |
//! | `clear`  | —                       | Delete all items                         |
//!
//! Each item has:
//! - `id`      – stable u64 counter, assigned on insert.
//! - `content` – plain-text description.
//! - `status`  – one of `"pending"`, `"in_progress"`, `"done"`, `"cancelled"`.

use std::sync::{Arc, Mutex};

use serde_json::{json, Value};

use crate::ai::agent::tools::{Tool, ToolFuture, ToolInvocation, ToolResult, ToolSpec};
use crate::error::{AppError, AppResult};

const TOOL_NAME: &str = "TodoList";

/// Valid status values understood by the tool.
const VALID_STATUSES: &[&str] = &["pending", "in_progress", "done", "cancelled"];

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
    pub fn new() -> Self {
        Self {
            store: Arc::new(Mutex::new(TodoStore::default())),
            spec: ToolSpec {
                name: TOOL_NAME.to_string(),
                description: "\
Manage an in-session to-do list to plan and track multi-step work. \
Use it to break down complex tasks, record progress, and avoid losing track \
of pending steps during a long agent run. \
The list is ephemeral — it lives only for the current session.\n\n\
Actions:\n\
• add    – add one item (content: string) or multiple (content: array of strings)\n\
• update – change content and/or status of an existing item (id required)\n\
• remove – delete an item by id\n\
• list   – return all items\n\
• clear  – delete all items\n\n\
Status values: pending | in_progress | done | cancelled"
                    .to_string(),
                schema: json!({
                    "type": "object",
                    "properties": {
                        "action": {
                            "type": "string",
                            "enum": ["add", "update", "remove", "list", "clear"],
                            "description": "The operation to perform."
                        },
                        "content": {
                            "description": "For 'add': a single string or an array of strings. For 'update': the new text.",
                            "oneOf": [
                                { "type": "string" },
                                { "type": "array", "items": { "type": "string" } }
                            ]
                        },
                        "id": {
                            "type": "integer",
                            "description": "Item id (required for 'update' and 'remove')."
                        },
                        "status": {
                            "type": "string",
                            "enum": ["pending", "in_progress", "done", "cancelled"],
                            "description": "New status (used with 'update')."
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
            "update" => {
                if input.get("id").and_then(Value::as_u64).is_none() {
                    return Err(AppError::Invalid(
                        "TodoList update: `id` must be a non-negative integer".into(),
                    ));
                }
                if let Some(s) = input.get("status").and_then(Value::as_str) {
                    if !VALID_STATUSES.contains(&s) {
                        return Err(AppError::Invalid(format!(
                            "TodoList update: invalid status {:?}; must be one of {:?}",
                            s, VALID_STATUSES
                        )));
                    }
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
                    "TodoList: unknown action {:?}; must be one of add|update|remove|list|clear",
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
                    let texts: Vec<String> = match content_val {
                        Value::String(s) => vec![s],
                        Value::Array(arr) => arr
                            .into_iter()
                            .filter_map(|v| v.as_str().map(str::to_string))
                            .collect(),
                        _ => {
                            return Ok(ToolResult::error(
                                "TodoList add: `content` must be a string or array of strings",
                            ));
                        }
                    };
                    if texts.is_empty() {
                        return Ok(ToolResult::error("TodoList add: no content items provided"));
                    }
                    let mut added = Vec::with_capacity(texts.len());
                    for text in texts {
                        let trimmed = text.trim().to_string();
                        if trimmed.is_empty() {
                            continue;
                        }
                        let item = store.add(trimmed);
                        added.push(item.to_json());
                    }
                    Ok(ToolResult::ok(json!({
                        "added": added,
                        "total": store.items.len()
                    })))
                }

                "update" => {
                    let id = invocation.input.get("id").and_then(Value::as_u64).unwrap();
                    let item = store.get_mut(id).ok_or_else(|| {
                        AppError::Invalid(format!("TodoList update: item id={id} not found"))
                    })?;
                    if let Some(c) = invocation.input.get("content").and_then(Value::as_str) {
                        let trimmed = c.trim().to_string();
                        if !trimmed.is_empty() {
                            item.content = trimmed;
                        }
                    }
                    if let Some(s) = invocation.input.get("status").and_then(Value::as_str) {
                        item.status = s.to_string();
                    }
                    let snapshot = item.to_json();
                    Ok(ToolResult::ok(json!({ "updated": snapshot })))
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
