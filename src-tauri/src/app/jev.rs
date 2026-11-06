//! jevnovel sentence scoring for the rewrite3 model.
//!
//! rewrite3 expects every sentence of the source to end with a `【0.04】`
//! style mark (`【NA】` when there is no score). The marks come from jevnovel,
//! a custom PyTorch model that only runs through its own `infer.py`, so the
//! scores are produced by a short Python run over the whole batch.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use serde::Deserialize;
use tokio::process::Command;

use crate::ai::agent::core::context::AbortSignal;
use crate::ai::rewrite_config::RewriteSettings;
use crate::error::{AppError, AppResult};

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

const SCRIPT: &str = include_str!("jev_score.py");

/// How much of the Python error output goes into the message shown to the user.
const ERR_TAIL_CHARS: usize = 800;

/// Text split into sentences, so scores can be put back after each one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parsed {
    lines: Vec<Line>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Line {
    /// Leading whitespace, or the whole line when it is blank.
    indent: String,
    sentences: Vec<String>,
}

impl Parsed {
    pub fn parse(text: &str) -> Self {
        let lines = text
            .replace("\r\n", "\n")
            .replace('\r', "\n")
            .split('\n')
            .map(|line| {
                let body = line.trim_start();
                let indent = line[..line.len() - body.len()].to_string();
                Line {
                    indent,
                    sentences: split_sentences(body),
                }
            })
            .collect();
        Self { lines }
    }

    pub fn sentences(&self) -> impl Iterator<Item = &str> {
        self.lines
            .iter()
            .flat_map(|line| line.sentences.iter().map(String::as_str))
    }

    /// Text with a score mark after each sentence. `scores` is consumed in
    /// sentence order; a missing score becomes `【NA】`.
    pub fn render(&self, scores: &mut dyn Iterator<Item = Option<f64>>) -> String {
        let mut out = String::new();
        for (index, line) in self.lines.iter().enumerate() {
            if index > 0 {
                out.push('\n');
            }
            out.push_str(&line.indent);
            for sentence in &line.sentences {
                out.push_str(sentence);
                out.push_str(&mark(scores.next().flatten()));
            }
        }
        out
    }
}

fn mark(score: Option<f64>) -> String {
    match score {
        Some(p) if p.is_finite() => format!("【{:.2}】", p.clamp(0.0, 1.0)),
        _ => "【NA】".to_string(),
    }
}

fn is_sentence_end(ch: char) -> bool {
    matches!(ch, '。' | '！' | '？' | '!' | '?')
}

/// Closing quotes and brackets that stay with the sentence they follow.
fn is_closer(ch: char) -> bool {
    matches!(
        ch,
        '”' | '’' | '"' | '\'' | '」' | '』' | '）' | ')' | '】' | '》' | '〕'
    )
}

/// Cuts after `。！？!?` plus any closing quotes that follow it. Text that
/// never ends a sentence is one sentence.
fn split_sentences(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut buf = String::new();
    let mut ended = false;
    for ch in text.chars() {
        if ended && !is_sentence_end(ch) && !is_closer(ch) {
            push_sentence(&mut out, &mut buf);
            ended = false;
        }
        buf.push(ch);
        if is_sentence_end(ch) {
            ended = true;
        }
    }
    push_sentence(&mut out, &mut buf);
    out
}

fn push_sentence(out: &mut Vec<String>, buf: &mut String) {
    let sentence = buf.trim();
    if !sentence.is_empty() {
        out.push(sentence.to_string());
    }
    buf.clear();
}

#[derive(Deserialize)]
struct ScoreRow {
    id: usize,
    p_bad: Option<f64>,
    error: Option<String>,
}

/// Removes the scratch directory when the run ends, however it ends.
struct Scratch(PathBuf);

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn check_settings(cfg: &RewriteSettings) -> AppResult<()> {
    if cfg.jev_python.is_empty() {
        return Err(AppError::Other(
            "已开启 jevnovel 打分，但没有填写 Python 路径。请到设置里填写，或关闭 jevnovel 打分。".into(),
        ));
    }
    if cfg.jev_dir.is_empty() {
        return Err(AppError::Other(
            "已开启 jevnovel 打分，但没有填写 jevnovel 模型目录。请到设置里填写，或关闭 jevnovel 打分。".into(),
        ));
    }
    if !Path::new(&cfg.jev_dir).join("infer.py").is_file() {
        return Err(AppError::Other(format!(
            "jevnovel 目录里找不到 infer.py：{}",
            cfg.jev_dir
        )));
    }
    Ok(())
}

/// Scores each sentence. `Ok(None)` means the run was cancelled. A sentence
/// jevnovel cannot score (for example it is over 512 tokens) comes back as
/// `None`; any problem with the run as a whole is an error.
pub async fn score_sentences(
    cfg: &RewriteSettings,
    sentences: &[String],
    signal: &AbortSignal,
) -> AppResult<Option<Vec<Option<f64>>>> {
    if sentences.is_empty() {
        return Ok(Some(Vec::new()));
    }
    check_settings(cfg)?;

    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    let dir = std::env::temp_dir().join(format!("moyan-jev-{}-{nanos}", std::process::id()));
    fs::create_dir_all(&dir)
        .map_err(|err| AppError::Other(format!("无法创建临时目录 {}：{err}", dir.display())))?;
    let _scratch = Scratch(dir.clone());

    let script = dir.join("jev_score.py");
    let input = dir.join("in.jsonl");
    let output = dir.join("out.jsonl");
    let err_log = dir.join("err.log");

    let mut lines = String::new();
    for (id, sentence) in sentences.iter().enumerate() {
        lines.push_str(&serde_json::json!({ "id": id, "sentence": sentence }).to_string());
        lines.push('\n');
    }
    fs::write(&script, SCRIPT).map_err(io_err(&script))?;
    fs::write(&input, lines).map_err(io_err(&input))?;
    let stderr = fs::File::create(&err_log).map_err(io_err(&err_log))?;

    let mut command = Command::new(&cfg.jev_python);
    command
        .arg(&script)
        .arg(&cfg.jev_dir)
        .arg(&cfg.jev_device)
        .arg(&input)
        .arg(&output)
        .env("PYTHONUTF8", "1")
        .env("PYTHONIOENCODING", "utf-8")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::from(stderr))
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);

    let mut child = command.spawn().map_err(|err| {
        AppError::Other(format!(
            "无法启动 jevnovel 的 Python（{}）：{err}",
            cfg.jev_python
        ))
    })?;
    let status = tokio::select! {
        _ = signal.wait_aborted() => {
            let _ = child.kill().await;
            return Ok(None);
        }
        status = child.wait() => status
            .map_err(|err| AppError::Other(format!("等待 jevnovel 失败：{err}")))?,
    };
    if !status.success() {
        let log = fs::read_to_string(&err_log).unwrap_or_default();
        return Err(AppError::Other(format!(
            "jevnovel 打分失败（{status}）：{}",
            tail_chars(log.trim(), ERR_TAIL_CHARS)
        )));
    }

    let raw = fs::read_to_string(&output).map_err(io_err(&output))?;
    let mut scores = vec![None; sentences.len()];
    for line in raw.lines().filter(|line| !line.trim().is_empty()) {
        let row: ScoreRow = serde_json::from_str(line)
            .map_err(|err| AppError::Other(format!("jevnovel 输出无法解析：{err}")))?;
        if let (Some(slot), None) = (scores.get_mut(row.id), row.error) {
            *slot = row.p_bad;
        }
    }
    Ok(Some(scores))
}

fn io_err(path: &Path) -> impl Fn(std::io::Error) -> AppError + '_ {
    move |err| AppError::Other(format!("无法读写 {}：{err}", path.display()))
}

fn tail_chars(text: &str, max: usize) -> String {
    let count = text.chars().count();
    if count <= max {
        return text.to_string();
    }
    let skip = count - max;
    format!("…{}", text.chars().skip(skip).collect::<String>())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render(text: &str, scores: &[Option<f64>]) -> String {
        Parsed::parse(text).render(&mut scores.iter().copied())
    }

    #[test]
    fn splits_on_sentence_ends() {
        let parsed = Parsed::parse("他走了。她笑了！谁在那里？");
        let got: Vec<&str> = parsed.sentences().collect();
        assert_eq!(got, vec!["他走了。", "她笑了！", "谁在那里？"]);
    }

    #[test]
    fn keeps_closing_quotes_with_their_sentence() {
        let parsed = Parsed::parse("“你好。”他说。");
        let got: Vec<&str> = parsed.sentences().collect();
        assert_eq!(got, vec!["“你好。”", "他说。"]);
    }

    #[test]
    fn text_without_an_end_is_one_sentence() {
        let parsed = Parsed::parse("没有句号的一段");
        let got: Vec<&str> = parsed.sentences().collect();
        assert_eq!(got, vec!["没有句号的一段"]);
    }

    #[test]
    fn marks_each_sentence_and_keeps_paragraphs() {
        let out = render(
            "他走了。她笑了。\n\n　　第二段。",
            &[Some(0.041), Some(0.9), None],
        );
        assert_eq!(out, "他走了。【0.04】她笑了。【0.90】\n\n　　第二段。【NA】");
    }

    #[test]
    fn missing_scores_become_na() {
        assert_eq!(render("甲。乙。", &[Some(0.5)]), "甲。【0.50】乙。【NA】");
    }

    #[test]
    fn sentence_count_matches_scores_needed() {
        let parsed = Parsed::parse("甲。乙。\n丙。");
        assert_eq!(parsed.sentences().count(), 3);
    }
}
