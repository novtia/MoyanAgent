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
use crate::data::paths;
use crate::error::AppResult;

const MAX_INCLUDE_DEPTH: u8 = 5;
/// Total files one `discover()` may pull in through `@` directives. Depth alone
/// does not bound the fan-out: five levels of ten includes each is 100k files.
const MAX_INCLUDE_FILES: usize = 64;
const MAX_INCLUDE_BYTES: usize = 256 * 1024;
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
        // Deliberately NO `std::env::current_dir()` here: the host process
        // directory (e.g. the app's own dev checkout) must never be scanned
        // for CLAUDE.md. Project paths come exclusively from the database;
        // an empty cwd disables the project-memory walk entirely.
        let cwd = PathBuf::new();
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
    /// Behind a lock because `cwd` is not known at startup: project paths live
    /// in the database and only arrive when a session starts generating.
    config: Mutex<UserContextConfig>,
    cache: Mutex<Option<UserContext>>,
}

impl FsUserContextLoader {
    pub fn new(config: UserContextConfig) -> Self {
        Self {
            config: Mutex::new(config),
            cache: Mutex::new(None),
        }
    }

    /// Point the project-memory walk at `cwd`, invalidating the cache when it
    /// differs from the directory the cached context was built for.
    ///
    /// Without this the configured `cwd` stays empty forever (see
    /// [`UserContextConfig::from_env`]) and project memory — `CLAUDE.md`,
    /// `.claude/rules/*.md`, `CLAUDE.local.md` — is silently never loaded, no
    /// matter what the user puts in their repository. `None` restores the
    /// no-project state so a plain chat does not inherit the last project's
    /// memory.
    pub fn set_project_cwd(&self, cwd: Option<&Path>) {
        let next = cwd.map(Path::to_path_buf).unwrap_or_default();
        let mut config = lock(&self.config);
        if config.cwd == next {
            return;
        }
        config.cwd = next;
        drop(config);
        // Discovery walks up from `cwd`, so a different project means a
        // different set of memory files.
        *lock(&self.cache) = None;
    }

    fn config(&self) -> UserContextConfig {
        lock(&self.config).clone()
    }

    /// Equivalent to `getMemoryFiles()` minus the conditional-only paths
    /// — those are kept on the resulting [`MemoryFile::path_globs`] for
    /// the nested-memory attachment stage to filter later.
    pub fn discover(&self) -> AppResult<Vec<MemoryFile>> {
        let config = self.config();
        if config.disable_claude_mds {
            return Ok(Vec::new());
        }

        let mut visited: HashSet<PathBuf> = HashSet::new();
        let mut out: Vec<MemoryFile> = Vec::new();

        // User memory (lowest priority of the kept layers).
        if let Some(home) = &config.home {
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
        // An empty cwd means "no project context" — skip the walk
        // entirely instead of resolving relative paths against the host
        // process directory.
        let mut project_chain = if config.cwd.as_os_str().is_empty() {
            Vec::new()
        } else {
            ancestor_paths(&config.cwd)
        };
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
            expand_includes(&mf, 0, &mut visited, &mut buffer, &config)?;
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
        if let Some(cached) = lock(&self.cache).clone() {
            return Ok(cached);
        }
        let files = self.discover()?;
        let rendered = Self::render(&files);
        let ctx = UserContext {
            memory_files: files,
            rendered,
        };
        *lock(&self.cache) = Some(ctx.clone());
        Ok(ctx)
    }

    fn invalidate(&self) {
        *lock(&self.cache) = None;
    }
}

// ─── helpers ────────────────────────────────────────────────────────────────

/// A poisoned lock here only means some other thread panicked mid-read; the
/// data itself is a plain config/cache and stays usable.
fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

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
/// An include may only reach the directory of the memory file that declares it,
/// the user memory root (`~/.claude`), or the project tree. Anything else is
/// quietly skipped: memory files travel with a project, so an unbounded `@`
/// would let a cloned or shared repository paste `~/.ssh/id_rsa` — or any file
/// it can name — straight into the system prompt of every later turn.
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
        // Canonicalize first, then bound: a symlink placed inside an allowed
        // directory must be judged by where it actually points.
        let Ok(canon) = target.canonicalize() else { continue };
        if !include_is_allowed(&canon, parent, config) {
            continue;
        }
        if !canon.is_file() {
            continue;
        }
        if visited.len() >= MAX_INCLUDE_FILES {
            return Ok(());
        }
        if !visited.insert(canon.clone()) {
            continue;
        }
        // An oversized include would crowd out the conversation itself; the
        // whole memory layer is meant to be a handful of small rule files.
        match canon.metadata() {
            Ok(meta) if meta.len() > MAX_INCLUDE_BYTES as u64 => continue,
            Ok(_) => {}
            Err(_) => continue,
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

/// The directories an `@include` may read from.
fn include_is_allowed(canon: &Path, base: &Path, config: &UserContextConfig) -> bool {
    let mut roots: Vec<PathBuf> = Vec::new();
    // The declaring file's own directory. Memory files are discovered along the
    // whole ancestor chain of the project, so this is what makes a plain
    // `@rules/style.md` next to an ancestor `CLAUDE.md` work.
    if !base.as_os_str().is_empty() {
        roots.push(base.to_path_buf());
    }
    // User memory root only — not all of `$HOME`, which would put the user's
    // entire documents and key material in reach.
    if let Some(home) = &config.home {
        roots.push(home.join(".claude"));
    }
    if !config.cwd.as_os_str().is_empty() {
        roots.push(config.cwd.clone());
    }
    roots.iter().any(|root| paths::is_within(root, canon))
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

#[cfg(test)]
mod project_cwd_tests {
    use super::*;

    fn project(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("moyan-cwd-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("CLAUDE.md"), "PROJECT MEMORY MARKER").unwrap();
        dir
    }

    fn loader() -> FsUserContextLoader {
        FsUserContextLoader::new(UserContextConfig {
            cwd: PathBuf::new(),
            home: None,
            disable_claude_mds: false,
        })
    }

    /// The bug: `cwd` starts empty by design, so until someone hands the loader
    /// the session's project path, a repository's `CLAUDE.md` is never read.
    #[test]
    fn project_memory_loads_only_once_the_cwd_is_known() {
        let dir = project("loads");
        let loader = loader();

        assert!(!loader.load().unwrap().rendered.contains("PROJECT MEMORY MARKER"));

        loader.set_project_cwd(Some(&dir));
        assert!(
            loader.load().unwrap().rendered.contains("PROJECT MEMORY MARKER"),
            "project memory must be discovered after the cwd is set"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A plain chat must not inherit whatever project ran before it.
    #[test]
    fn clearing_the_cwd_drops_project_memory_again() {
        let dir = project("cleared");
        let loader = loader();

        loader.set_project_cwd(Some(&dir));
        assert!(loader.load().unwrap().rendered.contains("PROJECT MEMORY MARKER"));

        loader.set_project_cwd(None);
        assert!(!loader.load().unwrap().rendered.contains("PROJECT MEMORY MARKER"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Re-setting the same path must not throw away a warm cache.
    #[test]
    fn setting_the_same_cwd_keeps_the_cache() {
        let dir = project("same");
        let loader = loader();

        loader.set_project_cwd(Some(&dir));
        let first = loader.load().unwrap();

        // Edit on disk: only a genuine invalidation would pick this up.
        std::fs::write(dir.join("CLAUDE.md"), "CHANGED ON DISK").unwrap();
        loader.set_project_cwd(Some(&dir));
        let second = loader.load().unwrap();

        assert_eq!(first.rendered, second.rendered);
        assert!(!second.rendered.contains("CHANGED ON DISK"));

        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod include_boundary_tests {
    use super::*;

    struct Fixture {
        root: PathBuf,
        config: UserContextConfig,
    }

    fn fixture(name: &str) -> Fixture {
        let root = std::env::temp_dir().join(format!("moyan-include-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let project = root.join("project");
        let home = root.join("home");
        std::fs::create_dir_all(project.join("rules")).unwrap();
        std::fs::create_dir_all(home.join(".claude")).unwrap();
        std::fs::create_dir_all(root.join("secrets")).unwrap();
        std::fs::write(root.join("secrets").join("id_rsa"), "PRIVATE KEY").unwrap();
        std::fs::write(project.join("rules").join("style.md"), "style rules").unwrap();
        std::fs::write(home.join(".claude").join("shared.md"), "shared rules").unwrap();

        Fixture {
            config: UserContextConfig {
                cwd: project,
                home: Some(home),
                disable_claude_mds: false,
            },
            root,
        }
    }

    /// Everything a memory file legitimately needs stays reachable.
    #[test]
    fn includes_within_the_project_and_user_memory_are_allowed() {
        let f = fixture("allowed");
        let base = f.config.cwd.clone();

        for token in ["./rules/style.md", "rules/style.md"] {
            let target = resolve_include_path(token, &base, &f.config)
                .and_then(|p| p.canonicalize().ok())
                .expect("resolves");
            assert!(
                include_is_allowed(&target, &base, &f.config),
                "`{token}` is inside the project"
            );
        }

        let shared = resolve_include_path("~/.claude/shared.md", &base, &f.config)
            .and_then(|p| p.canonicalize().ok())
            .expect("resolves");
        assert!(include_is_allowed(&shared, &base, &f.config));

        let _ = std::fs::remove_dir_all(&f.root);
    }

    /// A memory file travels with the project it lives in, so an unbounded `@`
    /// is an exfiltration primitive: whatever it names ends up in the prompt.
    #[test]
    fn includes_outside_every_allowed_root_are_refused() {
        let f = fixture("refused");
        let base = f.config.cwd.clone();
        let secret = f.root.join("secrets").join("id_rsa");

        let absolute = resolve_include_path(secret.to_str().unwrap(), &base, &f.config)
            .and_then(|p| p.canonicalize().ok())
            .expect("resolves");
        assert!(
            !include_is_allowed(&absolute, &base, &f.config),
            "an absolute path outside the project must be refused"
        );

        let traversal = resolve_include_path("../secrets/id_rsa", &base, &f.config)
            .and_then(|p| p.canonicalize().ok())
            .expect("resolves");
        assert!(
            !include_is_allowed(&traversal, &base, &f.config),
            "`..` must not climb out of the project"
        );

        // `$HOME` at large is not a memory root either — only `~/.claude`.
        let home_file = f.config.home.as_ref().unwrap().join("taxes.md");
        std::fs::write(&home_file, "private").unwrap();
        let home_file = home_file.canonicalize().unwrap();
        assert!(
            !include_is_allowed(&home_file, &base, &f.config),
            "only the user memory directory is in scope, not all of $HOME"
        );

        let _ = std::fs::remove_dir_all(&f.root);
    }

    /// A symlink is judged by its target, not by where the link sits.
    #[cfg(unix)]
    #[test]
    fn a_symlink_pointing_out_of_the_project_is_refused() {
        let f = fixture("symlink");
        let base = f.config.cwd.clone();
        let link = base.join("escape.md");
        std::os::unix::fs::symlink(f.root.join("secrets").join("id_rsa"), &link).unwrap();

        let resolved = resolve_include_path("./escape.md", &base, &f.config)
            .and_then(|p| p.canonicalize().ok())
            .expect("resolves through the link");
        assert!(!include_is_allowed(&resolved, &base, &f.config));

        let _ = std::fs::remove_dir_all(&f.root);
    }

    /// End-to-end: the refused include must not appear in the loaded memory.
    #[test]
    fn expansion_skips_a_refused_include_but_keeps_a_valid_one() {
        let f = fixture("expand");
        let secret = f.root.join("secrets").join("id_rsa");
        let declaring = f.config.cwd.join("CLAUDE.md");
        std::fs::write(
            &declaring,
            format!("@rules/style.md\n@{}\n", secret.display()),
        )
        .unwrap();

        let mf = MemoryFile {
            ty: MemoryType::Project,
            path: declaring.canonicalize().unwrap(),
            content: std::fs::read_to_string(&declaring).unwrap(),
            conditional: false,
            path_globs: None,
        };
        let mut visited = HashSet::new();
        let mut out = Vec::new();
        expand_includes(&mf, 0, &mut visited, &mut out, &f.config).unwrap();

        assert_eq!(out.len(), 1, "only the in-project include is expanded");
        assert_eq!(out[0].content.trim(), "style rules");
        assert!(
            !out.iter().any(|m| m.content.contains("PRIVATE KEY")),
            "the out-of-project file must never reach the prompt"
        );

        let _ = std::fs::remove_dir_all(&f.root);
    }
}
