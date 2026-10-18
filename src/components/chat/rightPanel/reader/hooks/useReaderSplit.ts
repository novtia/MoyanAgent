import { useCallback, useEffect, useRef, useState } from "react";
import { useReaderFind } from "../../../../../store/readerFind";
import { useRightPanel } from "../../../../../store/rightPanel";
import { useReaderTree } from "../../../../../store/readerTree";
import { useSession } from "../../../../../store/session";
import { usePanelTab } from "../../context/PanelTabContext";
import { DEFAULT_RATIO, MAX_RATIO, MIN_RATIO } from "../constants";
import type { RightView } from "../types";

function clampRatio(value: number): number {
  return Math.min(MAX_RATIO, Math.max(MIN_RATIO, value));
}

/**
 * Split layout of a reader tab. The file tree is shared by the whole project:
 * its visibility and the editor/tree ratio live in the per-project tree store,
 * so every reader tab shows the same tree. Only the right-pane mode (tree vs
 * find results) and markdown preview stay in the tab's own view state, because
 * they follow that tab's find session and file.
 */
export function useReaderSplit(isMarkdown: boolean, path: string | null | undefined) {
  const { tabId, isActive } = usePanelTab();
  const view = useRightPanel((s) => s.tabs.find((tb) => tb.id === tabId)?.view);
  const updateTabView = useRightPanel((s) => s.updateTabView);

  const projectId = useSession((s) => s.active?.session.project_id ?? null);
  const treeLayout = useReaderTree((s) => (projectId ? s.byProject[projectId] : undefined));
  const setTreeShow = useReaderTree((s) => s.setShowTree);
  const setTreeRatio = useReaderTree((s) => s.setRatio);

  const storedRatio = treeLayout?.ratio ?? DEFAULT_RATIO;
  const showTree = treeLayout?.showTree ?? true;
  const setShowTree = useCallback(
    (show: boolean) => {
      if (projectId) setTreeShow(projectId, show);
    },
    [projectId, setTreeShow],
  );
  const rightView: RightView = view?.rightView ?? "tree";
  const preview = (view?.preview ?? false) && isMarkdown;

  // Find state belongs to whichever tab it is bound to. A hidden pane (or one
  // whose bind has not landed yet) must not react to it, or it would rewrite
  // its own layout in the background.
  const findBound = useReaderFind((s) => s.boundTabId === tabId);
  const findOpen = useReaderFind((s) => s.open) && isActive && findBound;
  const openFind = useReaderFind((s) => s.openFind);
  const closeFind = useReaderFind((s) => s.close);

  // Live ratio while dragging: committing per mousemove would hammer storage.
  const [dragRatio, setDragRatio] = useState<number | null>(null);
  const [resizing, setResizing] = useState(false);
  const containerRef = useRef<HTMLDivElement | null>(null);
  const ratio = dragRatio ?? storedRatio;

  const setRatio = useCallback(
    (next: number) => {
      if (projectId) setTreeRatio(projectId, clampRatio(next));
    },
    [projectId, setTreeRatio],
  );

  const setPreview = useCallback(
    (next: boolean) => updateTabView(tabId, { preview: next }),
    [tabId, updateTabView],
  );

  // Preview only makes sense for markdown; drop the flag when switching files.
  useEffect(() => {
    if (!isMarkdown && view?.preview) updateTabView(tabId, { preview: false });
  }, [isMarkdown, path, view?.preview, tabId, updateTabView]);

  // Ctrl+F (or the search button) opens find → surface the search pane.
  useEffect(() => {
    if (!isActive || !findBound) return;
    if (findOpen) {
      if (rightView !== "search") updateTabView(tabId, { rightView: "search" });
      if (!showTree) setShowTree(true);
    } else if (rightView === "search") {
      updateTabView(tabId, { rightView: "tree" });
    }
  }, [isActive, findBound, findOpen, rightView, showTree, tabId, updateTabView, setShowTree]);

  const toggleFileTree = useCallback(() => {
    if (showTree && rightView === "tree") {
      setShowTree(false);
      return;
    }
    if (findOpen) closeFind();
    updateTabView(tabId, { rightView: "tree" });
    setShowTree(true);
  }, [showTree, rightView, findOpen, closeFind, tabId, updateTabView, setShowTree]);

  const toggleSearch = useCallback(() => {
    if (findOpen) {
      closeFind();
    } else {
      setShowTree(true);
      openFind();
    }
  }, [findOpen, closeFind, openFind, setShowTree]);

  useEffect(() => {
    if (!resizing) return;
    const onMove = (e: MouseEvent) => {
      const el = containerRef.current;
      if (!el) return;
      const rect = el.getBoundingClientRect();
      if (rect.width <= 0) return;
      setDragRatio(clampRatio((e.clientX - rect.left) / rect.width));
    };
    const onUp = () => setResizing(false);
    window.addEventListener("mousemove", onMove);
    window.addEventListener("mouseup", onUp);
    return () => {
      window.removeEventListener("mousemove", onMove);
      window.removeEventListener("mouseup", onUp);
    };
  }, [resizing]);

  // Commit the dragged ratio to the project once the mouse is released.
  useEffect(() => {
    if (resizing || dragRatio == null) return;
    setRatio(dragRatio);
    setDragRatio(null);
  }, [resizing, dragRatio, setRatio]);

  useEffect(() => {
    if (!resizing) return;
    const prevCursor = document.body.style.cursor;
    const prevSelect = document.body.style.userSelect;
    document.body.style.cursor = "col-resize";
    document.body.style.userSelect = "none";
    return () => {
      document.body.style.cursor = prevCursor;
      document.body.style.userSelect = prevSelect;
    };
  }, [resizing]);

  return {
    ratio,
    setRatio,
    resizing,
    setResizing,
    showTree,
    rightView,
    preview,
    setPreview,
    containerRef,
    findOpen,
    toggleFileTree,
    toggleSearch,
  };
}
