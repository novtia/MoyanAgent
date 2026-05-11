//! Task lifecycle for local / remote / teammate agents.
//!
//! Maps `Task.ts` + `tasks/*Task` on the TS side.
//!
//! ID prefixes (matching `agent-architecture.md` §3):
//!
//! | TaskKind             | Prefix |
//! | -------------------- | ------ |
//! | `LocalAgent`         | `a`    |
//! | `RemoteAgent`        | `r`    |
//! | `InProcessTeammate`  | `t`    |

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

use crate::ai::agent::types::{AgentId, TokenUsage};

/// Stable task identifier (`a01H...`, `r01H...`, `t01H...`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TaskId(pub String);

impl TaskId {
    pub fn new(kind: TaskKind) -> Self {
        let prefix = match kind {
            TaskKind::LocalAgent => "a",
            TaskKind::RemoteAgent => "r",
            TaskKind::InProcessTeammate => "t",
        };
        Self(format!("{}{}", prefix, ulid::Ulid::new()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for TaskId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskKind {
    LocalAgent,
    RemoteAgent,
    InProcessTeammate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskState {
    Pending,
    Running,
    Completed,
    Failed,
    Killed,
}

/// One row in the task registry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: TaskId,
    pub kind: TaskKind,
    pub state: TaskState,
    pub agent_id: AgentId,
    /// Original user-facing prompt that spawned the task.
    pub prompt: String,
    pub agent_type: String,
    pub model: Option<String>,
    pub started_at_ms: i64,
    pub ended_at_ms: Option<i64>,
    /// Final assistant text (when completed).
    pub result: Option<String>,
    pub error: Option<String>,
    /// Transcript sidechain file, when persisted.
    pub transcript_path: Option<PathBuf>,
    /// Cumulative usage across the agent's child query loop.
    pub usage: TokenUsage,
    /// True ⇒ runs in the background and emits TaskNotification on finish.
    pub background: bool,
    /// True ⇒ already reported a notification (avoid duplicates).
    pub notified: bool,
}

impl Task {
    pub fn new_local(agent_id: AgentId, agent_type: impl Into<String>, prompt: String) -> Self {
        let now = chrono_now_ms();
        Self {
            id: TaskId::new(TaskKind::LocalAgent),
            kind: TaskKind::LocalAgent,
            state: TaskState::Pending,
            agent_id,
            prompt,
            agent_type: agent_type.into(),
            model: None,
            started_at_ms: now,
            ended_at_ms: None,
            result: None,
            error: None,
            transcript_path: None,
            usage: TokenUsage::default(),
            background: false,
            notified: false,
        }
    }
}

fn chrono_now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// In-memory task registry. The Tauri layer is expected to wrap this in
/// `Arc<TaskStore>` and stash it in `AppState`.
#[derive(Default)]
pub struct TaskStore {
    inner: Mutex<HashMap<TaskId, Arc<Mutex<Task>>>>,
}

impl TaskStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&self, task: Task) -> TaskId {
        let id = task.id.clone();
        if let Ok(mut g) = self.inner.lock() {
            g.insert(id.clone(), Arc::new(Mutex::new(task)));
        }
        id
    }

    pub fn get(&self, id: &TaskId) -> Option<Arc<Mutex<Task>>> {
        self.inner.lock().ok()?.get(id).cloned()
    }

    pub fn list(&self) -> Vec<Task> {
        let Ok(g) = self.inner.lock() else {
            return vec![];
        };
        g.values()
            .filter_map(|t| t.lock().ok().map(|t| t.clone()))
            .collect()
    }

    pub fn set_state(&self, id: &TaskId, state: TaskState) {
        if let Some(slot) = self.get(id) {
            if let Ok(mut t) = slot.lock() {
                t.state = state;
                if matches!(
                    state,
                    TaskState::Completed | TaskState::Failed | TaskState::Killed
                ) {
                    t.ended_at_ms = Some(chrono_now_ms());
                }
            }
        }
    }

    pub fn complete(&self, id: &TaskId, result: Option<String>, usage: TokenUsage) {
        if let Some(slot) = self.get(id) {
            if let Ok(mut t) = slot.lock() {
                t.state = TaskState::Completed;
                t.result = result;
                t.usage = usage;
                t.ended_at_ms = Some(chrono_now_ms());
            }
        }
    }

    pub fn fail(&self, id: &TaskId, error: impl Into<String>) {
        if let Some(slot) = self.get(id) {
            if let Ok(mut t) = slot.lock() {
                t.state = TaskState::Failed;
                t.error = Some(error.into());
                t.ended_at_ms = Some(chrono_now_ms());
            }
        }
    }

    pub fn kill(&self, id: &TaskId) {
        self.set_state(id, TaskState::Killed);
    }

    pub fn remove(&self, id: &TaskId) -> Option<Task> {
        let Ok(mut g) = self.inner.lock() else {
            return None;
        };
        let slot = g.remove(id)?;
        let task = slot.lock().ok().map(|t| t.clone());
        task
    }
}
