import { Fragment, type ReactNode } from "react";
import { MENTION_RE } from "./core";
import { MentionChip } from "./MentionChip";

/**
 * Render plain message text, turning serialized `@<absolutePath>` mentions into
 * static reference cards ({@link MentionChip}). The chat list only persists the
 * message as plain text (mentions serialized as `@<path>`), so we re-parse them
 * here using the shared {@link MENTION_RE}.
 */
export function MentionText({ text }: { text: string }): ReactNode {
  if (!text) return null;
  if (!text.includes("@")) return text;

  const parts: ReactNode[] = [];
  let last = 0;
  let key = 0;
  MENTION_RE.lastIndex = 0;
  let m: RegExpExecArray | null;
  while ((m = MENTION_RE.exec(text)) !== null) {
    if (m.index > last) {
      parts.push(<Fragment key={`t${key++}`}>{text.slice(last, m.index)}</Fragment>);
    }
    parts.push(<MentionChip key={`m${key++}`} path={m[1]} />);
    last = m.index + m[0].length;
  }
  if (last < text.length) {
    parts.push(<Fragment key={`t${key++}`}>{text.slice(last)}</Fragment>);
  }
  return parts;
}
