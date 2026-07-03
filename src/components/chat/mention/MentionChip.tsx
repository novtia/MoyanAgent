import { mentionBasename } from "./core";
import { MentionIcon } from "./MentionIcon";

/**
 * Static, read-only mention "reference card" — the React counterpart of the
 * contenteditable chip created by {@link createMentionNode}. Used to render
 * mentions inside message history (chat list).
 */
export function MentionChip({ path }: { path: string }) {
  return (
    <span className="composer-mention composer-mention--static" title={path}>
      <MentionIcon path={path} />
      <span className="composer-mention-label">{mentionBasename(path)}</span>
    </span>
  );
}
