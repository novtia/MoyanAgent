/**
 * Unified `@file` mention "reference card" module.
 *
 * - {@link core} — path/icon/extension logic, project-scope validation, and the
 *   contenteditable chip DOM + serialization helpers (the single source).
 * - {@link MentionChip} — static React reference card.
 * - {@link MentionText} — renders message text with inline reference cards.
 * - {@link MentionEditor} — controlled contenteditable editor.
 * - {@link ComposerEditor} — the editor bound to the session store.
 */
export {
  MENTION_PREFIX,
  MENTION_RE,
  MENTION_ICON_INNER,
  normalizeMentionPath,
  mentionBasename,
  looksLikeDir,
  mentionIconKind,
  mentionIconSvg,
  parseMentionPaths,
  isWithinProject,
  createMentionNode,
  serializeMentions,
  collectMentions,
  buildMentionNodes,
  moveCaretToEnd,
  type MentionIconKind,
} from "./core";
export { MentionChip } from "./MentionChip";
export { MentionText } from "./MentionText";
export {
  MentionEditor,
  type MentionEditorHandle,
  type MentionEditorProps,
} from "./MentionEditor";
export {
  ComposerEditor,
  type ComposerEditorHandle,
  type ComposerEditorProps,
} from "./ComposerEditor";
