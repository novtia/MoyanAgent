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

/// Separator for the composite key. A unit separator cannot appear in a
/// session id or a provider-issued tool_call id.
const KEY_SEP: char = '\u{1f}';

/// In-flight prompts: `session_id + tool_call id` → oneshot sender.
///
/// The session is part of the key because tool_call ids are only unique within
/// one provider response: two conversations waiting at the same time can both
/// hold a prompt called `call_1`, and answering one would otherwise wake
/// whichever of the two happened to be in the map.
#[derive(Default)]
pub struct PromptRegistry {
    pending: Mutex<HashMap<String, oneshot::Sender<PromptAnswer>>>,
}

/// RAII: remove the pending entry when the tool future completes or is dropped.
pub struct PromptGuard {
    registry: Arc<PromptRegistry>,
    key: String,
}

impl Drop for PromptGuard {
    fn drop(&mut self) {
        if let Ok(mut pending) = self.registry.pending.lock() {
            pending.remove(&self.key);
        }
    }
}

fn prompt_key(session_id: Option<&str>, id: &str) -> String {
    format!("{}{KEY_SEP}{id}", session_id.unwrap_or_default())
}

impl PromptRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a wait. Duplicate keys replace the previous sender.
    pub fn register(
        self: &Arc<Self>,
        session_id: Option<&str>,
        id: &str,
    ) -> (oneshot::Receiver<PromptAnswer>, PromptGuard) {
        let key = prompt_key(session_id, id);
        let (tx, rx) = oneshot::channel();
        if let Ok(mut pending) = self.pending.lock() {
            pending.insert(key.clone(), tx);
        }
        (
            rx,
            PromptGuard {
                registry: Arc::clone(self),
                key,
            },
        )
    }

    /// Deliver the answer. Returns true if a waiter was woken.
    ///
    /// Falls back to the tool_call id alone when the session does not match a
    /// waiter — a sub-agent registers under its own temp session id, which the
    /// answering UI does not necessarily know. The fallback only fires when
    /// exactly one session is waiting on that id, so an ambiguous answer is
    /// dropped rather than delivered to the wrong conversation.
    pub fn answer(&self, session_id: Option<&str>, id: &str, answer: PromptAnswer) -> bool {
        let Ok(mut pending) = self.pending.lock() else {
            return false;
        };
        let key = match pending.contains_key(&prompt_key(session_id, id)) {
            true => prompt_key(session_id, id),
            false => {
                let suffix = format!("{KEY_SEP}{id}");
                let mut matches = pending.keys().filter(|k| k.ends_with(&suffix));
                match (matches.next().cloned(), matches.next()) {
                    (Some(only), None) => only,
                    _ => return false,
                }
            }
        };
        match pending.remove(&key) {
            Some(tx) => tx.send(answer).is_ok(),
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn answer(text: &str) -> PromptAnswer {
        PromptAnswer {
            answer: text.into(),
            items: Vec::new(),
        }
    }

    #[tokio::test]
    async fn an_answer_reaches_the_waiter_in_its_own_session() {
        let reg = Arc::new(PromptRegistry::new());
        let (mut a, _ga) = reg.register(Some("session-a"), "call_1");
        let (b, _gb) = reg.register(Some("session-b"), "call_1");

        assert!(reg.answer(Some("session-b"), "call_1", answer("for b")));

        assert_eq!(b.await.unwrap().answer, "for b");
        // A shared tool_call id must not hand session A's prompt B's answer.
        assert!(a.try_recv().is_err());
    }

    /// The sub-agent case: the tool waits under a temp session id the UI never
    /// sees, so an id-only answer still has to land.
    #[tokio::test]
    async fn an_unmatched_session_falls_back_to_a_unique_id() {
        let reg = Arc::new(PromptRegistry::new());
        let (rx, _g) = reg.register(Some("temp-child"), "call_9");

        assert!(reg.answer(Some("parent"), "call_9", answer("late")));
        assert_eq!(rx.await.unwrap().answer, "late");
    }

    #[tokio::test]
    async fn an_ambiguous_id_is_refused_rather_than_guessed() {
        let reg = Arc::new(PromptRegistry::new());
        let (mut a, _ga) = reg.register(Some("session-a"), "call_1");
        let (mut b, _gb) = reg.register(Some("session-b"), "call_1");

        assert!(!reg.answer(Some("session-c"), "call_1", answer("nobody")));
        assert!(a.try_recv().is_err());
        assert!(b.try_recv().is_err());
    }

    #[tokio::test]
    async fn a_dropped_guard_removes_the_waiter() {
        let reg = Arc::new(PromptRegistry::new());
        {
            let (_rx, _guard) = reg.register(Some("s"), "call_1");
        }
        assert!(!reg.answer(Some("s"), "call_1", answer("gone")));
    }
}
