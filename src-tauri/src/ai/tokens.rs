use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TokenUsage {
    pub prompt_tokens: Option<i64>,
    pub completion_tokens: Option<i64>,
    pub total_tokens: Option<i64>,
    /// Prompt tokens of the *latest* API call within an agent loop.
    ///
    /// Unlike `prompt_tokens` — which `accumulate_usage` sums across every
    /// tool-call round of a turn — this is *replaced* on each round, so it
    /// reflects the real context-window occupancy at the end of the turn (the
    /// last request already carries the full conversation history). The composer
    /// context ring uses this instead of the summed total to avoid over-counting
    /// when a turn fans out into multiple tool-call rounds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_prompt_tokens: Option<i64>,
}

impl TokenUsage {
    pub fn is_empty(&self) -> bool {
        self.prompt_tokens.is_none()
            && self.completion_tokens.is_none()
            && self.total_tokens.is_none()
    }
}

pub fn extract_usage(v: &Value) -> TokenUsage {
    let usage = v.get("usage").unwrap_or(&Value::Null);
    TokenUsage {
        prompt_tokens: usage.get("prompt_tokens").and_then(Value::as_i64),
        completion_tokens: usage.get("completion_tokens").and_then(Value::as_i64),
        total_tokens: usage.get("total_tokens").and_then(Value::as_i64),
        last_prompt_tokens: usage.get("last_prompt_tokens").and_then(Value::as_i64),
    }
}
