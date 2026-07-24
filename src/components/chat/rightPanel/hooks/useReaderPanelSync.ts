import { useLayoutEffect, useRef, type Dispatch, type SetStateAction } from "react";
import {
  useReader,
  normalizeReaderPath,
  applyReaderPathOpsToPath,
} from "../../../../store/reader";
import { newTabId, pickActiveId } from "../storage/panelTabs";
import type { PanelTab } from "../types";

export function useReaderPanelSync(
  openFileTab: (filePath: string) => void,
  setTabs: Dispatch<SetStateAction<PanelTab[]>>,
  setActiveTabId: Dispatch<SetStateAction<string | null>>,
) {
  // Auto-open: when a document is requested (openSeq bumps), ensure a reader
  // tab exists and make it active so the document shows immediately.
  // Also runs after rename/move (remapPaths bumps openSeq) so the panel closes
  // the old path slot and focuses the new path without a gap.
  const readerOpenSeq = useReader((s) => s.openSeq);
  const lastReaderSeq = useRef(readerOpenSeq);
  const lastReaderActive = useRef<string | null>(useReader.getState().activeTabId);
  const lastReaderActivePath = useRef<string | null>(
    useReader.getState().tabs.find((t) => t.id === useReader.getState().activeTabId)?.path ??
      null,
  );
  useLayoutEffect(() => {
    if (readerOpenSeq === lastReaderSeq.current) return;
    lastReaderSeq.current = readerOpenSeq;
    const st = useReader.getState();
    const active = st.tabs.find((tb) => tb.id === st.activeTabId) ?? null;
    const activePath = active?.path ?? null;
    const sameTab = st.activeTabId === lastReaderActive.current;
    const samePath =
      activePath != null &&
      lastReaderActivePath.current != null &&
      normalizeReaderPath(activePath) === normalizeReaderPath(lastReaderActivePath.current);
    lastReaderActive.current = st.activeTabId;
    lastReaderActivePath.current = activePath;
    // Passive lazy-loads keep both active tab and path unchanged.
    if (sameTab && samePath) return;
    if (activePath) openFileTab(activePath);
  }, [readerOpenSeq, openFileTab]);

  // Keep panel reader tab paths in sync when files are renamed/moved/deleted.
  // useLayoutEffect: apply before paint so the tab title never flashes the old name.
  const readerPathSeq = useReader((s) => s.pathSeq);
  const lastPathSeq = useRef(readerPathSeq);
  useLayoutEffect(() => {
    if (readerPathSeq === lastPathSeq.current) return;
    lastPathSeq.current = readerPathSeq;
    const ops = useReader.getState().lastPathOps;
    if (!ops.length) return;

    setTabs((prev) => {
      let changed = false;
      const next: PanelTab[] = [];
      const seen = new Set<string>();
      let activatePath: string | null = null;

      for (const tb of prev) {
        if (tb.kind !== "reader" || !tb.path) {
          next.push(tb);
          continue;
        }
        const rewritten = applyReaderPathOpsToPath(tb.path, ops);
        if (rewritten == null) {
          // Closed (deleted) — drop the chrome tab.
          changed = true;
          continue;
        }
        if (rewritten !== tb.path) {
          changed = true;
          activatePath = rewritten;
        }
        const key = normalizeReaderPath(rewritten);
        if (seen.has(key)) {
          // Duplicate destination after rename: close the extra old slot.
          changed = true;
          continue;
        }
        seen.add(key);
        next.push(rewritten === tb.path ? tb : { ...tb, path: rewritten });
      }

      // Remap targeted a file that had no panel chrome yet — open it.
      if (!changed) {
        for (const op of ops) {
          if (op.type !== "remap") continue;
          const key = normalizeReaderPath(op.to);
          if (seen.has(key)) continue;
          if (useReader.getState().getTabByPath(op.to)) {
            const tab: PanelTab = { id: newTabId(), kind: "reader", path: op.to };
            next.push(tab);
            seen.add(key);
            activatePath = op.to;
            changed = true;
          }
        }
      }

      if (!changed) return prev;

      if (activatePath) {
        const id =
          next.find(
            (tb) =>
              tb.kind === "reader" &&
              tb.path &&
              normalizeReaderPath(tb.path) === normalizeReaderPath(activatePath!),
          )?.id ?? null;
        if (id) setActiveTabId(id);
        else setActiveTabId((cur) => pickActiveId(next, cur));
      } else {
        setActiveTabId((cur) => pickActiveId(next, cur));
      }
      return next;
    });
  }, [readerPathSeq, setTabs, setActiveTabId]);
}
