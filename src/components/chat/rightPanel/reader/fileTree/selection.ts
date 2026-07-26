import { normalizeReaderPath } from "../../../../../store/reader";
import { useFileExplorer } from "../../../../../store/fileExplorer";

const TREE_PATH_ATTR = "data-tree-path";
const TREE_IS_DIR_ATTR = "data-tree-is-dir";

export { TREE_PATH_ATTR, TREE_IS_DIR_ATTR };

export function sameTreePath(a: string, b: string): boolean {
  return normalizeReaderPath(a) === normalizeReaderPath(b);
}

export function isPathSelected(path: string, selectedPaths: string[]): boolean {
  const key = normalizeReaderPath(path);
  return selectedPaths.some((p) => normalizeReaderPath(p) === key);
}

/** Visible tree rows in DOM order (matches expanded DFS). */
export function getVisibleTreePaths(container: HTMLElement | null): string[] {
  if (!container) return [];
  return Array.from(container.querySelectorAll<HTMLElement>(`[${TREE_PATH_ATTR}]`))
    .map((el) => el.getAttribute(TREE_PATH_ATTR) || "")
    .filter(Boolean);
}

export function getVisibleTreeEntries(
  container: HTMLElement | null,
): { path: string; isDir: boolean }[] {
  if (!container) return [];
  return Array.from(container.querySelectorAll<HTMLElement>(`[${TREE_PATH_ATTR}]`))
    .map((el) => {
      const path = el.getAttribute(TREE_PATH_ATTR) || "";
      if (!path) return null;
      return {
        path,
        isDir: el.getAttribute(TREE_IS_DIR_ATTR) === "1",
      };
    })
    .filter((x): x is { path: string; isDir: boolean } => !!x);
}

/** Shift-click range select over the currently visible tree order. */
export function selectVisibleRange(
  targetPath: string,
  visibleOrdered: string[],
): void {
  const store = useFileExplorer.getState();
  const anchor = store.selectedPath ?? targetPath;
  const orderKeys = visibleOrdered.map((p) => normalizeReaderPath(p));
  const ai = orderKeys.indexOf(normalizeReaderPath(anchor));
  const bi = orderKeys.indexOf(normalizeReaderPath(targetPath));
  if (ai < 0 || bi < 0) {
    store.setSelection(targetPath);
    return;
  }
  const [lo, hi] = ai <= bi ? [ai, bi] : [bi, ai];
  const range = visibleOrdered.slice(lo, hi + 1);
  store.setSelectedPaths(range);
}

/**
 * Paths to operate on for context menu / drag / delete.
 * If `path` is in the multi-selection, return the full selection; else `[path]`.
 */
export function resolveBulkPaths(path: string, selectedPaths: string[]): string[] {
  if (selectedPaths.length > 1 && isPathSelected(path, selectedPaths)) {
    return [...selectedPaths];
  }
  return [path];
}

export function isModifierClick(e: { ctrlKey: boolean; metaKey: boolean }): boolean {
  return e.ctrlKey || e.metaKey;
}
