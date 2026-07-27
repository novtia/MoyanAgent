/** Ctrl (Windows/Linux) or Meta (macOS) multi-select modifier. */
export function isModifierClick(e: { ctrlKey: boolean; metaKey: boolean }): boolean {
  return e.ctrlKey || e.metaKey;
}

/** Shift-click range select over the gallery media order. */
export function selectRange(
  orderedIds: string[],
  anchorId: string | null,
  targetId: string,
): string[] {
  const anchor = anchorId ?? targetId;
  const ai = orderedIds.indexOf(anchor);
  const bi = orderedIds.indexOf(targetId);
  if (ai < 0 || bi < 0) return [targetId];
  const [lo, hi] = ai <= bi ? [ai, bi] : [bi, ai];
  return orderedIds.slice(lo, hi + 1);
}

/**
 * Ids to operate on for context menu / bulk actions.
 * If `id` is in a multi-selection, return the full selection; else `[id]`.
 */
export function resolveBulkIds(id: string, selectedIds: string[]): string[] {
  if (selectedIds.length > 1 && selectedIds.includes(id)) {
    return [...selectedIds];
  }
  return [id];
}

export function toggleId(selectedIds: string[], id: string): string[] {
  if (selectedIds.includes(id)) {
    return selectedIds.filter((x) => x !== id);
  }
  return [...selectedIds, id];
}
