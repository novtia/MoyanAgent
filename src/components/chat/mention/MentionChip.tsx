import {
  mentionDisplayLabel,
  mediaMentionKind,
  type MentionRange,
} from "./core";
import { MentionIcon } from "./MentionIcon";

/**
 * Static, read-only mention "reference card" — the React counterpart of the
 * contenteditable chip created by {@link createMentionNode}. Used to render
 * mentions inside message history (chat list).
 */
export function MentionChip({
  path,
  previewSrc,
  range,
}: {
  path: string;
  previewSrc?: string;
  range?: MentionRange;
}) {
  const mediaKind = mediaMentionKind(path);
  const displayLabel = mentionDisplayLabel(path, range);
  return (
    <span
      className={`composer-mention composer-mention--static${
        mediaKind ? ` is-media media-${mediaKind}` : ""
      }${
        mediaKind === "image" && previewSrc ? " has-preview" : ""
      }`}
      title={mediaKind ? displayLabel : path}
    >
      {mediaKind === "image" && previewSrc ? (
        <img
          className="composer-mention-image"
          src={previewSrc}
          alt={displayLabel}
          draggable={false}
        />
      ) : (
        <>
          <MentionIcon path={path} />
          <span className="composer-mention-label">{displayLabel}</span>
        </>
      )}
    </span>
  );
}
