//! Interactive RPG option tool.
//!
//! Presents the player with a set of selectable action options at a decision
//! point in an `RPG` agent's narration. Unlike a blocking dice-check prompt,
//! this tool is **non-suspending**: `execute` returns immediately. Its only
//! job is to surface the options through the `gen://tool` `tool_use` event so
//! the frontend can render them as buttons.
//!
//! ━━━ HOW THE LOOP WORKS ━━━
//! 1. The agent narrates up to a decision point, then calls `RpgChoice` once
//!    with 2-5 `options` (each carrying a short `label` and an optional
//!    `text`).
//! 2. The tool returns right away; the model ends its turn (the system prompt
//!    instructs it to stop and wait).
//! 3. The frontend renders the option buttons from the tool's `input`. When
//!    the player clicks one, the option's `text` (or `label`) is inserted
//!    into the chat input box — it is NOT auto-sent, so the player can edit
//!    it before sending it as their next message.
//!
//! Because the player's choice flows back as an ordinary user message, the
//! tool needs no shared store, channel, or `submit_*` command — it is a pure,
//! stateless presenter.

use serde_json::{json, Value};

use crate::ai::agent::tools::{Tool, ToolFuture, ToolInvocation, ToolResult, ToolSpec};
use crate::error::{AppError, AppResult};

pub const TOOL_NAME: &str = "RpgChoice";

/// The `RpgChoice` tool. Stateless: it validates the options and echoes a
/// terse confirmation so the model knows the buttons were shown.
pub struct RpgChoiceTool {
    spec: ToolSpec,
}

impl Default for RpgChoiceTool {
    fn default() -> Self {
        Self::new()
    }
}

impl RpgChoiceTool {
    pub fn new() -> Self {
        Self {
            spec: ToolSpec {
                name: TOOL_NAME.to_string(),
                description: "\
Present the player with a set of selectable action options at a decision \
point in the story. Call this ONCE, right after narrating up to the choice \
point.\n\n\
━━━ HOW IT WORKS ━━━\n\
- Each option has a short `label` (the action text shown on the button) and \
  an optional `text` (the full first-person sentence that gets inserted into \
  the player's input box when they click it; defaults to `label`).\n\
- This tool does NOT pause the story and does NOT decide the outcome — it \
  only displays the buttons. When the player clicks one, its `text` is placed \
  in their input box so they can edit and send it as their next message.\n\
- After calling `RpgChoice`, STOP: end your turn immediately. Do NOT narrate \
  the consequences of any option yourself, and do NOT call the tool again \
  until the player has replied and the story reaches the next decision \
  point.\n\n\
━━━ DESIGNING OPTIONS ━━━\n\
- Provide 2-5 distinct, concrete options that meaningfully branch the story.\n\
- Keep `label` short (a few words). Write `text` as the in-fiction action the \
  player would take, in their voice (e.g. \"我推开那扇门，走进黑暗的房间。\")."
                    .to_string(),
                schema: json!({
                    "type": "object",
                    "properties": {
                        "prompt": {
                            "type": "string",
                            "description": "Optional situation/question shown above the option buttons."
                        },
                        "options": {
                            "type": "array",
                            "minItems": 2,
                            "description": "The selectable branching actions.",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "id": {
                                        "type": "string",
                                        "description": "Optional stable id for the option."
                                    },
                                    "label": {
                                        "type": "string",
                                        "description": "Short action text shown on the button."
                                    },
                                    "text": {
                                        "type": "string",
                                        "description": "Full sentence inserted into the player's input box on click. Defaults to `label`."
                                    }
                                },
                                "required": ["label"]
                            }
                        }
                    },
                    "required": ["options"]
                }),
                read_only: true,
                concurrency_safe: false,
            },
        }
    }
}

impl Tool for RpgChoiceTool {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    fn validate(&self, input: &Value) -> AppResult<()> {
        let options = input
            .get("options")
            .and_then(Value::as_array)
            .ok_or_else(|| AppError::Invalid("RpgChoice: `options` must be an array".into()))?;
        if options.len() < 2 {
            return Err(AppError::Invalid(
                "RpgChoice: provide at least 2 options".into(),
            ));
        }
        for (i, opt) in options.iter().enumerate() {
            if opt.get("label").and_then(Value::as_str).is_none() {
                return Err(AppError::Invalid(format!(
                    "RpgChoice: option {i} is missing a string `label`"
                )));
            }
        }
        Ok(())
    }

    fn execute<'a>(&'a self, invocation: ToolInvocation<'a>) -> ToolFuture<'a> {
        Box::pin(async move {
            let count = invocation
                .input
                .get("options")
                .and_then(Value::as_array)
                .map(|a| a.len())
                .unwrap_or(0);
            Ok(ToolResult::ok(json!({
                "presented": true,
                "options_count": count,
                "note": "Options shown to the player. End your turn now and wait for their reply."
            })))
        })
    }
}
