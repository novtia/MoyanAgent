/** Last scrollTop per open reader path + view, so agent Create/Edit cannot
 *  yank the viewport: switching between the plain editor and the diff view
 *  (or remounting either) restores where the user was reading. Preview and
 *  source keep separate positions — they do not share a scroller. */
const scrollTopByKey = new Map<string, number>();

function normalizePath(path: string): string {
  let p = path.trim();
  if (p.startsWith("\\\\?\\")) p = p.slice(4);
  return p.replace(/\\/g, "/").toLowerCase();
}

export type ReaderViewportSlot = "source" | "preview";

function storageKey(path: string, slot: ReaderViewportSlot): string {
  return `${normalizePath(path)}::${slot}`;
}

export function rememberReaderViewport(
  path: string | undefined,
  scrollTop: number,
  slot: ReaderViewportSlot = "source",
) {
  const trimmed = path?.trim();
  if (!trimmed) return;
  scrollTopByKey.set(storageKey(trimmed, slot), Math.max(0, scrollTop));
}

export function readerViewportTop(
  path: string | undefined,
  slot: ReaderViewportSlot = "source",
): number {
  const trimmed = path?.trim();
  if (!trimmed) return 0;
  return scrollTopByKey.get(storageKey(trimmed, slot)) ?? 0;
}
