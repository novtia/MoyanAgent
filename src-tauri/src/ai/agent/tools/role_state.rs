//! Incremental role-state tool.
//!
//! Drives the per-conversation "character state board". Unlike [`super::todo`]
//! the store is **session-scoped and shared** across every agent run that
//! belongs to the same chat session, so the dedicated `role-state` sub-agent
//! can read what previous turns established and only emit the *delta*.
//!
//! Semantics are deliberately incremental (NEVER full-replace):
//!
//! | action   | required fields                | description                              |
//! |----------|--------------------------------|------------------------------------------|
//! | `get`    | —                              | Return every role currently on the board |
//! | `create` | `id`, `role`                   | Insert a brand-new role (stable `id`)    |
//! | `update` | `id`, (`set` and/or `unset`)   | Field-level dot-path edits / deletions   |
//! | `delete` | `id`                           | Remove one role entirely                 |
//!
//! `set` is a flat map of dot-paths → values, e.g.
//! `{ "attributes.好感": 80, "mood": "羞涩" }`. `unset` is an array of
//! dot-paths to remove, e.g. `["tags.1", "nsfw.敏感点"]`.
//!
//! Each role is a free-form JSON object; the recommended shape (encoded in
//! the schema description) favours numeric/structured fields so the UI can
//! render polygons/bars instead of walls of text:
//!
//! ```json
//! {
//!   "id": "rin", "name": "凛", "gender": "female",
//!   "location": "…", "mood": "…", "outfit": "…",
//!   "attributes": { "好感": 72, "信任": 55 },
//!   "meters": { "体力": { "value": 80, "max": 100 } },
//!   "tags": ["害羞"],
//!   "nsfw": {
//!     "arousal": 40, "wetness": 55, "status": "迷离",
//!     "sensitive_spots": ["颈部"],
//!     "semen": {
//!       "exterior": "脸颊与胸前少量白浊",
//!       "swallowed": 12.5, "vaginal": 8.0, "anal": 0
//!     }
//!   }
//! }
//! ```
//! `gender` is `"male"` or `"female"` (required on `create`).
//! Semen fields under `nsfw.semen` depend on gender — use English keys only:
//! - **male** → `texture` (TEXT: semen quality / viscosity / warmth).
//! - **female** → `exterior` (TEXT: external residue) plus `swallowed` /
//!   `vaginal` / `anal` as millilitre amounts (ml, NOT 0-100).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde_json::{json, Map, Value};

use crate::ai::agent::tools::{Tool, ToolFuture, ToolInvocation, ToolResult, ToolSpec};
use crate::error::{AppError, AppResult};

pub const TOOL_NAME: &str = "RoleState";

/// Session-scoped, shared store of role boards. Lives on `AppState` so the
/// `role-state` sub-agent, the persistence layer and the Tauri commands all
/// see the same in-memory truth.
#[derive(Debug, Default)]
pub struct RoleStateStore {
    /// session_id → ordered list of role objects (insertion order preserved).
    sessions: Mutex<HashMap<String, Vec<Value>>>,
}

impl RoleStateStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Snapshot every role for a session as a JSON array (insertion order).
    pub fn snapshot(&self, session_id: &str) -> Vec<Value> {
        self.sessions
            .lock()
            .ok()
            .and_then(|g| g.get(session_id).cloned())
            .unwrap_or_default()
    }

    /// Replace a session's roles wholesale. Used when loading a persisted
    /// snapshot back into memory (session open / rollback).
    pub fn load(&self, session_id: &str, roles: Vec<Value>) {
        if let Ok(mut g) = self.sessions.lock() {
            if roles.is_empty() {
                g.remove(session_id);
            } else {
                g.insert(session_id.to_string(), roles);
            }
        }
    }

    /// Drop a session's roles entirely (e.g. when no snapshot remains).
    pub fn clear(&self, session_id: &str) {
        if let Ok(mut g) = self.sessions.lock() {
            g.remove(session_id);
        }
    }

    fn create(&self, session_id: &str, id: &str, role: Value) -> AppResult<Value> {
        let mut role = match role {
            Value::Object(m) => m,
            _ => {
                return Err(AppError::Invalid(
                    "RoleState create: `role` must be an object".into(),
                ))
            }
        };
        role.insert("id".into(), Value::String(id.to_string()));
        let role = Value::Object(role);

        let mut g = self
            .sessions
            .lock()
            .map_err(|_| AppError::Other("RoleState: store lock poisoned".into()))?;
        let list = g.entry(session_id.to_string()).or_default();
        match list.iter_mut().find(|r| role_id(r) == Some(id)) {
            Some(slot) => {
                // Idempotent create: overwrite the existing entry.
                *slot = role.clone();
            }
            None => list.push(role.clone()),
        }
        Ok(role)
    }

    fn update(
        &self,
        session_id: &str,
        id: &str,
        set: Option<&Map<String, Value>>,
        unset: &[String],
    ) -> AppResult<Value> {
        let mut g = self
            .sessions
            .lock()
            .map_err(|_| AppError::Other("RoleState: store lock poisoned".into()))?;
        let list = g
            .get_mut(session_id)
            .ok_or_else(|| AppError::Invalid("RoleState update: no roles for session".into()))?;
        let role = list
            .iter_mut()
            .find(|r| role_id(r) == Some(id))
            .ok_or_else(|| AppError::Invalid(format!("RoleState update: unknown role id {id:?}")))?;

        if let Some(set) = set {
            for (path, value) in set {
                set_dot_path(role, path, value.clone());
            }
        }
        for path in unset {
            unset_dot_path(role, path);
        }
        Ok(role.clone())
    }

    fn delete(&self, session_id: &str, id: &str) -> AppResult<bool> {
        let mut g = self
            .sessions
            .lock()
            .map_err(|_| AppError::Other("RoleState: store lock poisoned".into()))?;
        let Some(list) = g.get_mut(session_id) else {
            return Ok(false);
        };
        let before = list.len();
        list.retain(|r| role_id(r) != Some(id));
        let removed = list.len() < before;
        if list.is_empty() {
            g.remove(session_id);
        }
        Ok(removed)
    }
}

fn role_id(role: &Value) -> Option<&str> {
    role.get("id").and_then(Value::as_str)
}

/// Set a value at a dot-path, creating intermediate objects as needed.
/// Pure-numeric path segments index into arrays when the parent is one.
fn set_dot_path(root: &mut Value, path: &str, value: Value) {
    let parts: Vec<&str> = path.split('.').collect();
    let mut cur = root;
    for (i, part) in parts.iter().enumerate() {
        let last = i == parts.len() - 1;
        if last {
            assign_segment(cur, part, value);
            return;
        }
        cur = descend_segment(cur, part);
    }
}

fn assign_segment(parent: &mut Value, seg: &str, value: Value) {
    match parent {
        Value::Array(arr) => {
            if let Ok(idx) = seg.parse::<usize>() {
                if idx < arr.len() {
                    arr[idx] = value;
                } else {
                    arr.push(value);
                }
            }
        }
        Value::Object(map) => {
            map.insert(seg.to_string(), value);
        }
        other => {
            let mut map = Map::new();
            map.insert(seg.to_string(), value);
            *other = Value::Object(map);
        }
    }
}

fn descend_segment<'a>(parent: &'a mut Value, seg: &str) -> &'a mut Value {
    if !parent.is_object() && !parent.is_array() {
        *parent = Value::Object(Map::new());
    }
    match parent {
        Value::Object(map) => map
            .entry(seg.to_string())
            .or_insert_with(|| Value::Object(Map::new())),
        Value::Array(arr) => {
            let idx = seg.parse::<usize>().unwrap_or(0);
            while arr.len() <= idx {
                arr.push(Value::Object(Map::new()));
            }
            &mut arr[idx]
        }
        // Unreachable after the coercion above, but the borrow checker
        // wants every arm to yield a reference.
        other => other,
    }
}

fn unset_dot_path(root: &mut Value, path: &str) {
    let parts: Vec<&str> = path.split('.').collect();
    let mut cur = root;
    for (i, part) in parts.iter().enumerate() {
        let last = i == parts.len() - 1;
        if last {
            match cur {
                Value::Object(map) => {
                    map.remove(*part);
                }
                Value::Array(arr) => {
                    if let Ok(idx) = part.parse::<usize>() {
                        if idx < arr.len() {
                            arr.remove(idx);
                        }
                    }
                }
                _ => {}
            }
            return;
        }
        cur = match cur {
            Value::Object(map) => match map.get_mut(*part) {
                Some(v) => v,
                None => return,
            },
            Value::Array(arr) => match part.parse::<usize>().ok().and_then(|i| arr.get_mut(i)) {
                Some(v) => v,
                None => return,
            },
            _ => return,
        };
    }
}

/// The `RoleState` tool. Holds a shared [`RoleStateStore`] reference and the
/// session id is read off [`ToolUseContext`] at execution time.
#[derive(Clone)]
pub struct RoleStateTool {
    spec: ToolSpec,
    store: Arc<RoleStateStore>,
}

impl RoleStateTool {
    pub fn new(store: Arc<RoleStateStore>) -> Self {
        Self {
            store,
            spec: ToolSpec {
                name: TOOL_NAME.to_string(),
                description: "\
Maintain the per-conversation character state board. STRICTLY INCREMENTAL — \
never re-send a whole role you have already created.\n\n\
━━━ WORKFLOW ━━━\n\
1. Call `get` FIRST to see which roles already exist and their current fields.\n\
2. For a character that does NOT exist yet → `create` with a stable, \
lowercase ascii `id` (e.g. \"rin\", \"alice\") plus the initial `role` object.\n\
3. For a character that already exists → `update` ONLY the fields that changed, \
using dot-paths in `set` / `unset`. Do NOT recreate it.\n\
4. For a character that left the scene for good → `delete`.\n\n\
━━━ ROLE OBJECT SHAPE (prefer numbers over prose; English keys for nsfw) ━━━\n\
{\n  \"name\": \"凛\", \"gender\": \"female\",\n  \"location\": \"…\", \"mood\": \"…\", \"outfit\": \"…\",\n  \"attributes\": { \"好感\": 72, \"信任\": 55 },   // 0-100, radar polygon\n  \"meters\": { \"体力\": { \"value\": 80, \"max\": 100 } }, // bars\n  \"tags\": [\"害羞\"],\n  \"nsfw\": {\n    \"arousal\": 40, \"wetness\": 55, \"status\": \"迷离\",\n    \"sensitive_spots\": [\"颈部\"],\n    \"semen\": { \"exterior\": \"小腹至大腿漫溢\", \"swallowed\": 850, \"vaginal\": 2400, \"anal\": 0 }\n  }\n}\n\
`gender` — REQUIRED on `create`: `\"male\"` or `\"female\"`.\n\
`nsfw.semen` — gender-specific (update only what changed):\n\
  • **male** → `texture` (TEXT: semen quality — viscosity, warmth, colour, volume feel).\n\
  • **female** → `exterior` (TEXT: external residue on body) + `swallowed` / `vaginal` / \
`anal` as ml (can be large: horse/beast → 2000–8000 ml).\n\
Match FICTION scale, NOT clinical human averages. Use dot-paths, e.g. \
`nsfw.semen.vaginal`: 3200 or `nsfw.semen.texture`: \"浓稠、量大、温热\".\n\
Keep text fields brief. ALWAYS maintain `nsfw` (including `semen`) from the prose.\n\n\
━━━ INCREMENTAL EDIT EXAMPLES ━━━\n\
set: { \"nsfw.arousal\": 60, \"nsfw.semen.vaginal\": 1800, \"nsfw.semen.texture\": \"浓稠拉丝\" }\n\
unset: [\"tags.0\", \"nsfw.semen.anal\"]\n\n\
Do NOT narrate. Your visible text reply must be at most one short sentence."
                    .to_string(),
                schema: json!({
                    "type": "object",
                    "properties": {
                        "action": {
                            "type": "string",
                            "enum": ["get", "create", "update", "delete"],
                            "description": "The operation to perform."
                        },
                        "id": {
                            "type": "string",
                            "description": "Stable role id (required for create/update/delete)."
                        },
                        "role": {
                            "type": "object",
                            "description": "Full role object for `create`."
                        },
                        "set": {
                            "type": "object",
                            "description": "For `update`: map of dot-path → new value, e.g. {\"attributes.好感\": 80}."
                        },
                        "unset": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "For `update`: dot-paths to remove, e.g. [\"tags.0\"]."
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

impl Tool for RoleStateTool {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    fn validate(&self, input: &Value) -> AppResult<()> {
        let action = input
            .get("action")
            .and_then(Value::as_str)
            .ok_or_else(|| AppError::Invalid("RoleState: `action` must be a string".into()))?;
        match action {
            "get" => {}
            "create" => {
                if input.get("id").and_then(Value::as_str).is_none() {
                    return Err(AppError::Invalid(
                        "RoleState create: `id` must be a string".into(),
                    ));
                }
                if !input.get("role").map(Value::is_object).unwrap_or(false) {
                    return Err(AppError::Invalid(
                        "RoleState create: `role` must be an object".into(),
                    ));
                }
            }
            "update" => {
                if input.get("id").and_then(Value::as_str).is_none() {
                    return Err(AppError::Invalid(
                        "RoleState update: `id` must be a string".into(),
                    ));
                }
                let has_set = input.get("set").map(Value::is_object).unwrap_or(false);
                let has_unset = input.get("unset").map(Value::is_array).unwrap_or(false);
                if !has_set && !has_unset {
                    return Err(AppError::Invalid(
                        "RoleState update: provide `set` (object) and/or `unset` (array)".into(),
                    ));
                }
            }
            "delete" => {
                if input.get("id").and_then(Value::as_str).is_none() {
                    return Err(AppError::Invalid(
                        "RoleState delete: `id` must be a string".into(),
                    ));
                }
            }
            other => {
                return Err(AppError::Invalid(format!(
                    "RoleState: unknown action {other:?}; must be get|create|update|delete"
                )));
            }
        }
        Ok(())
    }

    fn execute<'a>(&'a self, invocation: ToolInvocation<'a>) -> ToolFuture<'a> {
        let store = self.store.clone();
        Box::pin(async move {
            let session_id = invocation.context.session_id.clone().ok_or_else(|| {
                AppError::Invalid(
                    "RoleState: no session id on this run; cannot persist role state".into(),
                )
            })?;
            let input = invocation.input;
            let action = input
                .get("action")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();

            match action.as_str() {
                "get" => {
                    let roles = store.snapshot(&session_id);
                    Ok(ToolResult::ok(json!({
                        "op": "get",
                        "roles": roles,
                    })))
                }
                "create" => {
                    let id = input.get("id").and_then(Value::as_str).unwrap_or("").to_string();
                    let role = input.get("role").cloned().unwrap_or(Value::Null);
                    let role = store.create(&session_id, &id, role)?;
                    Ok(ToolResult::ok(json!({
                        "op": "create",
                        "id": id,
                        "role": role,
                    })))
                }
                "update" => {
                    let id = input.get("id").and_then(Value::as_str).unwrap_or("").to_string();
                    let set = input.get("set").and_then(Value::as_object);
                    let unset: Vec<String> = input
                        .get("unset")
                        .and_then(Value::as_array)
                        .map(|a| {
                            a.iter()
                                .filter_map(|v| v.as_str().map(str::to_string))
                                .collect()
                        })
                        .unwrap_or_default();
                    let role = store.update(&session_id, &id, set, &unset)?;
                    Ok(ToolResult::ok(json!({
                        "op": "update",
                        "id": id,
                        "role": role,
                    })))
                }
                "delete" => {
                    let id = input.get("id").and_then(Value::as_str).unwrap_or("").to_string();
                    let removed = store.delete(&session_id, &id)?;
                    Ok(ToolResult::ok(json!({
                        "op": "delete",
                        "id": id,
                        "removed": removed,
                    })))
                }
                other => Err(AppError::Invalid(format!(
                    "RoleState: unknown action {other:?}"
                ))),
            }
        })
    }
}
