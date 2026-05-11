//! Nested-memory attachment collector.
//!
//! Implements the "Read a file → inject path-scoped CLAUDE.md / rules"
//! flow described in `context-memory-architecture.md` §15.3.
//!
//! Pipeline:
//!
//! 1. Some tool (typically [`FileReadTool`]) records the absolute path of
//!    a freshly-read file into
//!    [`ToolUseContext::nested_memory_attachment_triggers`].
//! 2. After each turn, the runner calls [`collect_nested_memory`] to
//!    drain those triggers and emit one [`Attachment`] per *conditional*
//!    memory file whose `path_globs` match any triggered path.
//! 3. The runner pushes those attachments back into the chat history so
//!    the model sees the rule before the next assistant turn.
//!
//! The matcher is intentionally tiny — `*` matches any single path
//! segment, `**` matches any number of segments. This covers the
//! `.claude/rules/*.md` patterns used in practice without pulling in a
//! `glob` dependency.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::ai::agent::core::attachment::{Attachment, AttachmentKind};
use crate::ai::agent::core::context::ToolUseContext;
use crate::ai::agent::memory::UserContext;

/// Drain triggers from `context` and produce one nested-memory
/// attachment for every conditional [`MemoryFile`] whose
/// `path_globs` match a triggered path that hasn't been surfaced
/// before this session.
pub fn collect_nested_memory(
    context: &Arc<ToolUseContext>,
    user_context: &UserContext,
) -> Vec<Attachment> {
    let triggers: Vec<PathBuf> = {
        let Ok(mut set) = context.nested_memory_attachment_triggers.lock() else {
            return Vec::new();
        };
        let drained: Vec<PathBuf> = set.iter().cloned().collect();
        set.clear();
        drained
    };
    if triggers.is_empty() {
        return Vec::new();
    }

    let loaded: HashSet<PathBuf> = context
        .loaded_nested_memory_paths
        .lock()
        .ok()
        .map(|g| g.iter().cloned().collect())
        .unwrap_or_default();

    let mut out: Vec<Attachment> = Vec::new();
    for mf in &user_context.memory_files {
        if !mf.conditional {
            continue;
        }
        let Some(globs) = &mf.path_globs else { continue };
        if globs.is_empty() {
            continue;
        }
        if loaded.contains(&mf.path) {
            continue;
        }
        let matched = triggers
            .iter()
            .any(|t| globs.iter().any(|g| glob_match(g, t)));
        if !matched {
            continue;
        }
        out.push(Attachment::for_main(AttachmentKind::NestedMemory {
            path: mf.path.clone(),
            content: mf.content.clone(),
        }));
    }

    // Mark these as loaded so we don't re-inject them on the next turn.
    if !out.is_empty() {
        if let Ok(mut g) = context.loaded_nested_memory_paths.lock() {
            for att in &out {
                if let AttachmentKind::NestedMemory { path, .. } = &att.kind {
                    g.insert(path.clone());
                }
            }
        }
    }
    out
}

/// Minimal glob matcher supporting `*` and `**`.
///
/// - `**` matches zero or more path segments.
/// - `*` matches any single path segment.
/// - any other character is matched literally.
///
/// The match is performed against the path's **relative** segments after
/// normalising separators to `/`. Absolute prefixes are tolerated by
/// trying both the canonical absolute string and the file name.
pub fn glob_match(pattern: &str, path: &Path) -> bool {
    let pattern = pattern.trim().trim_start_matches("./");
    let path_str = path.to_string_lossy().replace('\\', "/");
    if glob_match_str(pattern, &path_str) {
        return true;
    }
    // Fall back: try matching against just the trailing portion of the
    // path. This lets a relative glob like `src/**/*.ts` match an
    // absolute path that has the same suffix.
    let segments: Vec<&str> = path_str.split('/').collect();
    for start in 0..segments.len() {
        let suffix = segments[start..].join("/");
        if glob_match_str(pattern, &suffix) {
            return true;
        }
    }
    false
}

fn glob_match_str(pattern: &str, input: &str) -> bool {
    let pat_segs: Vec<&str> = pattern.split('/').collect();
    let in_segs: Vec<&str> = input.split('/').collect();
    glob_segments(&pat_segs, &in_segs)
}

fn glob_segments(pat: &[&str], input: &[&str]) -> bool {
    if pat.is_empty() {
        return input.is_empty();
    }
    let head = pat[0];
    if head == "**" {
        // `**` matches zero or more segments.
        if pat.len() == 1 {
            return true;
        }
        for i in 0..=input.len() {
            if glob_segments(&pat[1..], &input[i..]) {
                return true;
            }
        }
        return false;
    }
    if input.is_empty() {
        return false;
    }
    if !segment_match(head, input[0]) {
        return false;
    }
    glob_segments(&pat[1..], &input[1..])
}

fn segment_match(pattern: &str, segment: &str) -> bool {
    // Within a segment `*` matches any run of non-`/` characters; other
    // characters match literally.
    let mut p = pattern.chars().peekable();
    let mut s = segment.chars().peekable();
    loop {
        match (p.peek(), s.peek()) {
            (None, None) => return true,
            (None, Some(_)) => return false,
            (Some(&'*'), _) => {
                p.next();
                // Greedy: try consuming 0..=remaining
                let rest_pat: String = p.clone().collect();
                let mut tail: String = s.clone().collect();
                loop {
                    if segment_match(&rest_pat, &tail) {
                        return true;
                    }
                    if tail.is_empty() {
                        return false;
                    }
                    tail.remove(0);
                }
            }
            (Some(&pc), Some(&sc)) if pc == sc => {
                p.next();
                s.next();
            }
            _ => return false,
        }
    }
}
