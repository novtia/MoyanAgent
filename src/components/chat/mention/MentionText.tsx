import { Fragment, type ReactNode } from "react";
import {
  parseMentionSegments,
  type MentionMediaRenderData,
} from "./core";
import { MentionChip } from "./MentionChip";
import { RoleCiteChip } from "./RoleCiteChip";

/**
 * Render plain message text, turning serialized `@file` / `@role` tokens into
 * static reference cards.
 */
export function MentionText({
  text,
  mediaByPath = {},
}: {
  text: string;
  mediaByPath?: Record<string, MentionMediaRenderData>;
}): ReactNode {
  if (!text) return null;
  if (!text.includes("@")) return text;

  const segments = parseMentionSegments(text);
  if (segments.length === 1 && segments[0].type === "text") {
    return text;
  }

  return segments.map((seg, i) => {
    if (seg.type === "text") {
      return <Fragment key={`t${i}`}>{seg.value}</Fragment>;
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
