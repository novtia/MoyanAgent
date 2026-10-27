//! Reader rewrite / expand / pipeline. When auto-chunk is on, long text is
//! cut near the configured length on paragraph and sentence boundaries and
//! sent serially. When it is off, the whole passage goes out in one request.

use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use serde_json::{json, Map};
use tauri::{AppHandle, Emitter, State};

use crate::ai::agent::core::context::{AbortHandle, AbortSignal};
use crate::ai::chat::{ChatRequest, StreamDelta, TextDeltaCallback};
use crate::ai::parameters::{self, GenerationParameters};
use crate::ai::providers::ProviderFactory;
use crate::ai::rewrite_config::RewriteSettings;
use crate::data::settings::{self, ModelParamSettings, ModelProvider};
use crate::error::{AppError, AppResult};

use super::state::AppState;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RewriteMode {
    Rewrite,
    Expand,
    Pipeline,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stage {
    Rewrite,
    Expand,
}

impl Stage {
    fn as_str(self) -> &'static str {
        match self {
            Stage::Rewrite => "rewrite",
            Stage::Expand => "expand",
        }
    }

    fn instruction<'a>(self, cfg: &'a RewriteSettings) -> &'a str {
        match self {
            Stage::Rewrite => cfg.rewrite_instruction.as_str(),
            Stage::Expand => cfg.expand_instruction.as_str(),
        }
    }

    fn prompt_enabled(self, cfg: &RewriteSettings) -> bool {
        match self {
            Stage::Rewrite => cfg.rewrite_prompt_enabled,
            Stage::Expand => cfg.expand_prompt_enabled,
        }
    }

    fn max_tokens(self, cfg: &RewriteSettings) -> i64 {
        match self {
            Stage::Rewrite => cfg.rewrite_max_tokens,
            Stage::Expand => cfg.expand_max_tokens,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReaderRewriteArgs {
    pub path: String,
    pub text: String,
    pub mode: String,
    pub provider_id: String,
    pub model_id: String,
    pub client_token: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RewriteProgress<'a> {
    path: &'a str,
    client_token: &'a str,
    kind: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    stage: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    index: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    total: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
}

enum Step {
    Text(String),
    Cancelled,
}

struct ClearRewriteAbort<'a> {
    slot: &'a Mutex<Option<AbortHandle>>,
}

impl Drop for ClearRewriteAbort<'_> {
    fn drop(&mut self) {
        if let Ok(mut guard) = self.slot.lock() {
            *guard = None;
        }
    }
}

#[tauri::command]
pub async fn start_reader_rewrite(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    args: ReaderRewriteArgs,
) -> Result<(), AppError> {
    let mode = parse_mode(&args.mode)?;
    let text = args.text.replace("\r\n", "\n").replace('\r', "\n");
    if text.trim().is_empty() {
        return Err(AppError::Other("没有可改写的正文".into()));
    }
    let rewrite_cfg = load_rewrite_settings(&state)?;
    let (provider, model_id, context_window) = load_rewrite_target(&state, &args)?;
    let chunks = if rewrite_cfg.auto_chunk {
        let chunks = merge_short_chunks(
            split_chunks(&text, rewrite_cfg.chunk_chars as usize),
            MIN_INPUT_CHARS,
        );
        if chunks.is_empty() {
            return Err(AppError::Other("没有可改写的正文".into()));
        }
        chunks
    } else {
        vec![text.trim().to_string()]
    };
    let (signal, handle) = AbortSignal::new();
    {
        let mut slot = state
            .rewrite_abort
            .lock()
            .map_err(|_| AppError::Other("改文锁不可用".into()))?;
        if slot.is_some() {
            return Err(AppError::Other("改文正在进行".into()));
        }
        *slot = Some(handle);
    }
    let _clear = ClearRewriteAbort {
        slot: &state.rewrite_abort,
    };

    let factory = ProviderFactory::default();
    let total = chunks.len();
    let mut visible = String::new();

    for (offset, chunk) in chunks.iter().enumerate() {
        if signal.aborted() {
            emit_cancelled(&app, &args);
            return Ok(());
        }
        let index = offset + 1;
        let mut carried = chunk.clone();
        for stage in stages(mode) {
            let stage = *stage;
            if signal.aborted() {
                emit_cancelled(&app, &args);
                return Ok(());
            }
            emit(
                &app,
                RewriteProgress {
                    path: &args.path,
                    client_token: &args.client_token,
                    kind: "stage",
                    stage: Some(stage.as_str()),
                    index: Some(index),
                    total: Some(total),
                    text: None,
                    message: None,
                },
            );
            let show = stage_is_visible(mode, stage);
            if show && !visible.is_empty() {
                emit(
                    &app,
                    RewriteProgress {
                        path: &args.path,
                        client_token: &args.client_token,
                        kind: "delta",
                        stage: None,
                        index: None,
                        total: None,
                        text: Some("\n\n".into()),
                        message: None,
                    },
                );
            }
            match complete_stage(
                &factory,
                &provider,
                rewrite_cfg.auto_chunk,
                &model_id,
                context_window,
                stage,
                &rewrite_cfg,
                &carried,
                &signal,
                show,
                &app,
                &args,
            )
            .await?
            {
                Step::Cancelled => {
                    emit_cancelled(&app, &args);
                    return Ok(());
                }
                Step::Text(next) => {
                    let next = next.trim().to_string();
                    if next.is_empty() {
                        let message = format!("第 {index} 段没有返回正文");
                        emit_error(&app, &args, &message);
                        return Err(AppError::Other(message));
                    }
                    if show {
                        if !visible.is_empty() {
                            visible.push_str("\n\n");
                        }
                        visible.push_str(&next);
                    }
                    carried = next;
                }
            }
        }
    }

    emit(
        &app,
        RewriteProgress {
            path: &args.path,
            client_token: &args.client_token,
            kind: "done",
            stage: None,
            index: Some(total),
            total: Some(total),
            text: Some(visible),
            message: None,
        },
    );
    Ok(())
}

#[tauri::command]
pub fn cancel_reader_rewrite(state: State<'_, Arc<AppState>>) -> Result<(), AppError> {
    let slot = state
        .rewrite_abort
        .lock()
        .map_err(|_| AppError::Other("改文锁不可用".into()))?;
    if let Some(handle) = slot.as_ref() {
        handle.abort();
    }
    Ok(())
}

fn parse_mode(raw: &str) -> AppResult<RewriteMode> {
    match raw.trim() {
        "rewrite" => Ok(RewriteMode::Rewrite),
        "expand" => Ok(RewriteMode::Expand),
        "pipeline" => Ok(RewriteMode::Pipeline),
        other => Err(AppError::Other(format!("未知改文模式：{other}"))),
    }
}

fn stages(mode: RewriteMode) -> &'static [Stage] {
    match mode {
        RewriteMode::Rewrite => &[Stage::Rewrite],
        RewriteMode::Expand => &[Stage::Expand],
        RewriteMode::Pipeline => &[Stage::Rewrite, Stage::Expand],
    }
}

fn load_rewrite_settings(state: &AppState) -> AppResult<RewriteSettings> {
    let conn = state.conn()?;
    Ok(settings::read(&conn)?.rewrite)
}

fn stage_is_visible(mode: RewriteMode, stage: Stage) -> bool {
    match mode {
        RewriteMode::Rewrite => stage == Stage::Rewrite,
        RewriteMode::Expand => stage == Stage::Expand,
        RewriteMode::Pipeline => stage == Stage::Expand,
    }
}

fn load_rewrite_target(
    state: &AppState,
    args: &ReaderRewriteArgs,
) -> AppResult<(ModelProvider, String, Option<i64>)> {
    let conn = state.conn()?;
    let settings = settings::read(&conn)?;
    let provider = settings
        .model_services
        .into_iter()
        .find(|provider| provider.id == args.provider_id)
        .ok_or_else(|| AppError::Other("找不到改文供应商".into()))?;
    if !provider.enabled {
        return Err(AppError::Other("改文供应商已停用".into()));
    }
    if provider.endpoint.trim().is_empty() {
        return Err(AppError::Other("改文供应商没有 API 地址".into()));
    }
    let sdk = crate::ai::providers::normalize_sdk(&provider.sdk);
    if provider.api_key.trim().is_empty() && sdk != crate::ai::providers::LOCAL_SDK {
        return Err(AppError::Other("改文供应商的 API 密钥不能为空".into()));
    }
    let model = provider
        .models
        .iter()
        .find(|model| model.id == args.model_id)
        .ok_or_else(|| AppError::Other("找不到改文模型".into()))?;
    if !model.capabilities.iter().any(|cap| cap == "rewrite") {
        return Err(AppError::Other("所选模型不是改文类型".into()));
    }
    let context_window = model.context_window;
    let model_id = model.id.clone();
    Ok((provider, model_id, context_window))
}

async fn complete_stage(
    factory: &ProviderFactory,
    provider: &ModelProvider,
    auto_chunk: bool,
    model_id: &str,
    context_window: Option<i64>,
    stage: Stage,
    cfg: &RewriteSettings,
    chunk: &str,
    signal: &AbortSignal,
    emit_tokens: bool,
    app: &AppHandle,
    args: &ReaderRewriteArgs,
) -> AppResult<Step> {
    let body = chunk.trim();
    let prompt = if stage.prompt_enabled(cfg) {
        format!("{}\n{}", stage.instruction(cfg).trim_end(), body)
    } else {
        body.to_string()
    };
    let request = ChatRequest {
        provider: crate::ai::chat::ProviderConfig {
            id: provider.id.clone(),
            name: provider.name.clone(),
            sdk: provider.sdk.clone(),
            endpoint: provider.endpoint.clone(),
            api_key: provider.api_key.clone(),
            context_cache_enabled: false,
            safety_threshold: None,
        },
        model: model_id.to_string(),
        prompt,
        attachments: Vec::new(),
        system_prompt: String::new(),
        history: Vec::new(),
        parameters: rewrite_parameters(stage, cfg, auto_chunk),
        tools: Vec::new(),
        tool_chain: Vec::new(),
        tool_results: Vec::new(),
        pending_assistant_turn: None,
        previous_response_id: None,
        context_cache_enabled: false,
        context_window,
        todo_snapshot: None,
        route_providers: Vec::new(),
        forced_tools: Vec::new(),
    };

    let acc = Arc::new(Mutex::new(String::new()));
    let on_delta: TextDeltaCallback = {
        let acc = acc.clone();
        Arc::new(move |delta: StreamDelta| {
            let Some(text) = delta.text.filter(|text| !text.is_empty()) else {
                return;
            };
            if let Ok(mut buf) = acc.lock() {
                buf.push_str(&text);
            }
        })
    };

    let run = factory.chat_stream(request, on_delta);
    let result = tokio::select! {
        _ = signal.wait_aborted() => return Ok(Step::Cancelled),
        result = run => result,
    };
    let response = match result {
        Ok(response) => response,
        Err(err) => return Err(rewrite_failure(err)),
    };
    let streamed = acc.lock().map(|buf| buf.clone()).unwrap_or_default();
    let text = if streamed.trim().is_empty() {
        response.text.unwrap_or_default()
    } else {
        response.text.unwrap_or(streamed)
    };
    let split = crate::ai::rewrite_cot::split_gateway_message(
        Some(&text),
        response.thinking_content.as_deref(),
        None,
        None,
        None,
    );
    if split.body.trim().is_empty() {
        let message = if !split.thinking.trim().is_empty() {
            "思考未写完，没有正文"
        } else {
            "模型没有返回正文"
        };
        return Err(AppError::Other(message.into()));
    }
    if emit_tokens && !split.thinking.trim().is_empty() {
        emit(
            app,
            RewriteProgress {
                path: &args.path,
                client_token: &args.client_token,
                kind: "thinking",
                stage: None,
                index: None,
                total: None,
                text: Some(split.thinking.trim().to_string()),
                message: None,
            },
        );
    }
    if emit_tokens {
        emit(
            app,
            RewriteProgress {
                path: &args.path,
                client_token: &args.client_token,
                kind: "delta",
                stage: None,
                index: None,
                total: None,
                text: Some(split.body.clone()),
                message: None,
            },
        );
    }
    Ok(Step::Text(split.body))
}

fn rewrite_parameters(
    stage: Stage,
    cfg: &RewriteSettings,
    auto_chunk: bool,
) -> GenerationParameters {
    let mut params = parameters::factory().build(
        "auto".into(),
        "auto".into(),
        ModelParamSettings {
            temperature: Some(cfg.temperature),
            top_p: Some(cfg.top_p),
            // A per-chunk cap would cut a single full-passage request short.
            max_tokens: if auto_chunk {
                Some(stage.max_tokens(cfg))
            } else {
                None
            },
            ..ModelParamSettings::default()
        },
    );
    let mut custom = Map::new();
    custom.insert("top_k".into(), json!(cfg.top_k));
    custom.insert("repeat_penalty".into(), json!(cfg.repeat_penalty));
    custom.insert("repetition_penalty".into(), json!(cfg.repeat_penalty));
    params.custom = custom;
    params
}

fn rewrite_failure(err: AppError) -> AppError {
    let raw = err.to_string();
    let lower = raw.to_ascii_lowercase();
    let refused = lower.contains("connection refused")
        || lower.contains("os error 10061")
        || lower.contains("actively refused")
        || lower.contains("error trying to connect")
        || lower.contains("tcp connect");
    if raw.contains("输入太短了") {
        return AppError::Other("原文不到 400 字，北陵拒收了。再短模型容易复读。".into());
    }
    if refused {
        AppError::Other(
            "连不上本地改文服务。请到设置里打开「本地改文」供应商，点击「启动本地服务」。".into(),
        )
    } else {
        err
    }
}

fn emit_error(app: &AppHandle, args: &ReaderRewriteArgs, message: &str) {
    emit(
        app,
        RewriteProgress {
            path: &args.path,
            client_token: &args.client_token,
            kind: "error",
            stage: None,
            index: None,
            total: None,
            text: None,
            message: Some(message.to_string()),
        },
    );
}

fn emit_cancelled(app: &AppHandle, args: &ReaderRewriteArgs) {
    emit(
        app,
        RewriteProgress {
            path: &args.path,
            client_token: &args.client_token,
            kind: "cancelled",
            stage: None,
            index: None,
            total: None,
            text: None,
            message: None,
        },
    );
}

fn emit(app: &AppHandle, progress: RewriteProgress<'_>) {
    let _ = app.emit("rewrite://progress", &progress);
}

/// Pieces shorter than this are joined to a neighbor while auto-chunk is on.
const MIN_INPUT_CHARS: usize = 400;

/// Join a chunk that is under `min_chars` with the previous one. Sentences
/// stay intact; the joined chunk may run past the split limit.
fn merge_short_chunks(chunks: Vec<String>, min_chars: usize) -> Vec<String> {
    if min_chars == 0 {
        return chunks;
    }
    let mut out: Vec<String> = Vec::new();
    for chunk in chunks {
        let short = chunk.chars().count() < min_chars;
        if let Some(prev) = out.last_mut() {
            if short || prev.chars().count() < min_chars {
                prev.push('\n');
                prev.push_str(&chunk);
                continue;
            }
        }
        out.push(chunk);
    }
    out
}

/// Pack paragraphs that fit in 700 characters. A longer paragraph is split
/// only on `。！？`; if the next sentence end is past the limit, the cut moves
/// forward to that end. Text with no sentence end is kept whole.
pub(crate) fn split_chunks(text: &str, limit: usize) -> Vec<String> {
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    if text.trim().is_empty() {
        return Vec::new();
    }
    let paragraphs: Vec<&str> = text
        .split('\n')
        .map(str::trim)
        .filter(|para| !para.is_empty())
        .collect();
    let mut chunks = Vec::new();
    let mut buf = String::new();
    let mut buf_len = 0usize;

    for para in paragraphs {
        let pieces = split_long_paragraph(para, limit);
        for piece in pieces {
            let piece_len = piece.chars().count();
            if buf.is_empty() {
                buf = piece;
                buf_len = piece_len;
                continue;
            }
            let extra = piece_len + 1;
            if buf_len + extra > limit {
                chunks.push(std::mem::take(&mut buf));
                buf = piece;
                buf_len = piece_len;
                continue;
            }
            buf.push('\n');
            buf.push_str(&piece);
            buf_len += extra;
        }
    }
    if !buf.is_empty() {
        chunks.push(buf);
    }
    chunks
}

fn split_long_paragraph(para: &str, limit: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = para;
    while rest.chars().count() > limit {
        let cut = sentence_cut(rest, limit);
        let total = rest.chars().count();
        if cut == 0 || cut >= total {
            break;
        }
        let (head, tail) = split_at_chars(rest, cut);
        let head = head.trim().to_string();
        if head.is_empty() {
            break;
        }
        out.push(head);
        rest = tail.trim();
        if rest.is_empty() {
            break;
        }
    }
    if !rest.is_empty() {
        out.push(rest.to_string());
    }
    out
}

/// Exclusive cut after a sentence end. Prefers the last `。！？` inside
/// `limit`. If there is none, uses the next sentence end past the limit.
/// Returns the full length when the text never ends a sentence.
fn sentence_cut(text: &str, limit: usize) -> usize {
    let mut last_inside = None;
    let mut first_after = None;
    let mut count = 0usize;
    for ch in text.chars() {
        count += 1;
        if is_sentence_end(ch) {
            if count <= limit {
                last_inside = Some(count);
            } else if first_after.is_none() {
                first_after = Some(count);
                break;
            }
        }
    }
    if let Some(cut) = last_inside {
        return cut;
    }
    if let Some(cut) = first_after {
        return cut;
    }
    text.chars().count()
}

fn is_sentence_end(ch: char) -> bool {
    matches!(ch, '。' | '！' | '？' | '!' | '?')
}

fn split_at_chars(text: &str, chars: usize) -> (String, &str) {
    let mut end = 0usize;
    for (count, (idx, ch)) in text.char_indices().enumerate() {
        if count == chars {
            break;
        }
        end = idx + ch.len_utf8();
    }
    let (head, tail) = text.split_at(end);
    (head.to_string(), tail)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chars(n: usize, ch: char) -> String {
        std::iter::repeat(ch).take(n).collect()
    }

    #[test]
    fn keeps_a_short_paragraph_whole() {
        let text = chars(400, '甲');
        assert_eq!(split_chunks(&text, 700), vec![text]);
    }

    #[test]
    fn cuts_before_the_paragraph_that_would_pass_the_limit() {
        let first = chars(600, '甲');
        let second = chars(500, '乙');
        let text = format!("{first}\n{second}");
        assert_eq!(split_chunks(&text, 700), vec![first, second]);
    }

    #[test]
    fn packs_paragraphs_that_fit() {
        let a = chars(300, '甲');
        let b = chars(300, '乙');
        let text = format!("{a}\n{b}");
        assert_eq!(split_chunks(&text, 700), vec![text]);
    }

    #[test]
    fn splits_a_long_paragraph_on_the_last_sentence_inside_the_limit() {
        let head = format!("{}。", chars(500, '甲'));
        let tail = chars(400, '乙');
        let text = format!("{head}{tail}");
        let chunks = split_chunks(&text, 700);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0], head);
        assert_eq!(chunks[1], tail);
    }

    #[test]
    fn extends_past_the_limit_to_the_next_sentence_end() {
        let head = format!("{}。", chars(800, '甲'));
        let tail = chars(100, '乙');
        let text = format!("{head}{tail}");
        let chunks = split_chunks(&text, 700);
        assert_eq!(chunks, vec![head, tail]);
    }

    #[test]
    fn merges_a_short_tail_into_the_previous_chunk() {
        let head = chars(500, '甲');
        let tail = chars(80, '乙');
        let chunks = merge_short_chunks(vec![head.clone(), tail.clone()], MIN_INPUT_CHARS);
        assert_eq!(chunks, vec![format!("{head}\n{tail}")]);
    }

    #[test]
    fn does_not_hard_cut_a_sentence_without_punctuation() {
        let text = chars(1500, '甲');
        assert_eq!(split_chunks(&text, 700), vec![text]);
    }

    #[test]
    fn empty_text_has_no_chunks() {
        assert!(split_chunks("", 700).is_empty());
        assert!(split_chunks("   \n  ", 700).is_empty());
    }
}
