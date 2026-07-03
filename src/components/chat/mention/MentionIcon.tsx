import { MENTION_ICON_INNER, mentionIconKind } from "./core";

export function MentionIcon({ path, isDir }: { path: string; isDir?: boolean }) {
  const kind = mentionIconKind(path, isDir);
  return (
    <span className="composer-mention-icon" aria-hidden>
      <svg
        viewBox="0 0 24 24"
        width={14}
        height={14}
        fill="none"
        stroke="currentColor"
        strokeWidth={1.6}
        strokeLinecap="round"
        strokeLinejoin="round"
        dangerouslySetInnerHTML={{ __html: MENTION_ICON_INNER[kind] }}
      />
    </span>
  );
}
