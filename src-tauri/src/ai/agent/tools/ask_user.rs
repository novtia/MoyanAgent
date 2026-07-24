//! Ask the user one or more clarifying questions (blocking / human-in-the-loop).
//!
//! This tool **suspends** the agent loop: `execute` registers a oneshot on
//! [`PromptRegistry`] and awaits the user's answer. The frontend shows
//! questions in the composer; submitting calls `answer_ask_user`, which wakes
//! this tool. The answer becomes `tool_result` and the same generation continues.
//!
//! Cancellation: `invocation.context.abort` ends the wait with an error result.

use std::sync::Arc;

use serde_json::{json, Value};

use crate::ai::agent::tools::prompt_registry::{PromptAnswer, PromptRegistry};
use crate::ai::agent::tools::{Tool, ToolFuture, ToolInvocation, ToolResult, ToolSpec};
use crate::error::{AppError, AppResult};

pub const TOOL_NAME: &str = "AskUser";

const MAX_QUESTIONS: usize = 5;

/// Blocking AskUser tool. Holds a shared [`PromptRegistry`] with the Tauri command.
pub struct AskUserTool {
    spec: ToolSpec,
    registry: Arc<PromptRegistry>,
}

impl AskUserTool {
    pub fn new(registry: Arc<PromptRegistry>) -> Self {
        Self {
            registry,
            spec: ToolSpec {
                name: TOOL_NAME.to_string(),
                description: "\
Ask the user one or more clarifying questions and **wait** for their answer \
before continuing. Call this ONCE when you need a decision or missing \
information that only the user can provide.\n\n\
━━━ HOW IT WORKS ━━━\n\
- Provide 1–5 `questions`. Each has a `prompt` (shown above the input) and \
  at least one `option` (short `label`; optional `text` fills the input box \
  on click, defaulting to `label`).\n\
- This tool **pauses the agent loop** until the user answers in the composer. \
  Their reply is returned as this tool's result (`answer`); continue from \
  that answer. Do NOT invent the user's choice yourself.\n\
- Put the full question and all options in the tool input only — do NOT list \
  A/B/C choices as plain chat text.\n\n\
━━━ DESIGNING QUESTIONS ━━━\n\
- Prefer concrete, mutually distinct options. Keep `label` short.\n\
- Use multiple questions only when they are independent clarifications needed \
  in one round (e.g. style + scope). Cap at 5.".to_string(),
                schema: json!({
                    "type": "object",
                    "properties": {
                        "questions": {
                            "type": "array",
                            "minItems": 1,
                            "maxItems": 5,
                            "description": "One or more clarifying questions shown in the composer.",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "id": {
                                        "type": "string",
                                        "description": "Optional stable id for the question."
                                    },
                                    "prompt": {
                                        "type": "string",
                                        "description": "Question text shown above the input box."
                                    },
                                    "options": {
                                        "type": "array",
                                        "minItems": 1,
                                        "description": "Quick-fill choices for this question.",
                                        "items": {
                                            "type": "object",
                                            "properties": {
                                                "id": {
                                                    "type": "string",
                                                    "description": "Optional stable id for the option."
                                                },
                                                "label": {
                                                    "type": "string",
                                                    "description": "Short text shown next to ○/●."
                                                },
                                                "text": {
                                                    "type": "string",
                                                    "description": "Full reply inserted into the input box on click. Defaults to `label`."
                                                }
                                            },
                                            "required": ["label"]
                                        }
                                    }
                                },
                                "required": ["prompt", "options"]
                            }
                        }
                    },
                    "required": ["questions"]
                }),
                // Blocks the agent until the user answers — never concurrent / read-only.
                read_only: false,
                concurrency_safe: false,
            },
        }
    }
}

impl Tool for AskUserTool {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    fn validate(&self, input: &Value) -> AppResult<()> {
        let questions = input
            .get("questions")
            .and_then(Value::as_array)
            .ok_or_else(|| AppError::Invalid("AskUser: `questions` must be an array".into()))?;
        if questions.is_empty() {
            return Err(AppError::Invalid(
                "AskUser: provide at least 1 question".into(),
            ));
        }
        if questions.len() > MAX_QUESTIONS {
            return Err(AppError::Invalid(format!(
                "AskUser: at most {MAX_QUESTIONS} questions"
            )));
        }
        for (qi, q) in questions.iter().enumerate() {
            let prompt = q.get("prompt").and_then(Value::as_str).unwrap_or("").trim();
            if prompt.is_empty() {
                return Err(AppError::Invalid(format!(
                    "AskUser: question {qi} is missing a non-empty `prompt`"
                )));
            }
            let options = q
                .get("options")
                .and_then(Value::as_array)
                .ok_or_else(|| {
                    AppError::Invalid(format!(
                        "AskUser: question {qi} `options` must be an array"
                    ))
                })?;
            if options.is_empty() {
                return Err(AppError::Invalid(format!(
                    "AskUser: question {qi} needs at least 1 option"
                )));
            }
            for (oi, opt) in options.iter().enumerate() {
                if opt.get("label").and_then(Value::as_str).is_none() {
                    return Err(AppError::Invalid(format!(
                        "AskUser: question {qi} option {oi} is missing a string `label`"
                    )));
                }
            }
        }
        Ok(())
    }

    fn execute<'a>(&'a self, invocation: ToolInvocation<'a>) -> ToolFuture<'a> {
        Box::pin(async move {
            let id = invocation.id.as_str().to_string();
            let (rx, _guard) = self.registry.register(id);

            let answer = tokio::select! {
                received = rx => received.ok(),
                _ = invocation.context.abort.wait_aborted() => None,
            };

            let PromptAnswer { answer, items } = match answer {
                Some(a) => a,
                None => {
                    return Ok(ToolResult::error("用户未回答（提问已取消）"));
                }
            };

            let answer = answer.trim().to_string();
            if answer.is_empty() {
                return Ok(ToolResult::error("用户未回答（空答复）"));
            }

            Ok(ToolResult::ok(json!({
                "answered": true,
                "answer": answer,
                "items": items,
            })))
        })
    }
}
