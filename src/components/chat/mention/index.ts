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
  normalizeMentionPath,
  mentionBasename,
  looksLikeDir,
  mentionFileIconName,
  mediaMentionKind,
  mediaMentionIndex,
  mediaMentionLabel,
  mediaMentionDisplayLabel,
  mediaMentionKindFromMime,
  serializeMentionPath,
  serializeMentionRangeSuffix,
  formatMentionParagraphRange,
  mentionDisplayLabel,
  normalizeMentionRange,
  parseMentionAt,
  parseMentionSegments,
  parseMentionPaths,
  isWithinProject,
  createMentionNode,
  serializeMentions,
  collectMentions,
  buildMentionNodes,
  moveCaretToEnd,
  type MentionMediaRenderData,
  type MediaMentionKind,
  type MentionSegment,
  type MentionRange,
  type ParsedMention,
} from "./core";
export { MentionIcon } from "./MentionIcon";
export { MentionChip } from "./MentionChip";
export { RoleCiteChip } from "./RoleCiteChip";
export { MentionText } from "./MentionText";
export {
  ROLE_CITE_PREFIX,
  serializeRoleCite,
  parseRoleCiteAt,
  createRoleCiteNode,
  roleCiteDisplayName,
  type RoleCiteRef,
} from "./roleCite";
export {
  MentionEditor,
  type MentionEditorHandle,
  type MentionEditorProps,
  type MentionTriggerAnchor,
} from "./MentionEditor";
export {
  ComposerEditor,
  type ComposerEditorHandle,
  type ComposerEditorProps,
} from "./ComposerEditor";
export { computeCaretMentionStyle, scrollableAncestors } from "./placement";
