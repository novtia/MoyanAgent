//! Attachments — hidden user-meta messages injected at turn boundaries.
//!
//! Mirrors `utils/attachments.ts` and the queued-command / task-notification
//! injection described in `agent-architecture.md` §12.
//!
//! Attachments are *not* normal assistant/user turns; they are appended as
//! hidden user-role messages right before the next API request.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::ai::agent::core::task::{Task, TaskId, TaskState};
use crate::ai::agent::types::AgentId;

/// Concrete attachment payloads. The renderer in [`render`] converts these
/// into the system-reminder text used by the model.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AttachmentKind {
    /// Memory file pulled in because a Read tool touched its scope.
    NestedMemory { path: PathBuf, content: String },
    /// Relevant-memory prefetch result.
    RelevantMemories { entries: Vec<RelevantMemoryEntry> },
    /// Notification produced when a background task completes.
    TaskNotification(TaskNotification),
    /// Skill body reinstated after compact.
    InvokedSkill { name: String, body: String },
    /// Date / agent listing / MCP delta etc.
    Delta { topic: String, body: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelevantMemoryEntry {
    pub path: PathBuf,
    pub description: Option<String>,
    pub age_hint: Option<String>,
    pub body: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Attachment {
    pub kind: AttachmentKind,
    /// `Some(agent_id)` ⇒ only the named agent should drain this attachment.
    /// `None` ⇒ visible to the main loop.
    pub target: Option<AgentId>,
}

impl Attachment {
    pub fn for_main(kind: AttachmentKind) -> Self {
        Self { kind, target: None }
    }
    pub fn for_agent(agent_id: AgentId, kind: AttachmentKind) -> Self {
        Self {
            kind,
            target: Some(agent_id),
        }
    }
}

/// The XML-shaped `<task-notification>` payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskNotification {
    pub task_id: TaskId,
    pub status: TaskNotificationStatus,
    pub summary: String,
    pub result: Option<String>,
    pub usage: Option<crate::ai::agent::types::TokenUsage>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskNotificationStatus {
    Completed,
    Failed,
    Killed,
    Updated,
}

impl TaskNotification {
    /// Build a notification from a finished [`Task`]. Returns `None` if
    /// the task is still active (`Pending` / `Running`).
    pub fn from_task(task: &Task) -> Option<Self> {
        let status = match task.state {
            TaskState::Completed => TaskNotificationStatus::Completed,
            TaskState::Failed => TaskNotificationStatus::Failed,
            TaskState::Killed => TaskNotificationStatus::Killed,
            TaskState::Pending | TaskState::Running => return None,
        };
        let summary = format!(
            "Agent {agent_type} ({agent_id}) {state}",
            agent_type = task.agent_type,
            agent_id = task.agent_id,
            state = match status {
                TaskNotificationStatus::Completed => "completed",
                TaskNotificationStatus::Failed => "failed",
                TaskNotificationStatus::Killed => "was killed",
                TaskNotificationStatus::Updated => "updated",
            },
        );
        let result = task
            .result
            .clone()
            .or_else(|| task.error.clone());
        Some(Self {
            task_id: task.id.clone(),
            status,
            summary,
            result,
            usage: Some(task.usage.clone()),
        })
    }
}

/// Render an attachment into the hidden user-meta-message body.
pub fn render(attachment: &Attachment) -> String {
    match &attachment.kind {
        AttachmentKind::NestedMemory { path, content } => {
            format!(
                "<system-reminder>\nContents of {}:\n\n{}\n</system-reminder>",
                path.display(),
                content
            )
        }
        AttachmentKind::RelevantMemories { entries } => {
            let mut out = String::from("<system-reminder>\nRelevant memories:\n\n");
            for e in entries {
                out.push_str(&format!(
                    "## {}\n{}{}\n\n",
                    e.path.display(),
                    e.age_hint
                        .as_deref()
                        .map(|h| format!("[{h}] "))
                        .unwrap_or_default(),
                    e.body
                ));
            }
            out.push_str("</system-reminder>");
            out
        }
        AttachmentKind::TaskNotification(n) => render_notification(n),
        AttachmentKind::InvokedSkill { name, body } => {
            format!(
                "<system-reminder>\nContinue following the {} skill:\n\n{}\n</system-reminder>",
                name, body
            )
        }
        AttachmentKind::Delta { topic, body } => {
            format!(
                "<system-reminder>\n[{}]\n{}\n</system-reminder>",
                topic, body
            )
        }
    }
}

fn render_notification(n: &TaskNotification) -> String {
    let status = match n.status {
        TaskNotificationStatus::Completed => "completed",
        TaskNotificationStatus::Failed => "failed",
        TaskNotificationStatus::Killed => "killed",
        TaskNotificationStatus::Updated => "updated",
    };
    let usage = n
        .usage
        .as_ref()
        .map(|u| {
            format!(
                "<usage><total_tokens>{}</total_tokens></usage>",
                u.total_tokens.unwrap_or(0)
            )
        })
        .unwrap_or_default();
    format!(
        "<task-notification>\n<task-id>{}</task-id>\n<status>{}</status>\n<summary>{}</summary>\n<result>{}</result>\n{}\n</task-notification>",
        n.task_id,
        status,
        n.summary,
        n.result.as_deref().unwrap_or(""),
        usage
    )
}

/// FIFO queue of attachments drained at turn boundaries.
///
/// The main loop drains attachments with `target == None`; each sub-agent
/// drains attachments addressed to its own `AgentId`. This matches the
/// `pendingMessages` / `task-notification` routing described in
/// `agent-architecture.md` §13.
#[derive(Default)]
pub struct NotificationQueue {
    inner: Mutex<VecDeque<Attachment>>,
}

impl NotificationQueue {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&self, attachment: Attachment) {
        if let Ok(mut q) = self.inner.lock() {
            q.push_back(attachment);
        }
    }

    pub fn drain_for_main(&self) -> Vec<Attachment> {
        self.drain_where(|a| a.target.is_none())
    }

    pub fn drain_for_agent(&self, agent_id: &AgentId) -> Vec<Attachment> {
        self.drain_where(|a| a.target.as_ref() == Some(agent_id))
    }

    fn drain_where<F>(&self, predicate: F) -> Vec<Attachment>
    where
        F: Fn(&Attachment) -> bool,
    {
        let Ok(mut q) = self.inner.lock() else {
            return vec![];
        };
        let mut keep = VecDeque::with_capacity(q.len());
        let mut taken = Vec::new();
        while let Some(item) = q.pop_front() {
            if predicate(&item) {
                taken.push(item);
            } else {
                keep.push_back(item);
            }
        }
        *q = keep;
        taken
    }
}
