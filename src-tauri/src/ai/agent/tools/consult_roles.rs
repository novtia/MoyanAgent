//! `ConsultRoles` — TRPG director tool that consults multiple roles in parallel.
//!
//! Pipeline (v1, foreground):
//! 1. Load role cards from [`RoleStateStore`] for the session's role-state scope.
//! 2. Build a shared scene digest from `situation` + `question` (optionally compact).
//! 3. For each `role_id` in parallel:
//!    - `control == "user"` → mark `needs_ask_user` (director should call AskUser).
//!    - otherwise → one LLM call with the character prompt; append memory facts;
//!      return `{action, speech, reasoning_private}`.
//! 4. Aggregate JSON back to the parent (director).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use futures_util::future::join_all;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::ai::agent::config::builtin::AGENT_TRPG_CHARACTER;
use crate::ai::agent::config::definition::AgentDefinition;
use crate::ai::agent::config::prompts;
use crate::ai::agent::config::registry::AgentRegistry;
use crate::ai::agent::exec::engine::ProviderEngine;
use crate::ai::agent::tools::agent_tool::ChatRequestFactory;
use crate::ai::agent::tools::project_path::resolve_project_root;
use crate::ai::agent::tools::role_state::RoleStateStore;
use crate::ai::agent::tools::{Tool, ToolFuture, ToolInvocation, ToolResult, ToolSpec};
use crate::ai::chat::{ChatRequest, ProviderConfig};
use crate::ai::providers;
use crate::data::db::DbPool;
use crate::data::settings::{self, ModelProvider};
use crate::error::{AppError, AppResult};

pub const TOOL_NAME: &str = "ConsultRoles";

/// Rough char→token heuristic used for the shared scene digest threshold.
const CHARS_PER_TOKEN: usize = 4;
/// Soft threshold (~50k tokens) before we side-channel summarise the scene.
const COMPACT_TOKEN_THRESHOLD: usize = 50_000;
const SUMMARY_MAX_WORDS: u32 = 400;

const MEMORY_DIR: &str = ".moyan/trpg-memory";

#[derive(Debug, Clone, Deserialize)]
struct ConsultArgs {
    role_ids: Vec<String>,
    situation: String,
    question: String,
    #[serde(default)]
    force_compact: Option<bool>,
}

#[derive(Debug, Clone, Serialize)]
struct RoleChoice {
    role_id: String,
    name: String,
    control: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    action: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    speech: Option<String>,
    /// Kept in-process only — never returned to the director (information isolation).
    #[serde(skip_serializing)]
    reasoning_private: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    needs_ask_user: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    memory_path: Option<String>,
    /// Model id used for this AI role (card override or default).
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<String>,
}

/// Resolve `<project>/.moyan/trpg-memory/`, creating it when missing.
pub fn ensure_trpg_memory_dir(cwd: &Path) -> AppResult<PathBuf> {
    let root = resolve_project_root(cwd)?;
    let dir = root.join(MEMORY_DIR);
    std::fs::create_dir_all(&dir).map_err(|e| {
        AppError::Other(format!(
            "ConsultRoles: cannot create memory dir {}: {e}",
            dir.display()
        ))
    })?;
    Ok(dir)
}

/// Absolute path for a role's private memory markdown file (default location).
pub fn role_memory_path(cwd: &Path, role_id: &str) -> AppResult<PathBuf> {
    let safe = sanitize_role_id(role_id);
    Ok(ensure_trpg_memory_dir(cwd)?.join(format!("{safe}.md")))
}

/// Resolve absolute memory path honouring optional `memory_path` on the card.
/// Custom relative paths must stay inside the project root (no `..` escape).
pub fn resolve_role_memory_abs(
    cwd: &Path,
    role: &Value,
    role_id: &str,
) -> AppResult<(PathBuf, String)> {
    let root = resolve_project_root(cwd)?;
    let rel = normalize_memory_rel(&role_memory_rel_path(role, role_id))?;
    let abs = root.join(Path::new(&rel.replace('/', std::path::MAIN_SEPARATOR_STR)));
    if let Some(parent) = abs.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            AppError::Other(format!(
                "ConsultRoles: cannot create memory dir {}: {e}",
                parent.display()
            ))
        })?;
    }
    Ok((abs, rel))
}

fn normalize_memory_rel(rel: &str) -> AppResult<String> {
    let raw = rel.trim().replace('\\', "/");
    if raw.is_empty() {
        return Err(AppError::Invalid(
            "ConsultRoles: memory_path must be non-empty".into(),
        ));
    }
    if raw.starts_with('/') || Path::new(&raw).is_absolute() {
        return Err(AppError::Invalid(
            "ConsultRoles: memory_path must be project-relative".into(),
        ));
    }
    let mut parts: Vec<String> = Vec::new();
    for seg in raw.split('/') {
        let s = seg.trim();
        if s.is_empty() || s == "." {
            continue;
        }
        if s == ".." {
            return Err(AppError::Invalid(
                "ConsultRoles: memory_path must stay inside the project".into(),
            ));
        }
        // Reject Windows drive prefixes like `C:`.
        if s.len() == 2 && s.as_bytes()[1] == b':' {
            return Err(AppError::Invalid(
                "ConsultRoles: memory_path must be project-relative".into(),
            ));
        }
        parts.push(s.to_string());
    }
    if parts.is_empty() {
        return Err(AppError::Invalid(
            "ConsultRoles: memory_path resolved empty".into(),
        ));
    }
    Ok(parts.join("/"))
}

fn sanitize_role_id(role_id: &str) -> String {
    let trimmed = role_id.trim();
    if trimmed.is_empty() {
        return "unknown".into();
    }
    trimmed
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

fn estimate_tokens(text: &str) -> usize {
    text.chars().count().saturating_add(CHARS_PER_TOKEN - 1) / CHARS_PER_TOKEN
}

fn role_field<'a>(role: &'a Value, key: &str) -> Option<&'a str> {
    role.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

fn role_control(role: &Value) -> &str {
    match role_field(role, "control") {
        Some("user") => "user",
        _ => "ai",
    }
}

fn role_model_override(role: &Value) -> Option<String> {
    role_field(role, "model").map(str::to_string)
}

fn provider_config_from(provider: &ModelProvider) -> ProviderConfig {
    let sdk = providers::normalize_sdk(&provider.sdk);
    let context_cache_enabled = provider.context_cache_enabled
        && sdk == providers::OPENAI_RESPONSES_SDK
        && crate::ai::parameters::is_volcengine_endpoint(&provider.endpoint);
    ProviderConfig {
        id: provider.id.clone(),
        name: provider.name.clone(),
        sdk,
        endpoint: provider.endpoint.clone(),
        api_key: provider.api_key.clone(),
        context_cache_enabled,
    }
}

/// Apply optional per-role `model` onto a chat request (resolves provider).
fn apply_role_model(chat: &mut ChatRequest, settings: &settings::Settings, role: &Value) {
    let Some(model_id) = role_model_override(role) else {
        return;
    };
    let Some(provider) = settings::find_provider_for_model(settings, &model_id) else {
        return;
    };
    if provider.api_key.trim().is_empty() || provider.endpoint.trim().is_empty() {
        return;
    }
    chat.model = model_id;
    chat.provider = provider_config_from(provider);
    chat.context_cache_enabled = chat.provider.context_cache_enabled;
    // Model switch invalidates any previous Responses session cache.
    chat.previous_response_id = None;
}

fn role_name(role: &Value, fallback_id: &str) -> String {
    role_field(role, "name")
        .map(str::to_string)
        .unwrap_or_else(|| fallback_id.to_string())
}

fn role_memory_rel_path(role: &Value, role_id: &str) -> String {
    role_field(role, "memory_path")
        .map(str::to_string)
        .unwrap_or_else(|| format!("{MEMORY_DIR}/{}.md", sanitize_role_id(role_id)))
}

fn find_role<'a>(roles: &'a [Value], role_id: &str) -> Option<&'a Value> {
    roles.iter().find(|r| {
        r.get("id")
            .and_then(Value::as_str)
            .map(|id| id == role_id)
            .unwrap_or(false)
    })
}

fn read_memory_file(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_default()
}

fn append_memory_facts(path: &Path, facts: &[String]) -> AppResult<()> {
    if facts.is_empty() {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            AppError::Other(format!(
                "ConsultRoles: cannot create {}: {e}",
                parent.display()
            ))
        })?;
    }
    let mut existing = read_memory_file(path);
    if !existing.is_empty() && !existing.ends_with('\n') {
        existing.push('\n');
    }
    let stamp = chrono::Local::now().format("%Y-%m-%d %H:%M");
    existing.push_str(&format!("\n## {stamp}\n"));
    for fact in facts {
        let line = fact.trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('-') {
            existing.push_str(line);
            existing.push('\n');
        } else {
            existing.push_str("- ");
            existing.push_str(line);
            existing.push('\n');
        }
    }
    std::fs::write(path, existing).map_err(|e| {
        AppError::Other(format!(
            "ConsultRoles: cannot write memory {}: {e}",
            path.display()
        ))
    })?;
    Ok(())
}

fn build_scene_text(situation: &str, question: &str) -> String {
    format!(
        "## Situation\n{}\n\n## Question\n{}",
        situation.trim(),
        question.trim()
    )
}

async fn compact_scene(
    provider: &ProviderEngine,
    base: &ChatRequest,
    scene: &str,
) -> AppResult<String> {
    let mut req = base.clone();
    req.history.clear();
    req.tools.clear();
    req.tool_chain.clear();
    req.tool_results.clear();
    req.pending_assistant_turn = None;
    req.attachments.clear();
    req.previous_response_id = None;
    req.system_prompt = "\
You are a TRPG scene-compaction assistant. Summarise the public scene so \
role agents can decide. Preserve: location, visible events, public dialogue, \
known stakes, and the decision question. NEVER invent private knowledge, \
secrets known only to one character, or omniscient spoilers."
        .to_string();
    req.prompt = format!(
        "Produce a concise public scene digest in at most {SUMMARY_MAX_WORDS} words. \
Reply with the digest only.\n\n{scene}"
    );

    let resp = provider.run(req, None).await?;
    let summary = resp
        .text
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| scene.chars().take(8_000).collect());
    Ok(summary)
}

fn build_character_system(card: &Value, memory: &str, memory_rel: &str) -> String {
    let card_json = serde_json::to_string_pretty(card).unwrap_or_else(|_| "{}".into());
    let memory_block = if memory.trim().is_empty() {
        "(empty — no private notes yet)".to_string()
    } else {
        memory.trim().to_string()
    };
    format!(
        "{base}\n\n\
━━━ YOUR ROLE CARD (public board fields) ━━━\n\
```json\n{card_json}\n```\n\n\
━━━ YOUR PRIVATE MEMORY (`{memory_rel}`) ━━━\n\
{memory_block}\n",
        base = prompts::TRPG_CHARACTER_PROMPT,
        card_json = card_json,
        memory_rel = memory_rel,
        memory_block = memory_block,
    )
}

fn build_character_user_prompt(digest: &str, question: &str) -> String {
    format!(
        "## Shared scene digest\n{digest}\n\n\
## Decision question\n{question}\n\n\
Respond with ONE JSON object only (no markdown fences, no prose outside JSON):\n\
{{\n  \
\"memory_facts\": [\"…incremental private facts you newly learned or decided to remember…\"],\n  \
\"action\": \"what you do (short)\",\n  \
\"speech\": \"what you say out loud, or empty\",\n  \
\"reasoning_private\": \"private motive — never shared with other roles\"\n\
}}"
    )
}

fn extract_json_object(text: &str) -> Option<Value> {
    let trimmed = text.trim();
    if let Ok(v) = serde_json::from_str::<Value>(trimmed) {
        if v.is_object() {
            return Some(v);
        }
    }
    if let Some(start) = trimmed.find("```") {
        let after = &trimmed[start + 3..];
        let after = after
            .strip_prefix("json")
            .or_else(|| after.strip_prefix("JSON"))
            .unwrap_or(after)
            .trim_start();
        if let Some(end) = after.find("```") {
            let inner = after[..end].trim();
            if let Ok(v) = serde_json::from_str::<Value>(inner) {
                if v.is_object() {
                    return Some(v);
                }
            }
        }
    }
    if let (Some(i), Some(j)) = (trimmed.find('{'), trimmed.rfind('}')) {
        if j > i {
            if let Ok(v) = serde_json::from_str::<Value>(&trimmed[i..=j]) {
                if v.is_object() {
                    return Some(v);
                }
            }
        }
    }
    None
}

fn parse_memory_facts(v: &Value) -> Vec<String> {
    v.get("memory_facts")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|x| x.as_str().map(|s| s.trim().to_string()))
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

fn character_definition(registry: &AgentRegistry) -> AgentDefinition {
    registry.get(AGENT_TRPG_CHARACTER).unwrap_or_else(|| {
        AgentDefinition::builtin(AGENT_TRPG_CHARACTER, prompts::TRPG_CHARACTER_PROMPT)
    })
}

async fn run_ai_role(
    provider: Arc<ProviderEngine>,
    chat_factory: Arc<dyn ChatRequestFactory>,
    registry: Arc<AgentRegistry>,
    cwd: PathBuf,
    role_id: String,
    card: Value,
    digest: String,
    question: String,
    base_chat: ChatRequest,
    app_settings: settings::Settings,
) -> RoleChoice {
    let name = role_name(&card, &role_id);
    let model_hint = role_model_override(&card).or_else(|| {
        let m = base_chat.model.trim();
        if m.is_empty() {
            None
        } else {
            Some(m.to_string())
        }
    });
    let (mem_path, memory_rel) = match resolve_role_memory_abs(&cwd, &card, &role_id) {
        Ok(pair) => pair,
        Err(e) => {
            let memory_path = Some(role_memory_rel_path(&card, &role_id));
            return RoleChoice {
                role_id,
                name,
                control: "ai".into(),
                action: None,
                speech: None,
                reasoning_private: None,
                needs_ask_user: false,
                error: Some(e.to_string()),
                memory_path,
                model: model_hint,
            };
        }
    };
    let memory = read_memory_file(&mem_path);
    let definition = character_definition(registry.as_ref());
    let user_prompt = build_character_user_prompt(&digest, &question);

    let mut chat = match chat_factory.build(&user_prompt, AGENT_TRPG_CHARACTER, &definition) {
        Ok((c, _)) => c,
        Err(_) => {
            let mut fallback = base_chat.clone();
            fallback.prompt = user_prompt.clone();
            fallback
        }
    };

    chat.system_prompt = build_character_system(&card, &memory, &memory_rel);
    chat.prompt = user_prompt;
    chat.tools.clear();
    chat.tool_chain.clear();
    chat.tool_results.clear();
    chat.pending_assistant_turn = None;
    chat.history.clear();
    chat.attachments.clear();
    chat.previous_response_id = None;
    apply_role_model(&mut chat, &app_settings, &card);
    let used_model = Some(chat.model.clone());

    let resp = match provider.run(chat, None).await {
        Ok(r) => r,
        Err(e) => {
            return RoleChoice {
                role_id,
                name,
                control: "ai".into(),
                action: None,
                speech: None,
                reasoning_private: None,
                needs_ask_user: false,
                error: Some(format!("LLM error: {e}")),
                memory_path: Some(memory_rel),
                model: used_model,
            };
        }
    };

    let text = resp.text.unwrap_or_default();
    let parsed = extract_json_object(&text);
    let (action, speech, reasoning, facts) = match parsed.as_ref() {
        Some(v) => (
            v.get("action")
                .and_then(Value::as_str)
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty()),
            v.get("speech")
                .and_then(Value::as_str)
                .map(|s| s.to_string()),
            v.get("reasoning_private")
                .and_then(Value::as_str)
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty()),
            parse_memory_facts(v),
        ),
        None => {
            let t = text.trim().to_string();
            (
                if t.is_empty() { None } else { Some(t) },
                Some(String::new()),
                None,
                Vec::new(),
            )
        }
    };

    if let Err(e) = append_memory_facts(&mem_path, &facts) {
        return RoleChoice {
            role_id,
            name,
            control: "ai".into(),
            action,
            speech,
            reasoning_private: reasoning,
            needs_ask_user: false,
            error: Some(format!("memory write failed: {e}")),
            memory_path: Some(memory_rel),
            model: used_model,
        };
    }

    RoleChoice {
        role_id,
        name,
        control: "ai".into(),
        action,
        speech,
        reasoning_private: reasoning,
        needs_ask_user: false,
        error: None,
        memory_path: Some(memory_rel),
        model: used_model,
    }
}

/// The `ConsultRoles` tool. Holds provider + store + factory like RoleStateTool.
pub struct ConsultRolesTool {
    spec: ToolSpec,
    provider: Arc<ProviderEngine>,
    store: Arc<RoleStateStore>,
    chat_factory: Arc<dyn ChatRequestFactory>,
    registry: Arc<AgentRegistry>,
    pool: DbPool,
}

impl ConsultRolesTool {
    pub fn new(
        provider: Arc<ProviderEngine>,
        store: Arc<RoleStateStore>,
        chat_factory: Arc<dyn ChatRequestFactory>,
        registry: Arc<AgentRegistry>,
        pool: DbPool,
    ) -> Self {
        Self {
            provider,
            store,
            chat_factory,
            registry,
            pool,
            spec: ToolSpec {
                name: TOOL_NAME.to_string(),
                description: "\
Consult one or more in-scene roles for a TRPG decision. Pass the role ids on \
the board, a situation summary (public scene / recent context), and the \
decision question. The tool may compact a long scene once, then queries each \
AI-controlled role in parallel and returns structured choices.\n\n\
Roles with `control: \"user\"` are NOT decided by the LLM — the result marks \
`needs_ask_user: true` so you (the director) must call `AskUser` for them.\n\n\
Do NOT invent role decisions yourself — always call this tool when the plot \
needs character choices. After results arrive, narrate consequences from each \
role's `action` / `speech` only — never invent private motives for them."
                    .to_string(),
                schema: json!({
                    "type": "object",
                    "properties": {
                        "role_ids": {
                            "type": "array",
                            "items": { "type": "string" },
                            "minItems": 1,
                            "description": "Stable role ids currently in the scene (from RoleState)."
                        },
                        "situation": {
                            "type": "string",
                            "description": "Public scene summary / recent context visible to consulted roles."
                        },
                        "question": {
                            "type": "string",
                            "description": "The decision question each role should answer."
                        },
                        "force_compact": {
                            "type": "boolean",
                            "description": "If true, always summarise the scene before consulting roles."
                        }
                    },
                    "required": ["role_ids", "situation", "question"]
                }),
                read_only: false,
                concurrency_safe: false,
            },
        }
    }
}

impl Tool for ConsultRolesTool {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    fn validate(&self, input: &Value) -> AppResult<()> {
        let args: ConsultArgs = serde_json::from_value(input.clone()).map_err(|e| {
            AppError::Invalid(format!("ConsultRoles: invalid input: {e}"))
        })?;
        if args.role_ids.is_empty() {
            return Err(AppError::Invalid(
                "ConsultRoles: `role_ids` must be non-empty".into(),
            ));
        }
        if args.situation.trim().is_empty() {
            return Err(AppError::Invalid(
                "ConsultRoles: `situation` must be non-empty".into(),
            ));
        }
        if args.question.trim().is_empty() {
            return Err(AppError::Invalid(
                "ConsultRoles: `question` must be non-empty".into(),
            ));
        }
        Ok(())
    }

    fn execute<'a>(&'a self, invocation: ToolInvocation<'a>) -> ToolFuture<'a> {
        Box::pin(async move {
            let args: ConsultArgs = match serde_json::from_value(invocation.input.clone()) {
                Ok(a) => a,
                Err(e) => {
                    return Ok(ToolResult::error(format!(
                        "ConsultRoles input invalid: {e}"
                    )));
                }
            };

            let scope_id = match invocation
                .context
                .role_state_scope_id
                .clone()
                .or_else(|| invocation.context.session_id.clone())
            {
                Some(id) => id,
                None => {
                    return Ok(ToolResult::error(
                        "ConsultRoles: no role-state scope / session id on this run",
                    ));
                }
            };

            if invocation.context.cwd.as_os_str().is_empty() {
                return Ok(ToolResult::error(
                    "ConsultRoles: no project working directory; bind the session to a project \
                     so role memory can live under `.moyan/trpg-memory/`.",
                ));
            }
            let cwd = invocation.context.cwd.clone();

            if let Err(e) = ensure_trpg_memory_dir(&cwd) {
                return Ok(ToolResult::error(e.to_string()));
            }

            let board = self.store.snapshot(&scope_id);
            let scene = build_scene_text(&args.situation, &args.question);

            let app_settings = {
                let conn = match self.pool.get() {
                    Ok(c) => c,
                    Err(e) => {
                        return Ok(ToolResult::error(format!(
                            "ConsultRoles: cannot open db: {e}"
                        )));
                    }
                };
                match settings::read(&conn) {
                    Ok(s) => s,
                    Err(e) => {
                        return Ok(ToolResult::error(format!(
                            "ConsultRoles: cannot read settings: {e}"
                        )));
                    }
                }
            };

            let definition = character_definition(self.registry.as_ref());
            let (base_chat, _) = match self.chat_factory.build(
                "placeholder",
                AGENT_TRPG_CHARACTER,
                &definition,
            ) {
                Ok(pair) => pair,
                Err(e) => {
                    return Ok(ToolResult::error(format!(
                        "ConsultRoles: cannot build chat request: {e}"
                    )));
                }
            };

            let force = args.force_compact.unwrap_or(false);
            let need_compact = force || estimate_tokens(&scene) > COMPACT_TOKEN_THRESHOLD;
            let (digest, compacted) = if need_compact {
                match compact_scene(self.provider.as_ref(), &base_chat, &scene).await {
                    Ok(d) => (d, true),
                    Err(e) => {
                        let truncated: String = scene.chars().take(20_000).collect();
                        (
                            format!("{truncated}\n\n(compaction failed: {e}; truncated)"),
                            true,
                        )
                    }
                }
            } else {
                (scene, false)
            };

            let mut ordered: Vec<RoleChoice> = Vec::with_capacity(args.role_ids.len());
            let mut ai_futs = Vec::new();

            for role_id in &args.role_ids {
                let card = match find_role(&board, role_id) {
                    Some(c) => c.clone(),
                    None => {
                        ordered.push(RoleChoice {
                            role_id: role_id.clone(),
                            name: role_id.clone(),
                            control: "unknown".into(),
                            action: None,
                            speech: None,
                            reasoning_private: None,
                            needs_ask_user: false,
                            error: Some(format!(
                                "role `{role_id}` not found on RoleState board for this scope"
                            )),
                            memory_path: None,
                            model: None,
                        });
                        continue;
                    }
                };

                if role_control(&card) == "user" {
                    ordered.push(RoleChoice {
                        role_id: role_id.clone(),
                        name: role_name(&card, role_id),
                        control: "user".into(),
                        action: None,
                        speech: None,
                        reasoning_private: None,
                        needs_ask_user: true,
                        error: None,
                        memory_path: Some(role_memory_rel_path(&card, role_id)),
                        model: role_model_override(&card),
                    });
                    continue;
                }

                // Placeholder so we can splice AI results back in order.
                let idx = ordered.len();
                ordered.push(RoleChoice {
                    role_id: role_id.clone(),
                    name: role_name(&card, role_id),
                    control: "ai".into(),
                    action: None,
                    speech: None,
                    reasoning_private: None,
                    needs_ask_user: false,
                    error: Some("pending".into()),
                    memory_path: Some(role_memory_rel_path(&card, role_id)),
                    model: role_model_override(&card),
                });

                ai_futs.push((
                    idx,
                    run_ai_role(
                        self.provider.clone(),
                        self.chat_factory.clone(),
                        self.registry.clone(),
                        cwd.clone(),
                        role_id.clone(),
                        card,
                        digest.clone(),
                        args.question.clone(),
                        base_chat.clone(),
                        app_settings.clone(),
                    ),
                ));
            }

            let results = join_all(ai_futs.into_iter().map(|(idx, fut)| async move {
                (idx, fut.await)
            }))
            .await;

            for (idx, choice) in results {
                if let Some(slot) = ordered.get_mut(idx) {
                    *slot = choice;
                }
            }

            let ask_user_required = ordered.iter().any(|c| c.needs_ask_user);
            let ask_user_roles: Vec<Value> = ordered
                .iter()
                .filter(|c| c.needs_ask_user)
                .map(|c| {
                    json!({
                        "role_id": c.role_id,
                        "name": c.name,
                        "suggested_prompt": format!(
                            "【{}】{}",
                            c.name,
                            args.question.trim()
                        ),
                    })
                })
                .collect();

            Ok(ToolResult::ok(json!({
                "op": "consult",
                "compacted": compacted,
                "digest_chars": digest.chars().count(),
                "ask_user_required": ask_user_required,
                "ask_user_roles": ask_user_roles,
                "choices": ordered,
                "note": if ask_user_required {
                    "Some roles have control=user. Call AskUser for ask_user_roles before narrating their actions."
                } else {
                    "All AI roles returned choices. Narrate from action/speech only."
                },
            })))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn normalize_memory_rel_rejects_escape() {
        assert!(normalize_memory_rel("../secret.md").is_err());
        assert!(normalize_memory_rel("/abs.md").is_err());
        assert!(normalize_memory_rel("").is_err());
    }

    #[test]
    fn normalize_memory_rel_accepts_default() {
        let rel = normalize_memory_rel(".moyan/trpg-memory/alice.md").unwrap();
        assert_eq!(rel, ".moyan/trpg-memory/alice.md");
    }

    #[test]
    fn role_memory_rel_path_honours_custom() {
        let card = json!({ "memory_path": "notes/bob.md" });
        assert_eq!(role_memory_rel_path(&card, "bob"), "notes/bob.md");
        let empty = json!({});
        assert_eq!(
            role_memory_rel_path(&empty, "bob"),
            ".moyan/trpg-memory/bob.md"
        );
    }

    #[test]
    fn role_choice_hides_private_reasoning() {
        let choice = RoleChoice {
            role_id: "a".into(),
            name: "A".into(),
            control: "ai".into(),
            action: Some("draw".into()),
            speech: Some("hi".into()),
            reasoning_private: Some("secret".into()),
            needs_ask_user: false,
            error: None,
            memory_path: None,
            model: Some("gpt-test".into()),
        };
        let v = serde_json::to_value(&choice).unwrap();
        assert!(v.get("reasoning_private").is_none());
        assert_eq!(v.get("action").and_then(|x| x.as_str()), Some("draw"));
    }

    #[test]
    fn role_control_defaults_ai() {
        assert_eq!(role_control(&json!({})), "ai");
        assert_eq!(role_control(&json!({ "control": "user" })), "user");
        assert_eq!(role_control(&json!({ "control": "AI" })), "ai");
    }
}
