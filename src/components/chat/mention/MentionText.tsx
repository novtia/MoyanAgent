import { Fragment, type ReactNode } from "react";
import { parseMentionSegments } from "./core";
import { MentionChip } from "./MentionChip";

/**
 * Render plain message text, turning serialized `@<absolutePath>` mentions into
 * static reference cards ({@link MentionChip}).
 */
export function MentionText({ text }: { text: string }): ReactNode {
  if (!text) return null;
  if (!text.includes("@")) return text;

  const segments = parseMentionSegments(text);
  if (segments.length === 1 && segments[0].type === "text") {
    return text;
  }

  return segments.map((seg, i) =>
    seg.type === "text" ? (
      <Fragment key={`t${i}`}>{seg.value}</Fragment>
    ) : (
      <MentionChip key={`m${i}`} path={seg.path} />
    ),
  );
}
