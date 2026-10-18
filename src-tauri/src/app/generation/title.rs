//! Automatic session titles.
//!
//! The quick model does not answer with free text that we then try to clean up.
//! It is given one internal tool, [`TITLE_TOOL`], and names the session by
//! calling it. [`run_title_tool`] is that tool's implementation: it validates
//! the arguments (no file paths, no empty strings) and returns the title the
//! caller persists. The tool is private to this module — it is never part of
//! the agent's `ToolPool` and the main agent cannot see it.

use std::sync::Arc;

use serde_json::{json, Value};
use tauri::{AppHandle, Emitter};

use crate::ai::{chat, parameters};
use crate::app::state::AppState;
use crate::data::{db, session, settings};
use crate::error::AppResult;

/// Name of the internal tool the quick model calls to set the title.
pub(crate) const TITLE_TOOL: &str = "SetSessionTitle";

const MAX_TITLE_CHARS: usize = 40;
const FALLBACK_TITLE_CHARS: usize = 28;
const PLACEHOLDER_TITLE: &str = "New session";

/// Tool definition sent to the quick model.
pub(crate) fn title_tool() -> chat::ToolDefinition {
    chat::ToolDefinition {
        name: TITLE_TOOL.to_string(),
        description: "Set the title of the current conversation. Call this exactly once, \
            as your only action; do not reply with text. The title is a short topic \
            phrase (at most 12 Chinese characters or 6 English words). It must never \
            contain a file path, drive letter, backslash, file extension, `@` mention, \
            URL or `#P003`-style marker; refer to a document by its name only."
            .to_string(),
        schema: json!({
            "type": "object",
            "properties": {
                "title": {
                    "type": "string",
                    "description": "The title text only: no quotes, trailing punctuation, \
                        numbering or prefix such as \"Title:\"."
                }
            },
            "required": ["title"]
        }),
    }
}

/// Execute the title tool: validate the model's arguments and return the
/// cleaned title, or a message saying what was wrong with them.
pub(crate) fn run_title_tool(input: &Value) -> Result<String, String> {
    let raw = input
        .get("title")
        .and_then(Value::as_str)
        .ok_or_else(|| "`title` must be a string".to_string())?;
    let title = clean_title(raw);
    if title.is_empty() {
        return Err("`title` is empty".into());
    }
    if looks_like_path(&title) {
        return Err("`title` must not be a file path or an `@` mention".into());
    }
    Ok(title)
}

/// First line only, surrounding quotes/punctuation removed, length capped.
fn clean_title(raw: &str) -> String {
    let first_line = raw.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
    first_line
        .trim()
        .trim_matches(|c: char| {
            c.is_whitespace()
                || matches!(
                    c,
                    '"' | '\'' | '`' | '「' | '」' | '『' | '』' | '《' | '》' | '。' | '.' | '：' | ':'
                )
        })
        .chars()
        .take(MAX_TITLE_CHARS)
        .collect()
}

/// True for text that is a file reference rather than a title.
fn looks_like_path(s: &str) -> bool {
    let t = s.trim();
    let drive = t.chars().nth(1) == Some(':') && t.chars().next().is_some_and(|c| c.is_ascii_alphabetic());
    t.starts_with('@') || t.contains('\\') || t.contains("://") || t.contains(":/") || drive
}

/// System prompt for title generation: the user's setting, or the built-in.
pub(crate) fn title_system_prompt(s: &settings::Settings) -> String {
    let custom = s.quick_model_prompt.trim();
    if custom.is_empty() {
        settings::DEFAULT_TITLE_SYSTEM_PROMPT.to_string()
    } else {
        custom.to_string()
    }
}

/// The user's message reduced to what a title should be built from.
///
/// Serialized file mentions (`@"C:\\Users\\…\\大纲.md"#P003-P007`, or a bare
/// `@C:\…\a.png`) are replaced by the document's name without its extension,
/// so "what the message is about" survives and the path does not. Whitespace
/// is collapsed to single spaces.
pub(crate) fn title_source_text(prompt: &str) -> String {
    let chars: Vec<char> = prompt.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] != '@' {
            out.push(chars[i]);
            i += 1;
            continue;
        }
        // Quoted form: @"<json-escaped path>" with an optional #P003-P007 tail.
        if chars.get(i + 1) == Some(&'"') {
            let mut j = i + 2;
            let mut payload = String::new();
            while j < chars.len() && chars[j] != '"' {
                if chars[j] == '\\' && j + 1 < chars.len() {
                    // `\\` is one backslash; any other escape keeps the
                    // escaped character itself.
                    payload.push(chars[j + 1]);
                    j += 2;
                } else {
                    payload.push(chars[j]);
                    j += 1;
                }
            }
            if j < chars.len() {
                j += 1; // closing quote
                if chars.get(j) == Some(&'#') && chars.get(j + 1) == Some(&'P') {
                    j += 2;
                    while j < chars.len() && chars[j].is_ascii_digit() {
                        j += 1;
                    }
                    if chars.get(j) == Some(&'-') && chars.get(j + 1) == Some(&'P') {
                        j += 2;
                        while j < chars.len() && chars[j].is_ascii_digit() {
                            j += 1;
                        }
                    }
                }
                out.push_str(&mention_title_name(&payload));
                i = j;
                continue;
            }
        }
        // Bare form: @C:\dir\file.png or @/abs/path — runs to whitespace.
        let mut j = i + 1;
        while j < chars.len() && !chars[j].is_whitespace() {
            j += 1;
        }
        let token: String = chars[i + 1..j].iter().collect();
        let drive = token.chars().nth(1) == Some(':');
        if drive || token.contains('\\') || token.contains('/') {
            out.push_str(&mention_title_name(&token));
            i = j;
            continue;
        }
        out.push('@');
        i += 1;
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// File name without directories or extension: `C:\a\大纲.md` → `大纲`.
fn mention_title_name(path: &str) -> String {
    let name = path.rsplit(['\\', '/']).next().unwrap_or(path).trim();
    match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() && ext.len() <= 5 => stem.to_string(),
        _ => name.to_string(),
    }
}

/// True when the session still carries the placeholder title, i.e. it has not
/// been renamed (manually or automatically) yet.
pub(crate) fn session_title_is_default(conn: &db::DbConn, id: &str) -> AppResult<bool> {
    let cur: Option<String> = conn
        .query_row(
            "SELECT title FROM sessions WHERE id=?1",
            rusqlite::params![id],
            |r| r.get(0),
        )
        .ok();
    Ok(match cur {
        Some(t) => t == PLACEHOLDER_TITLE || t.trim().is_empty(),
        None => false,
    })
}

/// Title taken from the start of the (already cleaned) message. Used when no
/// quick model is configured, or when the model did not set a usable title.
pub(crate) fn fallback_title(source: &str) -> String {
    let title = clean_title(&source.chars().take(FALLBACK_TITLE_CHARS).collect::<String>());
    if looks_like_path(&title) {
        String::new()
    } else {
        title
    }
}

/// Rename the session to the fallback title if it still has the placeholder.
pub(crate) fn apply_fallback_title(conn: &db::DbConn, id: &str, source: &str) -> AppResult<()> {
    let title = fallback_title(source);
    if title.is_empty() || !session_title_is_default(conn, id)? {
        return Ok(());
    }
    conn.execute(
        "UPDATE sessions SET title=?1 WHERE id=?2",
        rusqlite::params![title, id],
    )?;
    Ok(())
}

/// Ask the quick model to name the session by calling [`TITLE_TOOL`], then
/// persist the result. Best-effort and run off the request path: when the model
/// fails or never calls the tool, the session is named from the start of the
/// message instead.
pub(crate) async fn generate_title_with_quick_model(
    app: AppHandle,
    state: Arc<AppState>,
    session_id: String,
    source: String,
    provider: chat::ProviderConfig,
    model: String,
    system_prompt: String,
) {
    let request = chat::ChatRequest {
        provider,
        model,
        prompt: source.clone(),
        attachments: Vec::new(),
        system_prompt,
        history: Vec::new(),
        parameters: parameters::factory().build(
            "auto".into(),
            "auto".into(),
            settings::ModelParamSettings::default(),
        ),
        tools: vec![title_tool()],
        tool_chain: Vec::new(),
        tool_results: Vec::new(),
        pending_assistant_turn: None,
        previous_response_id: None,
        context_cache_enabled: false,
        context_window: None,
        todo_snapshot: None,
        route_providers: Vec::new(),
        forced_tools: vec![TITLE_TOOL.to_string()],
    };

    let factory = crate::ai::providers::ProviderFactory::default();
    let from_model = match factory.chat(request).await {
        Ok(resp) => resp
            .tool_calls
            .iter()
            .find(|c| c.name == TITLE_TOOL)
            .map(|call| run_title_tool(&call.arguments))
            .unwrap_or_else(|| Err("the model did not call the title tool".into())),
        Err(err) => Err(err.to_string()),
    };
    let title = match from_model {
        Ok(title) => title,
        Err(reason) => {
            eprintln!("[atelier] quick-model title not set: {reason}");
            fallback_title(&source)
        }
    };
    if title.is_empty() {
        return;
    }

    let Ok(conn) = state.conn() else {
        return;
    };
    // The user may have renamed the session while we waited.
    if !session_title_is_default(&conn, &session_id).unwrap_or(false) {
        return;
    }
    if session::rename(&conn, &session_id, &title).is_ok() {
        let _ = app.emit(
            "session://title",
            json!({ "session_id": session_id, "title": title }),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_schema_requires_a_title_string() {
        let tool = title_tool();
        assert_eq!(tool.name, TITLE_TOOL);
        assert_eq!(tool.schema["required"], json!(["title"]));
        assert_eq!(tool.schema["properties"]["title"]["type"], "string");
    }

    #[test]
    fn tool_accepts_a_plain_title_and_tidies_it() {
        assert_eq!(
            run_title_tool(&json!({ "title": "后续三章大纲设计" })).unwrap(),
            "后续三章大纲设计"
        );
        assert_eq!(
            run_title_tool(&json!({ "title": "「润色开头」\n多余的第二行" })).unwrap(),
            "润色开头"
        );
    }

    #[test]
    fn tool_rejects_paths_mentions_and_bad_input() {
        for bad in [
            r#"@"C:\\Users\\A\\x.md""#,
            r"C:\Users\A\x.md",
            "D:/books/x.md",
            "https://example.com/a",
            "   ",
        ] {
            assert!(run_title_tool(&json!({ "title": bad })).is_err(), "{bad}");
        }
        assert!(run_title_tool(&json!({})).is_err());
        assert!(run_title_tool(&json!({ "title": 3 })).is_err());
    }

    #[test]
    fn quoted_mention_becomes_the_document_name() {
        let p = r##"@"C:\\Users\\A\\Project\\杂文\\大纲.md"#P003-P007 设计后续 3 章的大纲"##;
        assert_eq!(title_source_text(p), "大纲 设计后续 3 章的大纲");
    }

    #[test]
    fn bare_media_mention_and_plain_at_are_handled() {
        assert_eq!(
            title_source_text(r"看看 @C:\pics\cover.png 这张图"),
            "看看 cover 这张图"
        );
        assert_eq!(title_source_text("联系 @小明 一下"), "联系 @小明 一下");
        assert_eq!(title_source_text("没有引用"), "没有引用");
    }

    #[test]
    fn fallback_title_never_comes_out_as_a_path() {
        let cleaned = title_source_text(r##"@"C:\\x\\大纲.md"#P001 写下一章"##);
        assert_eq!(fallback_title(&cleaned), "大纲 写下一章");
        assert_eq!(fallback_title(r"C:\x\y.md"), "");
    }

    #[test]
    fn blank_setting_falls_back_to_the_built_in_prompt() {
        let mut s = settings::Settings::default();
        s.quick_model_prompt = "   ".into();
        assert_eq!(
            title_system_prompt(&s),
            settings::DEFAULT_TITLE_SYSTEM_PROMPT
        );
        s.quick_model_prompt = "自定义".into();
        assert_eq!(title_system_prompt(&s), "自定义");
    }
}
