//! Split Beileng rewrite3 output into thinking and body.
//!
//! The gateway sends the analysis in `reasoning` / `reasoning_content` /
//! `revnovel.cot`, and the prose in `revnovel.body`. `content` still wraps the
//! same analysis in `<think>...</think>` and then appends the body. Streaming
//! delivers that wrapper as ordinary `delta.content` chunks.

const OPEN: &str = "<think>";
const CLOSE: &str = "</think>";

/// Result of separating one model message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CotSplit {
    pub thinking: String,
    pub body: String,
    /// A `<think>` block or gateway field was present.
    pub detected: bool,
    /// The analysis started and no body followed.
    pub incomplete: bool,
}

/// Streaming filter. `<think>` may be split across chunks.
#[derive(Debug, Default)]
pub struct ThinkStream {
    inside: bool,
    hold: String,
}

impl ThinkStream {
    pub fn push(&mut self, input: &str) -> (String, String) {
        if input.is_empty() {
            return (String::new(), String::new());
        }
        self.hold.push_str(input);
        let mut body = String::new();
        let mut thinking = String::new();
        loop {
            if self.inside {
                if let Some(index) = self.hold.find(CLOSE) {
                    thinking.push_str(&self.hold[..index]);
                    self.hold.drain(..index + CLOSE.len());
                    self.inside = false;
                    continue;
                }
                let keep = partial_tag_suffix(&self.hold, CLOSE);
                let emit_to = self.hold.len() - keep;
                thinking.push_str(&self.hold[..emit_to]);
                self.hold.drain(..emit_to);
                break;
            }
            if let Some(index) = self.hold.find(OPEN) {
                body.push_str(&self.hold[..index]);
                self.hold.drain(..index + OPEN.len());
                self.inside = true;
                continue;
            }
            let keep = partial_tag_suffix(&self.hold, OPEN);
            let emit_to = self.hold.len() - keep;
            body.push_str(&self.hold[..emit_to]);
            self.hold.drain(..emit_to);
            break;
        }
        (body, thinking)
    }
}

/// Prefer the gateway's split fields. Fall back to `<think>` tags in `content`.
pub fn split_gateway_message(
    content: Option<&str>,
    reasoning: Option<&str>,
    reasoning_content: Option<&str>,
    revnovel_cot: Option<&str>,
    revnovel_body: Option<&str>,
) -> CotSplit {
    let cot = first_non_empty([revnovel_cot, reasoning, reasoning_content]);
    if let Some(body) = revnovel_body {
        let body = body.trim();
        let thinking = cot.unwrap_or("").trim().to_string();
        let incomplete = body.is_empty() && !thinking.is_empty();
        let detected = !thinking.is_empty() || content.unwrap_or("").contains(OPEN);
        return CotSplit {
            thinking,
            body: body.to_string(),
            detected,
            incomplete,
        };
    }
    let content = content.unwrap_or("").trim();
    if content.is_empty() && cot.is_none() {
        return CotSplit {
            thinking: String::new(),
            body: String::new(),
            detected: false,
            incomplete: false,
        };
    }
    let (tagged_thinking, tagged_body, closed) = split_think_tags(content);
    let thinking = cot
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or(tagged_thinking);
    let detected = !thinking.is_empty() || content.contains(OPEN);
    if content.contains(OPEN) && !closed {
        return CotSplit {
            thinking,
            body: String::new(),
            detected: true,
            incomplete: true,
        };
    }
    CotSplit {
        thinking,
        body: tagged_body,
        detected,
        incomplete: false,
    }
}

fn first_non_empty<'a>(items: [Option<&'a str>; 3]) -> Option<&'a str> {
    items
        .into_iter()
        .flatten()
        .map(str::trim)
        .find(|s| !s.is_empty())
}

fn split_think_tags(content: &str) -> (String, String, bool) {
    let mut stream = ThinkStream::default();
    let (body, thinking) = stream.push(content);
    let closed = !stream.inside && stream.hold.is_empty();
    (
        thinking.trim().to_string(),
        body.trim().to_string(),
        closed || !content.contains(OPEN),
    )
}

fn partial_tag_suffix(buf: &str, tag: &str) -> usize {
    let bytes = buf.as_bytes();
    let tag_bytes = tag.as_bytes();
    let max = bytes.len().min(tag_bytes.len().saturating_sub(1));
    for size in (1..=max).rev() {
        let start = bytes.len() - size;
        if !buf.is_char_boundary(start) {
            continue;
        }
        if tag_bytes.starts_with(&bytes[start..]) {
            return size;
        }
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefers_gateway_body_and_cot() {
        let split = split_gateway_message(
            Some("<think>\n分析\n</think>\n\n正文"),
            Some("分析"),
            Some("分析"),
            Some("分析"),
            Some("正文"),
        );
        assert!(split.detected);
        assert!(!split.incomplete);
        assert_eq!(split.thinking, "分析");
        assert_eq!(split.body, "正文");
    }

    #[test]
    fn empty_gateway_body_is_incomplete() {
        let split = split_gateway_message(Some("<think>\n分析"), Some("分析"), None, Some("分析"), Some(""));
        assert!(split.incomplete);
        assert!(split.body.is_empty());
    }

    #[test]
    fn strips_think_tags_when_the_gateway_omits_revnovel() {
        let split = split_gateway_message(
            Some("<think>\n先看第一句。\n</think>\n\n午后三点，林夏打开窗子。"),
            None,
            None,
            None,
            None,
        );
        assert_eq!(split.thinking, "先看第一句。");
        assert_eq!(split.body, "午后三点，林夏打开窗子。");
        assert!(!split.incomplete);
    }

    #[test]
    fn unclosed_think_tag_is_incomplete() {
        let split = split_gateway_message(Some("<think>\n还在分析"), None, None, None, None);
        assert!(split.incomplete);
        assert!(split.body.is_empty());
    }

    #[test]
    fn plain_prose_is_unchanged() {
        let split = split_gateway_message(Some("午后三点，林夏打开窗子。"), None, None, None, None);
        assert!(!split.detected);
        assert_eq!(split.body, "午后三点，林夏打开窗子。");
    }

    #[test]
    fn stream_reassembles_a_split_tag() {
        let mut stream = ThinkStream::default();
        let (body, thinking) = stream.push("<thi");
        assert!(body.is_empty() && thinking.is_empty());
        let (body, thinking) = stream.push("nk>\n分析</think>\n\n正文");
        assert_eq!(thinking.trim(), "分析");
        assert_eq!(body.trim(), "正文");
    }
}
