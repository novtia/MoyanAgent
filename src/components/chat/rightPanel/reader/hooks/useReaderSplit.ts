import { useCallback, useEffect, useRef, useState } from "react";
import { useReaderFind } from "../../../../../store/readerFind";
import { MAX_RATIO, MIN_RATIO } from "../constants";
import {
  persistRatio,
  persistShowTreeValue,
  readStoredRatio,
  readStoredShowTree,
} from "../storage/splitStorage";
import type { RightView } from "../types";

export function useReaderSplit(isMarkdown: boolean, path: string | null | undefined) {
  const findOpen = useReaderFind((s) => s.open);
  const openFind = useReaderFind((s) => s.openFind);
  const closeFind = useReaderFind((s) => s.close);

  const [ratio, setRatio] = useState<number>(readStoredRatio);
  const [resizing, setResizing] = useState(false);
  const [showTree, setShowTree] = useState<boolean>(readStoredShowTree);
  const [rightView, setRightView] = useState<RightView>("tree");
  const [preview, setPreview] = useState(false);
  const containerRef = useRef<HTMLDivElement | null>(null);

  // Preview only makes sense for markdown; reset when switching files.
  useEffect(() => {
    if (!isMarkdown) setPreview(false);
  }, [isMarkdown, path]);

  // Ctrl+F (or the search button) opens find → surface the search panel.
  useEffect(() => {
    if (findOpen) {
      setShowTree(true);
      setRightView("search");
    } else {
      setRightView((v) => (v === "search" ? "tree" : v));
    }
  }, [findOpen]);

  const persistShowTree = useCallback((next: boolean) => {
    setShowTree(next);
    persistShowTreeValue(next);
  }, []);

  const toggleFileTree = useCallback(() => {
    if (showTree && rightView === "tree") {
      persistShowTree(false);
      return;
    }
    if (findOpen) closeFind();
    setRightView("tree");
    persistShowTree(true);
  }, [showTree, rightView, findOpen, closeFind, persistShowTree]);

  const toggleSearch = useCallback(() => {
    if (findOpen) {
      closeFind();
    } else {
      persistShowTree(true);
      openFind();
    }
  }, [findOpen, closeFind, openFind, persistShowTree]);

  useEffect(() => {
    if (resizing) return;
    persistRatio(ratio);
  }, [ratio, resizing]);

  useEffect(() => {
    if (!resizing) return;
    const onMove = (e: MouseEvent) => {
      const el = containerRef.current;
      if (!el) return;
      const rect = el.getBoundingClientRect();
      if (rect.width <= 0) return;
      const next = (e.clientX - rect.left) / rect.width;
      setRatio(Math.min(MAX_RATIO, Math.max(MIN_RATIO, next)));
    };
    const onUp = () => setResizing(false);
    window.addEventListener("mousemove", onMove);
    window.addEventListener("mouseup", onUp);
    return () => {
      window.removeEventListener("mousemove", onMove);
      window.removeEventListener("mouseup", onUp);
    };
  }, [resizing]);

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
