//! In-session TodoList tool.
//!
//! Gives the model a lightweight, ephemeral task-list it can use to plan
//! and track multi-step work within a single agent run. Lists are kept in
//! memory only, keyed per `(session, agent)` scope so that concurrent
//! sessions and sub-agents never see each other's items; the engine clears
//! a scope when its run ends.
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
//! Historical `create`/`update` tool rounds are **not** kept in the prompt.
//! Instead the query engine injects [`TodoListTool::prompt_snapshot`] at the
//! **end** of every model request so a long context cannot bury the list.
//!
//! Each item has:
//! - `id`     – simple sequential number assigned on creation (1, 2, 3, …).
//! - `title`  – short task title (immutable).
//! - `detail` – longer task description (immutable, may be empty).
//! - `status` – one of `"pending"`, `"in_progress"`, `"done"`, `"cancelled"`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};

use crate::ai::agent::core::context::ToolUseContext;
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

    fn checklist(&self) -> String {
        self.items
            .iter()
            .map(|t| {
                let mark = status_mark(&t.status);
                if t.detail.is_empty() {
                    format!("{mark} #{} {} [{}]", t.id, t.title, t.status)
                } else {
                    format!(
                        "{mark} #{} {} [{}] — {}",
                        t.id, t.title, t.status, t.detail
                    )
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn has_incomplete(&self) -> bool {
        self.items
            .iter()
            .any(|t| t.status == "pending" || t.status == "in_progress")
    }
}

/// `✔` is the only completed mark. Anything else is unfinished work.
fn status_mark(status: &str) -> &'static str {
    match status {
        "done" => "✔",
        "cancelled" => "✕",
        "in_progress" => "►",
        _ => "☐",
    }
}

/// Scope key isolating one agent run from every other session / sub-agent.
pub fn scope_key(ctx: &ToolUseContext) -> String {
    format!("{}#{}", ctx.session_id.as_deref().unwrap_or("-"), ctx.agent_id)
}

/// The TodoList tool. One process-wide instance holds a map of per-run
/// stores so concurrent sessions and sub-agents never share lists.
#[derive(Clone)]
pub struct TodoListTool {
    spec: ToolSpec,
    stores: Arc<Mutex<HashMap<String, TodoStore>>>,
}

impl Default for TodoListTool {
    fn default() -> Self {
        Self::new()
    }
}

impl TodoListTool {
    /// Compact live checklist injected at the end of every model request.
    ///
    /// Placed last so recency bias still sees it after a long transcript or
    /// a compaction summary. `premature_stop` adds a "do not halt" rider
    /// when the model tried to end the turn with unfinished items remaining.
    pub fn prompt_snapshot(&self, ctx: &ToolUseContext, premature_stop: bool) -> Option<String> {
        self.prompt_snapshot_for_key(&scope_key(ctx), premature_stop)
    }

    fn prompt_snapshot_for_key(&self, key: &str, premature_stop: bool) -> Option<String> {
        let stores = self.stores.lock().ok()?;
        let store = stores.get(key)?;
        if !store.created || store.items.is_empty() {
            return None;
        }
        let mut body = format!(
            "<todolist>\n\
             Live task list (authoritative — ignore earlier TodoList tool results).\n\
             Only ✔ is complete. ☐ / ► items are NOT done and still need work.\n\
             Do not stop until every item is ✔ (done) or ✕ (cancelled).\n\
             {}\n</todolist>",
            store.checklist()
        );
        if premature_stop && store.has_incomplete() {
            body.push_str(
                "\n\n[SYSTEM] You tried to stop with unfinished items. \
                 Do NOT write a final reply. Finish each ☐ / ► item, then call \
                 TodoList `update` to set its status to `done` (✔).",
            );
        }
        Some(body)
    }

    /// Returns a nudge when pending / in_progress items remain in this
    /// run's list. Used by the query engine to continue the loop.
    pub fn incomplete_nudge_message(&self, ctx: &ToolUseContext) -> Option<String> {
        self.incomplete_nudge_for_key(&scope_key(ctx))
    }

    fn incomplete_nudge_for_key(&self, key: &str) -> Option<String> {
        {
            let stores = self.stores.lock().ok()?;
            let store = stores.get(key)?;
            if !store.has_incomplete() {
                return None;
            }
        }
        self.prompt_snapshot_for_key(key, true)
    }

    /// Drop this run's list so the next generation can `create` again and
    /// the in-memory map does not grow without bound.
    pub fn clear_scope(&self, ctx: &ToolUseContext) {
        if let Ok(mut stores) = self.stores.lock() {
            stores.remove(&scope_key(ctx));
        }
    }

    pub fn new() -> Self {
        Self {
            stores: Arc::new(Mutex::new(HashMap::new())),
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
━━━ LIVE CHECKLIST ━━━\n\
The runtime injects a live ✔/☐ checklist at the END of every model turn. \
That snapshot is the source of truth; earlier TodoList tool results may be \
dropped from context. Only ✔ means done. ☐ or ► means the item is unfinished \
and you must keep working.\n\n\
━━━ DO NOT STOP EARLY ━━━\n\
If ANY item is still `pending` or `in_progress` (no ✔), you MUST keep working \
and then call `update` to mark it `done`. Never end your turn with only a text \
summary while ☐ items remain — the runtime will reject premature completion.\n\n\
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
        let stores = self.stores.clone();
        let key = scope_key(invocation.context);
        Box::pin(async move {
            let action = invocation
                .input
                .get("action")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();

            let tasks = invocation.input.get("tasks").cloned().unwrap_or(Value::Null);

            let mut stores = stores
                .lock()
                .map_err(|_| AppError::Other("TodoList: store lock poisoned".into()))?;
            let store = stores.entry(key).or_default();

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
            let mut stores = tool.stores.lock().unwrap();
            let store = stores.entry("s#a".into()).or_default();
            store.create(vec![
                ("task a".into(), String::new()),
                ("task b".into(), String::new()),
            ]);
            store.items[0].status = "done".into();
        }
        let msg = tool.incomplete_nudge_for_key("s#a").expect("nudge");
        assert!(msg.contains("✔"));
        assert!(msg.contains("☐"));
        assert!(msg.contains("task b"));
        assert!(msg.contains("You tried to stop"));
        assert!(tool.incomplete_nudge_for_key("other").is_none());
    }

    #[test]
    fn snapshot_is_injected_even_when_all_done() {
        let tool = TodoListTool::new();
        {
            let mut stores = tool.stores.lock().unwrap();
            let store = stores.entry("s#a".into()).or_default();
            store.create(vec![("task".into(), String::new())]);
            store.get_mut(1).unwrap().status = "done".into();
        }
        let snap = tool.prompt_snapshot_for_key("s#a", false).expect("snapshot");
        assert!(snap.contains("✔ #1 task [done]"));
        assert!(!snap.contains("You tried to stop"));
        assert!(tool.prompt_snapshot_for_key("missing", false).is_none());
    }

    #[test]
    fn no_nudge_when_all_done() {
        let tool = TodoListTool::new();
        {
            let mut stores = tool.stores.lock().unwrap();
            let store = stores.entry("s#a".into()).or_default();
            store.create(vec![("task".into(), String::new())]);
            store.get_mut(1).unwrap().status = "done".into();
        }
        assert!(tool.incomplete_nudge_for_key("s#a").is_none());
    }

    #[test]
    fn clear_scope_drops_the_list() {
        let tool = TodoListTool::new();
        {
            let mut stores = tool.stores.lock().unwrap();
            stores.entry("sess#agent".into()).or_default().create(vec![
                ("task".into(), String::new()),
            ]);
        }
        assert!(tool.incomplete_nudge_for_key("sess#agent").is_some());
        if let Ok(mut stores) = tool.stores.lock() {
            stores.remove("sess#agent");
        }
        assert!(tool.incomplete_nudge_for_key("sess#agent").is_none());
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
