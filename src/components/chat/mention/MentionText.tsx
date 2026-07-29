import { Fragment, type ReactNode } from "react";
import {
  parseMentionSegments,
  type MentionMediaRenderData,
} from "./core";
import { MentionChip } from "./MentionChip";
import { RoleCiteChip } from "./RoleCiteChip";
import { highlightQuery } from "../../../utils/highlightQuery";

/**
 * Render plain message text, turning serialized `@file` / `@role` tokens into
 * static reference cards.
 */
export function MentionText({
  text,
  mediaByPath = {},
  highlight,
}: {
  text: string;
  mediaByPath?: Record<string, MentionMediaRenderData>;
  /** When set, wrap plain-text segments with case-insensitive <mark> highlights. */
  highlight?: string;
}): ReactNode {
  if (!text) return null;

  const renderText = (value: string): ReactNode =>
    highlight ? highlightQuery(value, highlight) : value;

  if (!text.includes("@")) return renderText(text);

  const segments = parseMentionSegments(text);
  if (segments.length === 1 && segments[0].type === "text") {
    return renderText(text);
  }

  return segments.map((seg, i) => {
    if (seg.type === "text") {
      return <Fragment key={`t${i}`}>{renderText(seg.value)}</Fragment>;
    }
    if (seg.type === "roleCite") {
      return <RoleCiteChip key={`r${i}`} id={seg.id} name={seg.name} />;
    }
    return (
      <MentionChip
        key={`m${i}`}
        path={seg.path}
        range={seg.range}
        previewSrc={mediaByPath[seg.path]?.previewSrc}
      />
    );
  });
}
