//! In-session TodoList tool.
//!
//! Gives the model a lightweight, ephemeral task-list it can use to plan
//! and track multi-step work within a single agent run. The list lives
//! only in memory for the duration of the session; it is never written
//! to disk and is not shared across concurrent sub-agents.
//!
//! Supported operations (the `action` field):
//!
//! | action   | required fields | description                                  |
//! |----------|-----------------|----------------------------------------------|
//! | `create` | `tasks`         | ONE-TIME: create the whole list at once      |
//! | `update` | `tasks`         | Update the status of one or more items       |
//!
//! The TodoList maintains its own state entirely. Task titles and details
//! are frozen at creation time; only their `status` can change via `update`.
//!
//! Each item has:
//! - `id`     – simple sequential number assigned on creation (1, 2, 3, …).
//! - `title`  – short task title (immutable).
//! - `detail` – longer task description (immutable, may be empty).
//! - `status` – one of `"pending"`, `"in_progress"`, `"done"`, `"cancelled"`.

use std::sync::{Arc, Mutex};

use serde_json::{json, Value};

use crate::ai::agent::tools::{Tool, ToolFuture, ToolInvocation, ToolResult, ToolSpec};
use crate::error::{AppError, AppResult};

pub const TOOL_NAME: &str = "TodoList";

const VALID_STATUSES: [&str; 4] = ["pending", "in_progress", "done", "cancelled"];

#[derive(Debug, Clone)]
struct TodoItem {
    id: u64,
    title: String,
    detail: String,
    status: String,
}

impl TodoItem {
    fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "title": self.title,
            "detail": self.detail,
            "status": self.status,
        })
    }
}

#[derive(Debug, Default)]
struct TodoStore {
    items: Vec<TodoItem>,
    created: bool,
}

impl TodoStore {
    /// Create the whole list at once. Assigns sequential ids starting at 1.
    fn create(&mut self, tasks: Vec<(String, String)>) {
        self.items = tasks
            .into_iter()
            .enumerate()
            .map(|(idx, (title, detail))| TodoItem {
                id: (idx as u64) + 1,
                title,
                detail,
                status: "pending".into(),
            })
            .collect();
        self.created = true;
    }

    fn get_mut(&mut self, id: u64) -> Option<&mut TodoItem> {
        self.items.iter_mut().find(|t| t.id == id)
    }

    fn list(&self) -> Vec<Value> {
        self.items.iter().map(TodoItem::to_json).collect()
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
            .map(|t| format!("- [#{}] ({}) {}", t.id, t.status, t.title))
            .collect();
        Some(format!(
            "[SYSTEM] Your task list is NOT complete. {} item(s) remain:\n{}\n\
             Continue working NOW: finish each remaining task using the appropriate \
             tool, then call TodoList with action `update` to set that item's status \
             to `done`. Do NOT stop with a summary or final reply until every item is \
             `done` or `cancelled`.",
            incomplete.len(),
            lines.join("\n")
        ))
    }

    pub fn new() -> Self {
        Self {
            store: Arc::new(Mutex::new(TodoStore::default())),
            spec: ToolSpec {
                name: TOOL_NAME.to_string(),
                description: "\
Manage an in-session task list that tracks its OWN state. Follow ALL rules strictly.\n\n\
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
━━━ WORKFLOW (two actions only) ━━━\n\
1. CREATE once: call `create` with ALL tasks at the very start. `tasks` is a JSON \
   array of objects, each { title, detail }. The runtime assigns simple numeric \
   ids automatically (1, 2, 3, …) — do NOT invent ids. You may only call `create` \
   ONCE per session; titles and details are then FIXED.\n\
2. UPDATE status: as work progresses, call `update` with `tasks` as an array of \
   { id, status } to mark items `in_progress`, `done`, or `cancelled`. \
   Only the status changes — titles and details are immutable.\n\n\
━━━ DO NOT STOP EARLY ━━━\n\
If ANY item is still `pending` or `in_progress`, you MUST keep working and then \
call `update` to advance its status. Never end your turn with only a text summary \
while tasks remain — the runtime will reject premature completion and ask you to \
continue.\n\n\
Status lifecycle: pending → in_progress → done | cancelled"
                    .to_string(),
                schema: json!({
                    "type": "object",
                    "properties": {
                        "action": {
                            "type": "string",
                            "enum": ["create", "update"],
                            "description": "The operation to perform. `create` builds the whole list once; `update` changes item status."
                        },
                        "tasks": {
                            "type": "array",
                            "description": "For `create`: array of { title, detail } objects — the runtime assigns numeric ids 1,2,3,…, so do NOT include id. \
                                For `update`: array of { id, status } objects — only status may change.",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "id": {
                                        "type": "integer",
                                        "description": "Item id (required for `update`; ignored for `create`)."
                                    },
                                    "title": {
                                        "type": "string",
                                        "description": "Short task title (required for `create`)."
                                    },
                                    "detail": {
                                        "type": "string",
                                        "description": "Longer task description (optional for `create`)."
                                    },
                                    "status": {
                                        "type": "string",
                                        "enum": ["pending", "in_progress", "done", "cancelled"],
                                        "description": "New status (required for `update`)."
                                    }
                                }
                            }
                        }
                    },
                    "required": ["action", "tasks"]
                }),
                read_only: false,
                concurrency_safe: false,
            },
        }
    }
}

/// Parse the `tasks` array for `create` into `(title, detail)` pairs.
fn parse_create_tasks(tasks: &Value) -> AppResult<Vec<(String, String)>> {
    let arr = tasks.as_array().ok_or_else(|| {
        AppError::Invalid("TodoList create: `tasks` must be an array of objects".into())
    })?;
    if arr.is_empty() {
        return Err(AppError::Invalid(
            "TodoList create: `tasks` must contain at least one task".into(),
        ));
    }
    let mut out = Vec::with_capacity(arr.len());
    for (idx, v) in arr.iter().enumerate() {
        let obj = v.as_object().ok_or_else(|| {
            AppError::Invalid(format!(
                "TodoList create: task at index {idx} must be an object with `title`"
            ))
        })?;
        let title = obj
            .get("title")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                AppError::Invalid(format!(
                    "TodoList create: task at index {idx} is missing a non-empty `title`"
                ))
            })?;
        let detail = obj
            .get("detail")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string();
        out.push((title.to_string(), detail));
    }
    Ok(out)
}

/// Parse the `tasks` array for `update` into `(id, status)` pairs.
fn parse_update_tasks(tasks: &Value) -> AppResult<Vec<(u64, String)>> {
    let arr = tasks.as_array().ok_or_else(|| {
        AppError::Invalid("TodoList update: `tasks` must be an array of objects".into())
    })?;
    if arr.is_empty() {
        return Err(AppError::Invalid(
            "TodoList update: `tasks` must contain at least one item".into(),
        ));
    }
    let mut out = Vec::with_capacity(arr.len());
    for (idx, v) in arr.iter().enumerate() {
        let obj = v.as_object().ok_or_else(|| {
            AppError::Invalid(format!(
                "TodoList update: task at index {idx} must be an object with `id` and `status`"
            ))
        })?;
        let id = obj.get("id").and_then(Value::as_u64).ok_or_else(|| {
            AppError::Invalid(format!(
                "TodoList update: task at index {idx} is missing a valid integer `id`"
            ))
        })?;
        let status = obj
            .get("status")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                AppError::Invalid(format!(
                    "TodoList update: task at index {idx} is missing a `status`"
                ))
            })?;
        if !VALID_STATUSES.contains(&status) {
            return Err(AppError::Invalid(format!(
                "TodoList update: invalid status {status:?}; must be one of pending|in_progress|done|cancelled"
            )));
        }
        out.push((id, status.to_string()));
    }
    Ok(out)
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
            "create" => {
                parse_create_tasks(input.get("tasks").unwrap_or(&Value::Null))?;
            }
            "update" => {
                parse_update_tasks(input.get("tasks").unwrap_or(&Value::Null))?;
            }
            other => {
                return Err(AppError::Invalid(format!(
                    "TodoList: unknown action {other:?}; must be one of create|update"
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

            let tasks = invocation.input.get("tasks").cloned().unwrap_or(Value::Null);

            let mut store = store
                .lock()
                .map_err(|_| AppError::Other("TodoList: store lock poisoned".into()))?;

            match action.as_str() {
                "create" => {
                    if store.created {
                        return Ok(ToolResult::error(
                            "TodoList create: the list was already created. Use action `update` \
                             to change item status; titles and details are fixed.",
                        ));
                    }
                    let parsed = parse_create_tasks(&tasks)?;
                    store.create(parsed);
                    Ok(ToolResult::ok(json!({
                        "items": store.list(),
                        "total": store.items.len()
                    })))
                }

                "update" => {
                    if !store.created {
                        return Ok(ToolResult::error(
                            "TodoList update: no list exists yet. Call action `create` first.",
                        ));
                    }
                    let parsed = parse_update_tasks(&tasks)?;
                    for (id, _) in &parsed {
                        if store.get_mut(*id).is_none() {
                            return Ok(ToolResult::error(format!(
                                "TodoList update: item id={id} not found"
                            )));
                        }
                    }
                    for (id, status) in parsed {
                        if let Some(item) = store.get_mut(id) {
                            item.status = status;
                        }
                    }
                    Ok(ToolResult::ok(json!({
                        "items": store.list(),
                        "total": store.items.len()
                    })))
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
    fn create_assigns_sequential_ids() {
        let mut store = TodoStore::default();
        store.create(vec![
            ("撰写正文".into(), "目标 3 万字".into()),
            ("自查字数".into(), String::new()),
        ]);
        assert_eq!(store.items.len(), 2);
        assert_eq!(store.items[0].id, 1);
        assert_eq!(store.items[1].id, 2);
        assert_eq!(store.items[0].title, "撰写正文");
        assert_eq!(store.items[0].detail, "目标 3 万字");
        assert_eq!(store.items[0].status, "pending");
    }

    #[test]
    fn parse_create_tasks_requires_title() {
        let tasks = json!([{ "detail": "no title" }]);
        assert!(parse_create_tasks(&tasks).is_err());
    }

    #[test]
    fn parse_create_tasks_ignores_supplied_id() {
        let tasks = json!([
            { "id": 99, "title": "任务一", "detail": "详情一" },
            { "id": 5, "title": "任务二" }
        ]);
        let parsed = parse_create_tasks(&tasks).unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0], ("任务一".to_string(), "详情一".to_string()));
        assert_eq!(parsed[1], ("任务二".to_string(), String::new()));
    }

    #[test]
    fn parse_update_tasks_validates_status() {
        let bad = json!([{ "id": 1, "status": "bogus" }]);
        assert!(parse_update_tasks(&bad).is_err());
        let good = json!([{ "id": 1, "status": "done" }]);
        let parsed = parse_update_tasks(&good).unwrap();
        assert_eq!(parsed, vec![(1u64, "done".to_string())]);
    }

    #[test]
    fn parse_update_tasks_requires_id() {
        let tasks = json!([{ "status": "done" }]);
        assert!(parse_update_tasks(&tasks).is_err());
    }

    #[test]
    fn incomplete_nudge_when_items_open() {
        let tool = TodoListTool::new();
        {
            let mut store = tool.store.lock().unwrap();
            store.create(vec![
                ("task a".into(), String::new()),
                ("task b".into(), String::new()),
            ]);
            store.items[0].status = "done".into();
        }
        let msg = tool.incomplete_nudge_message().expect("nudge");
        assert!(msg.contains("NOT complete"));
        assert!(msg.contains("task b"));
        assert!(msg.contains("update"));
    }

    #[test]
    fn no_nudge_when_all_done() {
        let tool = TodoListTool::new();
        {
            let mut store = tool.store.lock().unwrap();
            store.create(vec![("task".into(), String::new())]);
            store.get_mut(1).unwrap().status = "done".into();
        }
        assert!(tool.incomplete_nudge_message().is_none());
    }

    #[test]
    fn update_preserves_title_and_detail() {
        let mut store = TodoStore::default();
        store.create(vec![("重写 P013".into(), "细节说明".into())]);
        store.get_mut(1).unwrap().status = "done".into();
        let item = &store.items[0];
        assert_eq!(item.title, "重写 P013");
        assert_eq!(item.detail, "细节说明");
        assert_eq!(item.status, "done");
    }
}
