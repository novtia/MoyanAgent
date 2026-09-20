//! `NovelAI` — generate an image with NovelAI Diffusion V5 and save it
//! inside the current project.
//!
//! Token, model, sampler, quality tags and default size come from settings
//! (read live on every call). The model only supplies the prompt and optional
//! overrides (size, seed, character pins). Images are always written to the
//! `novelai` directory inside the current project.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::{json, Value};

use crate::ai::agent::core::file_snapshot::{FileOp, FileSnapshotStore};
use crate::ai::agent::tools::project_path::display_path;
use crate::ai::agent::tools::{Tool, ToolFuture, ToolInvocation, ToolResult, ToolSpec};
use crate::ai::nai::{self, CharacterPrompt, GenerateRequest};
use crate::data::db::DbPool;
use crate::data::settings;
use crate::error::{AppError, AppResult};

const TOOL_NAME: &str = "NovelAI";
const OUTPUT_DIR: &str = "novelai";

pub struct NovelAITool {
    spec: ToolSpec,
    pool: Arc<DbPool>,
    snapshots: Arc<FileSnapshotStore>,
}

impl NovelAITool {
    pub fn new(pool: Arc<DbPool>, snapshots: Arc<FileSnapshotStore>) -> Self {
        Self {
            spec: ToolSpec {
                name: TOOL_NAME.to_string(),
                description: "Generate an illustration with NovelAI Diffusion V5 \
                    (text-to-image) and save the PNG inside the current project. \
                    Use this when the user asks for a character portrait, scene, \
                    cover, or other still image. Danbooru-style tags plus short \
                    natural language work best. Default model, size, sampler, \
                    quality tags, artist chain and negative prompt come from the \
                    user's NovelAI tool settings — only override them when the \
                    user asks. Do not repeat the user's artist chain in `prompt`; \
                    it is prepended automatically. PNGs are always saved to the \
                    project's `novelai` directory. Requires a Persistent API Token \
                    in Settings → Tools → NovelAI."
                    .to_string(),
                schema: json!({
                    "type": "object",
                    "properties": {
                        "prompt": {
                            "type": "string",
                            "description": "Positive prompt: subject, clothing, pose, scene. Prefer Danbooru tags plus a short natural-language sentence. Do not include the user's configured artist chain — it is prepended automatically."
                        },
                        "uc": {
                            "type": "string",
                            "description": "Negative prompt (undesired content). Omit to use the user's configured default."
                        },
                        "width": {
                            "type": "integer",
                            "minimum": 64,
                            "maximum": 2048,
                            "description": "Image width in pixels (snapped to a multiple of 64). Omit to use the setting default."
                        },
                        "height": {
                            "type": "integer",
                            "minimum": 64,
                            "maximum": 2048,
                            "description": "Image height in pixels (snapped to a multiple of 64). Omit to use the setting default."
                        },
                        "seed": {
                            "type": "integer",
                            "description": "Seed. Negative or omitted picks a random seed."
                        },
                        "n_samples": {
                            "type": "integer",
                            "minimum": 1,
                            "maximum": 4,
                            "description": "How many images to generate (1–4). Defaults to 1."
                        },
                        "title": {
                            "type": "string",
                            "description": "File-name stem for the saved PNG (without extension). Defaults to `nai`."
                        },
                        "characters": {
                            "type": "array",
                            "description": "Optional per-character prompts with optional pin coordinates (x/y 0–1). Up to 22 characters.",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "prompt": { "type": "string", "description": "Character prompt, e.g. `girl, red hair, coat`." },
                                    "uc": { "type": "string", "description": "Per-character negative prompt." },
                                    "x": { "type": "number", "minimum": 0, "maximum": 1, "description": "Horizontal pin 0–1. Default 0.5." },
                                    "y": { "type": "number", "minimum": 0, "maximum": 1, "description": "Vertical pin 0–1. Default 0.5." }
                                },
                                "required": ["prompt"]
                            }
                        }
                    },
                    "required": ["prompt"]
                }),
                read_only: false,
                concurrency_safe: false,
            },
            pool,
            snapshots,
        }
    }
}

impl Tool for NovelAITool {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    fn validate(&self, input: &Value) -> AppResult<()> {
        let prompt = input
            .get("prompt")
            .and_then(Value::as_str)
            .ok_or_else(|| AppError::Invalid(format!("{TOOL_NAME}: `prompt` must be a string")))?;
        if prompt.trim().is_empty() {
            return Err(AppError::Invalid(format!(
                "{TOOL_NAME}: `prompt` must be non-empty"
            )));
        }
        if let Some(n) = input.get("n_samples") {
            let n = n.as_i64().ok_or_else(|| {
                AppError::Invalid(format!("{TOOL_NAME}: `n_samples` must be an integer"))
            })?;
            if !(1..=4).contains(&n) {
                return Err(AppError::Invalid(format!(
                    "{TOOL_NAME}: `n_samples` must be between 1 and 4"
                )));
            }
        }
        if let Some(w) = input.get("width") {
            if w.as_i64().is_none() && w.as_f64().is_none() {
                return Err(AppError::Invalid(format!(
                    "{TOOL_NAME}: `width` must be a number"
                )));
            }
        }
        if let Some(h) = input.get("height") {
            if h.as_i64().is_none() && h.as_f64().is_none() {
                return Err(AppError::Invalid(format!(
                    "{TOOL_NAME}: `height` must be a number"
                )));
            }
        }
        if let Some(chars) = input.get("characters") {
            if !chars.is_array() {
                return Err(AppError::Invalid(format!(
                    "{TOOL_NAME}: `characters` must be an array"
                )));
            }
        }
        Ok(())
    }

    fn execute<'a>(&'a self, invocation: ToolInvocation<'a>) -> ToolFuture<'a> {
        Box::pin(async move {
            let prompt = invocation
                .input
                .get("prompt")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            if prompt.trim().is_empty() {
                return Ok(ToolResult::error(format!(
                    "{TOOL_NAME}: `prompt` must be non-empty"
                )));
            }

            let uc = invocation
                .input
                .get("uc")
                .and_then(Value::as_str)
                .map(|s| s.to_string());
            let width = int_arg(&invocation.input, "width").map(|n| n.clamp(64, 2048) as u32);
            let height = int_arg(&invocation.input, "height").map(|n| n.clamp(64, 2048) as u32);
            let seed = invocation.input.get("seed").and_then(json_i64);
            let n_samples = int_arg(&invocation.input, "n_samples").map(|n| n.clamp(1, 4) as u32);
            let title = invocation
                .input
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim()
                .to_string();
            let characters: Vec<CharacterPrompt> = invocation
                .input
                .get("characters")
                .map(nai::parse_characters)
                .unwrap_or_default();

            let conn = self.pool.get().map_err(AppError::from)?;
            let config = settings::read_novelai_config(&conn);
            drop(conn);

            if config.api_key.trim().is_empty() {
                return Ok(ToolResult::error(
                    "还没有配置 NovelAI Persistent API Token。请到设置 → 工具 → NovelAI 填写。",
                ));
            }

            let dir = match resolve_output_dir(&invocation.context.cwd) {
                Ok(d) => d,
                Err(e) => return Ok(ToolResult::error(e.to_string())),
            };
            std::fs::create_dir_all(&dir).map_err(|e| {
                AppError::Other(format!("{TOOL_NAME}: mkdir {:?}: {e}", dir))
            })?;

            let request = GenerateRequest {
                prompt,
                uc,
                width,
                height,
                seed,
                n_samples,
                characters,
            };

            let outcome = tokio::select! {
                _ = invocation.context.abort.wait_aborted() => {
                    return Ok(ToolResult::error("generation cancelled"));
                }
                result = nai::generate_images(&config, &request) => result,
            };

            let (images, meta) = match outcome {
                Ok(pair) => pair,
                Err(e) => return Ok(ToolResult::error(e.to_string())),
            };
            if images.is_empty() {
                return Ok(ToolResult::error("NovelAI 没有返回图片"));
            }

            let stem_base = sanitize_file_name(if title.is_empty() { "nai" } else { &title });
            let mut saved = Vec::new();
            for (i, bytes) in images.iter().enumerate() {
                let stem = if images.len() == 1 {
                    format!("{stem_base}-{}", meta.seed)
                } else {
                    format!("{stem_base}-{}-{}", meta.seed, i + 1)
                };
                let path = unique_png_path(&dir, &stem);
                self.snapshots.record_before(
                    invocation.context.session_id.as_deref(),
                    invocation.context.correlation_id.as_deref(),
                    &path,
                    FileOp::Create,
                )?;
                std::fs::write(&path, bytes).map_err(|e| {
                    AppError::Other(format!("{TOOL_NAME}: write {:?}: {e}", path))
                })?;
                let (w, h) = nai::png_size(bytes).unwrap_or((meta.width, meta.height));
                saved.push(json!({
                    "path": display_path(&path),
                    "width": w,
                    "height": h,
                    "bytes": bytes.len(),
                }));
            }

            let content = json!({
                "images": saved,
                "model": meta.model,
                "seed": meta.seed,
                "width": meta.width,
                "height": meta.height,
                "steps": meta.steps,
                "sampler": meta.sampler,
                "scale": meta.scale,
                "prompt": meta.prompt,
                "uc": meta.uc,
            });
            let mut result = ToolResult::ok(content);
            result.metadata = Some(json!({
                "kind": "novelai",
                "count": saved.len(),
                "seed": meta.seed,
                "model": meta.model,
            }));
            Ok(result)
        })
    }
}

fn resolve_output_dir(cwd: &Path) -> AppResult<PathBuf> {
    if cwd.as_os_str().is_empty() || !cwd.is_absolute() {
        return Err(AppError::Invalid(
            "NovelAI: 当前会话没有绑定项目，无法保存图片。请先绑定一个项目。".into(),
        ));
    }
    let root_canon = std::fs::canonicalize(cwd).unwrap_or_else(|_| cwd.to_path_buf());
    Ok(root_canon.join(OUTPUT_DIR))
}

fn sanitize_file_name(title: &str) -> String {
    let mut s: String = title
        .trim()
        .trim_end_matches(".png")
        .trim_end_matches(".PNG")
        .chars()
        .map(|c| match c {
            '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();
    s = s.trim().trim_end_matches('.').trim().to_string();
    if s.is_empty() {
        "nai".into()
    } else {
        s
    }
}

fn json_i64(v: &Value) -> Option<i64> {
    v.as_i64()
        .or_else(|| v.as_u64().map(|n| n as i64))
        .or_else(|| v.as_f64().map(|n| n as i64))
}

fn int_arg(input: &Value, key: &str) -> Option<i64> {
    input.get(key).and_then(json_i64)
}

fn unique_png_path(dir: &Path, stem: &str) -> PathBuf {
    let mut path = dir.join(format!("{stem}.png"));
    if !path.exists() {
        return path;
    }
    for i in 2u32..=1000 {
        path = dir.join(format!("{stem}-{i}.png"));
        if !path.exists() {
            return path;
        }
    }
    dir.join(format!("{stem}-{}.png", ulid::Ulid::new()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::agent::core::context::ToolUseContextBuilder;
    use crate::ai::agent::types::{AgentId, MessageId};
    use crate::data::db::test_support::TempDb;

    fn tool() -> (TempDb, NovelAITool) {
        let db = TempDb::new("novelai-tool");
        let tool = NovelAITool::new(Arc::new(db.pool()), Arc::new(FileSnapshotStore::new()));
        (db, tool)
    }

    #[test]
    fn validate_rejects_empty_prompt() {
        let (_db, tool) = tool();
        let err = tool.validate(&json!({ "prompt": "  " })).unwrap_err();
        assert!(err.to_string().contains("prompt"));
    }

    #[test]
    fn validate_rejects_missing_prompt() {
        let (_db, tool) = tool();
        let err = tool.validate(&json!({})).unwrap_err();
        assert!(err.to_string().contains("prompt"));
    }

    #[test]
    fn validate_accepts_prompt() {
        let (_db, tool) = tool();
        tool.validate(&json!({ "prompt": "1girl, solo" })).unwrap();
    }

    #[test]
    fn output_dir_is_project_novelai() {
        let dir = std::env::temp_dir().join(format!(
            "moyan-nai-outdir-{}-{}",
            std::process::id(),
            ulid::Ulid::new()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let canon = std::fs::canonicalize(&dir).unwrap();
        let out = resolve_output_dir(&dir).unwrap();
        assert_eq!(out, canon.join("novelai"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn output_dir_requires_project() {
        let err = resolve_output_dir(Path::new("")).unwrap_err();
        assert!(err.to_string().contains("项目"), "{err}");
    }

    #[tokio::test]
    async fn execute_without_token_errors() {
        let (_db, tool) = tool();
        let dir = std::env::temp_dir().join(format!(
            "moyan-nai-test-{}-{}",
            std::process::id(),
            ulid::Ulid::new()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let (ctx, _) = ToolUseContextBuilder::new(AgentId::new(), dir).build();
        let result = tool
            .execute(ToolInvocation {
                id: MessageId("nai".into()),
                input: json!({ "prompt": "1girl" }),
                context: ctx.as_ref(),
            })
            .await
            .unwrap();
        assert!(result.is_error);
        let msg = result.content.get("error").and_then(Value::as_str).unwrap();
        assert!(msg.contains("Token"), "{msg}");
    }
}
