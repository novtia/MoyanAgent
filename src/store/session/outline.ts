import type { MessageAbs, MessageOutlineItem } from "../../types";

export const MESSAGE_WINDOW_SIZE = 60;
/** Prefetch more history when the virtual list nears this many rows from the edge. */
export const MESSAGE_WINDOW_PREFETCH = 12;

export function outlineFromMessages(messages: MessageAbs[]): MessageOutlineItem[] {
  return messages
    .filter((m) => !m.id.startsWith("tmp-"))
    .map((m) => ({
      id: m.id,
      role: m.role,
      preview: previewFromText(m.role, m.text),
      created_at: m.created_at,
    }));
}

export function previewFromText(role: string, text: string | null | undefined): string | null {
  const raw = (text ?? "").trim();
  if (!raw) {
    if (role === "assistant") return "(tools/media)";
    if (role === "user") return "(attachment)";
    if (role === "error") return "(error)";
    return null;
  }
  const collapsed = raw.replace(/\s+/g, " ").trim();
  if (collapsed.length <= 100) return collapsed;
  return `${collapsed.slice(0, 99)}…`;
}

export function upsertOutlineItem(
  outline: MessageOutlineItem[],
  item: MessageOutlineItem,
): MessageOutlineItem[] {
  const idx = outline.findIndex((o) => o.id === item.id);
  if (idx < 0) {
    const next = [...outline, item];
    next.sort((a, b) => a.created_at - b.created_at);
    return next;
  }
  const next = outline.slice();
  next[idx] = item;
  return next;
}

export function removeOutlineItem(
  outline: MessageOutlineItem[],
  messageId: string,
): MessageOutlineItem[] {
  return outline.filter((o) => o.id !== messageId);
}

export function mergeMessageWindow(
  existing: MessageAbs[],
  incoming: MessageAbs[],
): MessageAbs[] {
  if (incoming.length === 0) return existing;
  const byId = new Map<string, MessageAbs>();
  for (const m of existing) byId.set(m.id, m);
  for (const m of incoming) byId.set(m.id, m);
  return [...byId.values()].sort((a, b) => a.created_at - b.created_at);
}
