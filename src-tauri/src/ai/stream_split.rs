//! Strip host-only tool-transcript lines that a model may echo back into
//! its visible reply.
//!
//! The host used to fold prior tool calls into assistant history as plain
//! text lines (`[已调用工具 ...]`, `[阶段: ...]`). Structured timeline
//! replay (see [`crate::ai::block_timeline`]) stops feeding those lines to
//! the model, but this module is the defense-in-depth belt: it cleans any
//! residual echo out of live stream deltas and out of legacy persisted
//! rows whose visible text already contains the leaked lines.

/// Marker prefixes for host tool-transcript lines. A line whose trimmed
/// start matches one of these is dropped from user-visible text.
pub const HOST_TOOL_LOG_MARKERS: &[&str] = &["[已调用工具", "[阶段:"];

/// Remove any whole line that is a host tool-transcript marker line.
/// Line-oriented and idempotent; preserves all non-marker content and
/// the original line breaks between surviving lines.
pub fn strip_leaked_host_tool_log(text: &str) -> String {
    if !text.contains('[') {
        return text.to_string();
    }
    let mut kept: Vec<&str> = Vec::new();
    let mut any_dropped = false;
    for line in text.split('\n') {
        if is_marker_line(line) {
            any_dropped = true;
            continue;
        }
        kept.push(line);
    }
    if !any_dropped {
        return text.to_string();
    }
    // Trim leading/trailing blank lines left behind by dropped markers so
    // the reply doesn't end with a run of empty lines.
    while kept.first().map(|l| l.trim().is_empty()).unwrap_or(false) {
        kept.remove(0);
    }
    while kept.last().map(|l| l.trim().is_empty()).unwrap_or(false) {
        kept.pop();
    }
    kept.join("\n")
}

fn is_marker_line(line: &str) -> bool {
    let trimmed = line.trim_start();
    HOST_TOOL_LOG_MARKERS
        .iter()
        .any(|m| trimmed.starts_with(m))
}

/// Would `partial` (a trailing line fragment with no newline yet) still be
/// able to become a marker line? True when it's a confirmed marker start
/// or a strict prefix of one — in either case we hold it back until the
/// line completes rather than emit a half-marker to the UI.
fn could_be_marker_prefix(partial: &str) -> bool {
    let trimmed = partial.trim_start();
    if trimmed.is_empty() {
        return false;
    }
    HOST_TOOL_LOG_MARKERS.iter().any(|m| {
        // confirmed marker-in-progress, or an ambiguous strict prefix.
        trimmed.starts_with(m) || (m.starts_with(trimmed) && trimmed.len() < m.len())
    })
}

/// Stateful, streaming-safe cleaner. Feed it text deltas; it emits cleaned
/// text and holds back only a trailing fragment that might still turn into
/// a marker line (so normal prose streams through unimpeded).
#[derive(Default)]
pub struct StreamContentSplitter {
    pending: String,
}

impl StreamContentSplitter {
    /// Consume a text chunk, returning the portion safe to show now.
    pub fn push(&mut self, chunk: &str) -> String {
        if chunk.is_empty() {
            return String::new();
        }
        self.pending.push_str(chunk);
        let mut out = String::new();

        loop {
            let Some(nl) = self.pending.find('\n') else {
                break;
            };
            let line: String = self.pending[..nl].to_string();
            // drain the line plus its trailing newline
            self.pending.drain(..nl + 1);
            if is_marker_line(&line) {
                continue;
            }
            out.push_str(&line);
            out.push('\n');
        }

        // No newline left: `pending` is a partial last line. Emit it now
        // unless it could still become a marker line.
        if !self.pending.is_empty() && !could_be_marker_prefix(&self.pending) {
            out.push_str(&self.pending);
            self.pending.clear();
        }

        out
    }

    /// Flush the remaining buffered fragment at stream end. Drops it if it
    /// resolved into a marker line; otherwise returns it verbatim.
    #[allow(dead_code)]
    pub fn flush(&mut self) -> String {
        if self.pending.is_empty() {
            return String::new();
        }
        let rest = std::mem::take(&mut self.pending);
        if is_marker_line(&rest) {
            String::new()
        } else {
            rest
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_marker_lines() {
        let raw = "第三章正文。\n[已调用工具 Edit: {\"path\":\"a.md\"}]\n[阶段: general-purpose]";
        assert_eq!(strip_leaked_host_tool_log(raw), "第三章正文。");
    }

    #[test]
    fn keeps_plain_text_untouched() {
        let raw = "普通正文，没有标记。";
        assert_eq!(strip_leaked_host_tool_log(raw), raw);
    }

    #[test]
    fn keeps_non_marker_bracket_lines() {
        let raw = "他说：[停!]\n继续写。";
        assert_eq!(strip_leaked_host_tool_log(raw), raw);
    }

    #[test]
    fn splitter_drops_marker_line_across_chunks() {
        let mut s = StreamContentSplitter::default();
        let mut out = String::new();
        out.push_str(&s.push("正文一段。\n[已调用工"));
        out.push_str(&s.push("具 Edit: {}]\n后续正文"));
        out.push_str(&s.flush());
        assert_eq!(out, "正文一段。\n后续正文");
    }

    #[test]
    fn splitter_streams_prose_without_newline() {
        let mut s = StreamContentSplitter::default();
        // prose that does not start a marker should emit immediately
        assert_eq!(s.push("林凡冷笑了一声，"), "林凡冷笑了一声，");
        assert_eq!(s.push("转身离开。"), "转身离开。");
    }

    #[test]
    fn splitter_holds_ambiguous_prefix_then_releases() {
        let mut s = StreamContentSplitter::default();
        // "[阶" is a strict prefix of "[阶段:" → held back
        assert_eq!(s.push("结尾。\n[阶"), "结尾。\n");
        // resolves to non-marker → released on flush
        assert_eq!(s.flush(), "[阶");
    }
}
