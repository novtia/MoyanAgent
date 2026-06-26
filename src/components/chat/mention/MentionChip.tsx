import { MENTION_PREFIX, mentionBasename } from "./core";

/**
 * Static, read-only mention "reference card" — the React counterpart of the
 * contenteditable chip created by {@link createMentionNode}. Used to render
 * mentions inside message history (chat list).
 *
 * The leading `@` glyph is a real text node so the inline-flex chip exposes a
 * genuine text baseline, keeping the card's text aligned with surrounding text.
 */
export function MentionChip({ path }: { path: string }) {
  return (
    <span className="composer-mention composer-mention--static" title={path}>
      <span className="composer-mention-at">{MENTION_PREFIX}</span>
      <span className="composer-mention-label">{mentionBasename(path)}</span>
    </span>
  );
}
