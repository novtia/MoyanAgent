//! Filesystem-backed [`UserContextLoader`].
//!
//! Mirrors the discovery rules from `context-memory-architecture.md`
//! §6–§7 with project-appropriate trimming:
//!
//! - **User memory**: `~/.claude/CLAUDE.md` and `~/.claude/rules/*.md`
//! - **Project memory**: walk from CWD up to the filesystem root,
//!   reading `CLAUDE.md`, `.claude/CLAUDE.md`, `.claude/rules/*.md`.
//! - **Local memory**: walk up looking for `CLAUDE.local.md`.
//! - `@path` includes (recursive, depth ≤ 5, cycle-protected).
//! - YAML frontmatter `paths:` parsed into glob strings; the loader
//!   stores them on [`MemoryFile::path_globs`] and marks the rule as
//!   `conditional` so it isn't injected into base user context.
//!
//! Glob matching against actual Read paths is left to the attachments
//! layer (`nested_memory`) — that wiring is intentionally out of scope
//! for the loader.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::ai::agent::memory::{MemoryFile, MemoryType, UserContext, UserContextLoader};
use crate::error::AppResult;

const MAX_INCLUDE_DEPTH: u8 = 5;
const RULE_EXTENSION: &str = "md";

/// Configuration for [`FsUserContextLoader`].
#[derive(Debug, Clone)]
pub struct UserContextConfig {
    pub cwd: PathBuf,
    /// Override for `$HOME`. `None` ⇒ disable user memory (useful in tests).
    pub home: Option<PathBuf>,
    /// Hard switch matching `CLAUDE_CODE_DISABLE_CLAUDE_MDS`.
    pub disable_claude_mds: bool,
}

impl UserContextConfig {
    pub fn from_env() -> Self {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let home = std::env::var_os("HOME")
            .or_else(|| std::env::var_os("USERPROFILE"))
            .map(PathBuf::from);
        let disable_claude_mds = matches!(
            std::env::var("CLAUDE_CODE_DISABLE_CLAUDE_MDS").as_deref(),
            Ok("1") | Ok("true") | Ok("TRUE")
        );
        Self {
            cwd,
            home,
            disable_claude_mds,
        }
    }
}

/// Filesystem-backed implementation of [`UserContextLoader`].
///
/// The loader is cheap to share via `Arc` and caches the rendered
/// [`UserContext`] in an inner `Mutex`. Call [`Self::invalidate`] from
/// `runPostCompactCleanup()`-equivalent code paths.
pub struct FsUserContextLoader {
    config: UserContextConfig,
    cache: Mutex<Option<UserContext>>,
}

impl FsUserContextLoader {
    pub fn new(config: UserContextConfig) -> Self {
        Self {
            config,
            cache: Mutex::new(None),
        }
    }

    /// Equivalent to `getMemoryFiles()` minus the conditional-only paths
    /// — those are kept on the resulting [`MemoryFile::path_globs`] for
    /// the nested-memory attachment stage to filter later.
    pub fn discover(&self) -> AppResult<Vec<MemoryFile>> {
        if self.config.disable_claude_mds {
            return Ok(Vec::new());
        }

        let mut visited: HashSet<PathBuf> = HashSet::new();
        let mut out: Vec<MemoryFile> = Vec::new();

        // User memory (lowest priority of the kept layers).
        if let Some(home) = &self.config.home {
            let user_root = home.join(".claude");
            push_file_if_exists(
                MemoryType::User,
                &user_root.join("CLAUDE.md"),
                &mut out,
                &mut visited,
            )?;
            push_rules_dir(
                MemoryType::User,
                &user_root.join("rules"),
                &mut out,
                &mut visited,
            )?;
        }

        // Project memory: walk up from CWD to the filesystem root.
        let mut project_chain = ancestor_paths(&self.config.cwd);
        // closest directory wins, but we discover root → leaf so callers
        // see ascending order. Reverse so leaf (most specific) comes last.
        project_chain.reverse();
        for dir in &project_chain {
            push_file_if_exists(
                MemoryType::Project,
                &dir.join("CLAUDE.md"),
                &mut out,
                &mut visited,
            )?;
            push_file_if_exists(
                MemoryType::Project,
                &dir.join(".claude").join("CLAUDE.md"),
                &mut out,
                &mut visited,
            )?;
            push_rules_dir(
                MemoryType::Project,
                &dir.join(".claude").join("rules"),
                &mut out,
                &mut visited,
            )?;
        }

        // Local memory (per-user, per-project).
        for dir in &project_chain {
            push_file_if_exists(
                MemoryType::Local,
                &dir.join("CLAUDE.local.md"),
                &mut out,
                &mut visited,
            )?;
        }

        // Expand `@path` includes for every discovered file.
        let mut expanded: Vec<MemoryFile> = Vec::with_capacity(out.len());
        for mf in out {
            let mut buffer = Vec::new();
            expand_includes(&mf, 0, &mut visited, &mut buffer, &self.config)?;
            expanded.push(mf);
            expanded.append(&mut buffer);
        }

        Ok(expanded)
    }

    /// Render discovered memory files into a single user-context string.
    /// Conditional rules (with `paths:` frontmatter) are excluded from
    /// the base render — they live on the [`MemoryFile`] until a Read
    /// triggers nested-memory injection.
    pub fn render(files: &[MemoryFile]) -> String {
        let mut out = String::new();
        for mf in files {
            if mf.conditional {
                continue;
            }
            out.push_str("<system-reminder>\n");
            out.push_str(&format!(
                "Contents of {} ({}):\n\n",
                mf.path.display(),
                memory_type_label(mf.ty)
            ));
            out.push_str(&mf.content);
            out.push_str("\n</system-reminder>\n\n");
        }
        out
    }
}

impl UserContextLoader for FsUserContextLoader {
    fn load(&self) -> AppResult<UserContext> {
        if let Some(cached) = self.cache.lock().ok().and_then(|g| g.clone()) {
            return Ok(cached);
        }
        let files = self.discover()?;
        let rendered = Self::render(&files);
        let ctx = UserContext {
            memory_files: files,
            rendered,
        };
        if let Ok(mut g) = self.cache.lock() {
            *g = Some(ctx.clone());
        }
        Ok(ctx)
    }

    fn invalidate(&self) {
        if let Ok(mut g) = self.cache.lock() {
            *g = None;
        }
    }
}

// ─── helpers ────────────────────────────────────────────────────────────────

fn memory_type_label(t: MemoryType) -> &'static str {
    match t {
        MemoryType::Managed => "managed",
        MemoryType::User => "user",
        MemoryType::Project => "project",
        MemoryType::Local => "local",
        MemoryType::AutoMem => "auto-memory",
        MemoryType::TeamMem => "team-memory",
    }
}

fn ancestor_paths(start: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut cursor = Some(start.to_path_buf());
    while let Some(p) = cursor {
        out.push(p.clone());
        cursor = p.parent().map(Path::to_path_buf);
    }
    out
}

fn push_file_if_exists(
    ty: MemoryType,
    path: &Path,
    out: &mut Vec<MemoryFile>,
    visited: &mut HashSet<PathBuf>,
) -> AppResult<()> {
    let canonical = match path.canonicalize() {
        Ok(p) => p,
        Err(_) => return Ok(()),
    };
    if !visited.insert(canonical.clone()) {
        return Ok(());
    }
    let content = match std::fs::read_to_string(&canonical) {
        Ok(s) => s,
        Err(_) => return Ok(()),
    };
    let (body, path_globs) = parse_frontmatter(&content);
    out.push(MemoryFile {
        ty,
        path: canonical,
        content: body,
        conditional: path_globs.as_ref().map(|g| !g.is_empty()).unwrap_or(false),
        path_globs,
    });
    Ok(())
}

fn push_rules_dir(
    ty: MemoryType,
    dir: &Path,
    out: &mut Vec<MemoryFile>,
    visited: &mut HashSet<PathBuf>,
) -> AppResult<()> {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return Ok(()),
    };
    let mut files: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.is_file())
        .filter(|p| {
            p.extension()
                .and_then(|s| s.to_str())
                .map(|e| e.eq_ignore_ascii_case(RULE_EXTENSION))
                .unwrap_or(false)
        })
        .collect();
    files.sort();
    for f in files {
        push_file_if_exists(ty, &f, out, visited)?;
    }
    Ok(())
}

/// Strip a YAML frontmatter block and return `(body, parsed paths globs)`.
///
/// Intentionally tiny — we parse just enough of `paths:` to match the
/// shapes used by `.claude/rules/*.md`:
///
/// ```yaml
/// ---
/// paths:
///   - "src/**/*.ts"
///   - "src/utils/**/*.rs"
/// ---
/// ```
fn parse_frontmatter(input: &str) -> (String, Option<Vec<String>>) {
    let trimmed = input.trim_start_matches('\u{FEFF}');
    if !trimmed.starts_with("---") {
        return (input.to_string(), None);
    }
    // Find the end of the frontmatter block.
    let after = &trimmed[3..];
    let after = after.trim_start_matches(['\n', '\r']);
    let Some(end) = find_frontmatter_end(after) else {
        return (input.to_string(), None);
    };
    let yaml = &after[..end];
    let body_start = end + after[end..].find('\n').map(|n| n + 1).unwrap_or(end);
    let body = after[body_start..].to_string();

    let mut globs: Vec<String> = Vec::new();
    let mut in_paths = false;
    for line in yaml.lines() {
        let trimmed_line = line.trim_end();
        if trimmed_line == "paths:" {
            in_paths = true;
            continue;
        }
        if in_paths {
            let Some(rest) = trimmed_line.strip_prefix("  - ") else {
                if !trimmed_line.starts_with("  ") && !trimmed_line.is_empty() {
                    in_paths = false;
                }
                continue;
            };
            let value = rest.trim().trim_matches(|c| c == '"' || c == '\'');
            if !value.is_empty() {
                globs.push(value.to_string());
            }
        }
    }

    let parsed = if globs.is_empty() { None } else { Some(globs) };
    (body, parsed)
}

fn find_frontmatter_end(after: &str) -> Option<usize> {
    let mut start = 0;
    for line in after.split_inclusive('\n') {
        if line.trim_end_matches(['\r', '\n']) == "---" {
            return Some(start);
        }
        start += line.len();
    }
    None
}

/// Recursively resolve `@path` include directives.
///
/// Includes outside the memory file's directory and outside `$HOME` are
/// quietly skipped (matches the conservative behaviour of the TS
/// loader's external-include policy).
fn expand_includes(
    mf: &MemoryFile,
    depth: u8,
    visited: &mut HashSet<PathBuf>,
    out: &mut Vec<MemoryFile>,
    config: &UserContextConfig,
) -> AppResult<()> {
    if depth >= MAX_INCLUDE_DEPTH {
        return Ok(());
    }
    let parent = mf.path.parent().unwrap_or(Path::new(""));
    for line in mf.content.lines() {
        let trimmed = line.trim();
        let Some(rest) = trimmed.strip_prefix('@') else {
            continue;
        };
        if rest.starts_with("type ") || rest.starts_with("ts-") {
            continue; // not an include — looks like an annotation
        }
        let candidate = resolve_include_path(rest.trim(), parent, config);
        let Some(target) = candidate else { continue };
        let Ok(canon) = target.canonicalize() else { continue };
        if !visited.insert(canon.clone()) {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(&canon) else {
            continue;
        };
        let (body, path_globs) = parse_frontmatter(&content);
        let nested = MemoryFile {
            ty: mf.ty,
            path: canon,
            content: body,
            conditional: path_globs.as_ref().map(|g| !g.is_empty()).unwrap_or(false),
            path_globs,
        };
        let mut sub_buffer = Vec::new();
        expand_includes(&nested, depth + 1, visited, &mut sub_buffer, config)?;
        out.push(nested);
        out.append(&mut sub_buffer);
    }
    Ok(())
}

fn resolve_include_path(token: &str, base: &Path, config: &UserContextConfig) -> Option<PathBuf> {
    if let Some(rest) = token.strip_prefix("~/") {
        return config.home.as_ref().map(|h| h.join(rest));
    }
    if let Some(rest) = token.strip_prefix("./") {
        return Some(base.join(rest));
    }
    let p = Path::new(token);
    if p.is_absolute() {
        return Some(p.to_path_buf());
    }
    Some(base.join(token))
}
