import { useCallback, type Dispatch, type MutableRefObject, type SetStateAction } from "react";
import { useReader, normalizeReaderPath } from "../../../../store/reader";
import { newTabId } from "../storage/panelTabs";
import type { PanelTab } from "../types";

export function useOpenFileTab(
  setTabs: Dispatch<SetStateAction<PanelTab[]>>,
  setActiveTabId: Dispatch<SetStateAction<string | null>>,
  activeTabIdRef: MutableRefObject<string | null>,
) {
  // Open a file as a top-level reader tab: reuse an existing tab for the same
  // path, replace a stale renamed path in-place, fill an empty reader tab, or
  // create a new tab.
  return useCallback(
    (filePath: string) => {
      if (!filePath) return;
      const key = normalizeReaderPath(filePath);
      setTabs((prev) => {
        const existing = prev.find(
          (tb) => tb.kind === "reader" && tb.path && normalizeReaderPath(tb.path) === key,
        );
        if (existing) {
          setActiveTabId(existing.id);
          return prev;
        }

        const readerKeys = new Set(
          useReader.getState().tabs.map((t) => normalizeReaderPath(t.path)),
        );
        const active = prev.find((tb) => tb.id === activeTabIdRef.current);

        // Rename/move: active (or any) reader chrome still points at a path the
        // reader store no longer has — swap that slot to the new path in place.
        const stale =
          (active?.kind === "reader" &&
            active.path &&
            !readerKeys.has(normalizeReaderPath(active.path)) &&
            active) ||
          prev.find(
            (tb) =>
              tb.kind === "reader" &&
              !!tb.path &&
              !readerKeys.has(normalizeReaderPath(tb.path)),
          );
        if (stale && stale.kind === "reader") {
          setActiveTabId(stale.id);
          return prev.map((tb) =>
            tb.id === stale.id ? { ...tb, path: filePath } : tb,
          );
        }

        if (active && active.kind === "reader" && !active.path) {
          return prev.map((tb) => (tb.id === active.id ? { ...tb, path: filePath } : tb));
        }
        const tab: PanelTab = { id: newTabId(), kind: "reader", path: filePath };
        setActiveTabId(tab.id);
        return [...prev, tab];
      });
    },
    [setTabs, setActiveTabId, activeTabIdRef],
  );
}
