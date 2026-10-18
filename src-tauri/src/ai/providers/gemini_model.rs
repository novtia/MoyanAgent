//! Gemini model-id capability rules, keyed off the parsed version number
//! rather than hard-coded family names, so new releases (3.8, 4, ...) work
//! without touching call sites.
//!
//! Mirrored on the frontend in `src/utils/geminiModel.ts`; keep both in sync.

/// First version that ships thinking (`thinkingBudget`).
const THINKING_MIN: (u32, u32) = (2, 5);
/// First version that switched to discrete `thinkingLevel` and rejects
/// `thinkingBudget` outright.
const THINKING_LEVEL_MIN: (u32, u32) = (3, 0);

/// Parse `(major, minor)` out of ids such as `gemini-2.5-pro`,
/// `gemini-3-flash-preview`, `gemini-3.8-flash`, `gemini-4-argon`,
/// `google/gemini-3.1-pro` or `publishers/google/models/gemini-4-argon`.
/// A missing minor is `0`. Returns `None` for ids without a `gemini-<n>` stem.
pub(crate) fn gemini_version(model: &str) -> Option<(u32, u32)> {
    let id = model.to_ascii_lowercase();
    let rest = &id[id.find("gemini-")? + "gemini-".len()..];

    let (major, after_major) = take_number(rest)?;
    let minor = after_major
        .strip_prefix('.')
        .and_then(take_number)
        .map(|(n, _)| n)
        .unwrap_or(0);
    Some((major, minor))
}

fn take_number(s: &str) -> Option<(u32, &str)> {
    let end = s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
    if end == 0 {
        return None;
    }
    s[..end].parse().ok().map(|n| (n, &s[end..]))
}

/// Image / video generators never take a `thinkingConfig`.
pub(crate) fn is_media_model(model: &str) -> bool {
    let m = model.to_ascii_lowercase();
    m.contains("image") || m.contains("imagen") || m.contains("veo")
}

/// The model reasons by version alone (2.5 and every later release).
pub(crate) fn implies_reasoning(model: &str) -> bool {
    !is_media_model(model) && gemini_version(model).is_some_and(|v| v >= THINKING_MIN)
}

/// Whether to ask for thought summaries (`includeThoughts`) even when the user
/// has not picked an effort. Also covers legacy `*-thinking` experiments.
pub(crate) fn supports_thought_summaries(model: &str) -> bool {
    if is_media_model(model) {
        return false;
    }
    implies_reasoning(model) || model.to_ascii_lowercase().contains("thinking")
}

/// `thinkingLevel` (Gemini 3+) vs. `thinkingBudget` (2.5 and unversioned
/// thinking models).
pub(crate) fn uses_thinking_level(model: &str) -> bool {
    gemini_version(model).is_some_and(|v| v >= THINKING_LEVEL_MIN)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_versions() {
        assert_eq!(gemini_version("gemini-2.5-pro"), Some((2, 5)));
        assert_eq!(gemini_version("gemini-3-flash-preview"), Some((3, 0)));
        assert_eq!(gemini_version("gemini-3.8-flash"), Some((3, 8)));
        assert_eq!(gemini_version("gemini-4-argon"), Some((4, 0)));
        assert_eq!(gemini_version("Google/Gemini-3.1-Pro"), Some((3, 1)));
        assert_eq!(
            gemini_version("publishers/google/models/gemini-4-argon"),
            Some((4, 0))
        );
        assert_eq!(gemini_version("gemini-1.5-flash"), Some((1, 5)));
        assert_eq!(gemini_version("gemini-flash-latest"), None);
        assert_eq!(gemini_version("gpt-5"), None);
    }

    #[test]
    fn thinking_style_follows_version() {
        for id in ["gemini-3-flash-preview", "gemini-3.8-flash", "gemini-4-argon", "gemini-10-x"] {
            assert!(uses_thinking_level(id), "{id}");
            assert!(implies_reasoning(id), "{id}");
        }
        assert!(!uses_thinking_level("gemini-2.5-pro"));
        assert!(implies_reasoning("gemini-2.5-pro"));
        assert!(!implies_reasoning("gemini-2.0-flash"));
        assert!(!implies_reasoning("gemini-1.5-pro"));
    }

    #[test]
    fn media_and_legacy_thinking() {
        assert!(!supports_thought_summaries("gemini-3-pro-image-preview"));
        assert!(!implies_reasoning("gemini-2.5-flash-image"));
        assert!(supports_thought_summaries("gemini-2.0-flash-thinking-exp"));
        assert!(!uses_thinking_level("gemini-2.0-flash-thinking-exp"));
    }
}
