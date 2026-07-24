//! Human-in-the-loop wait registry for AskUser.
//!
//! `AskUser::execute` registers a oneshot and awaits it, pausing the agent
//! loop. The frontend submits via `answer_ask_user`, which calls
//! [`PromptRegistry::answer`] and wakes the tool so the loop continues.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use tokio::sync::oneshot;

/// One answered question (for tool_result + history UI).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PromptAnswerItem {
    #[serde(default)]
    pub prompt: String,
    #[serde(default)]
    pub answer: String,
}

/// User's reply for one AskUser invocation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PromptAnswer {
    /// Aggregated text fed back to the model.
    #[serde(default)]
    pub answer: String,
    /// Structured per-question replies for the history card.
    #[serde(default)]
    pub items: Vec<PromptAnswerItem>,
}

/// In-flight prompts: tool_call id → oneshot sender.
#[derive(Default)]
pub struct PromptRegistry {
    pending: Mutex<HashMap<String, oneshot::Sender<PromptAnswer>>>,
}

/// RAII: remove the pending entry when the tool future completes or is dropped.
pub struct PromptGuard {
    registry: Arc<PromptRegistry>,
    id: String,
}

impl Drop for PromptGuard {
    fn drop(&mut self) {
        if let Ok(mut pending) = self.registry.pending.lock() {
            pending.remove(&self.id);
        }
    }
}

impl PromptRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a wait. Duplicate ids replace the previous sender.
    pub fn register(
        self: &Arc<Self>,
        id: impl Into<String>,
    ) -> (oneshot::Receiver<PromptAnswer>, PromptGuard) {
        let id = id.into();
        let (tx, rx) = oneshot::channel();
        if let Ok(mut pending) = self.pending.lock() {
            pending.insert(id.clone(), tx);
        }
        (
            rx,
            PromptGuard {
                registry: Arc::clone(self),
                id,
            },
        )
    }

    /// Deliver the answer. Returns true if a waiter was woken.
    pub fn answer(&self, id: &str, answer: PromptAnswer) -> bool {
        let tx = self
            .pending
            .lock()
            .ok()
            .and_then(|mut pending| pending.remove(id));
        match tx {
            Some(tx) => tx.send(answer).is_ok(),
            None => false,
        }
    }
}
