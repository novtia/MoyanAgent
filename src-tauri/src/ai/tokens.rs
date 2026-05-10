use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TokenUsage {
    pub prompt_tokens: Option<i64>,
    pub completion_tokens: Option<i64>,
    pub total_tokens: Option<i64>,
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
    }
}
