import { useEffect, useRef, useState } from "react";
import { DEFAULT_WIDTH, MAX_WIDTH, MIN_WIDTH } from "../constants";
import { persistWidth, readStoredWidth } from "../storage/panelWidth";

export function usePanelResize(open: boolean) {
  const [width, setWidth] = useState<number>(readStoredWidth);
  const [resizing, setResizing] = useState(false);
  const dragRef = useRef<{ rightEdge: number } | null>(null);
  const asideRef = useRef<HTMLElement | null>(null);

  useEffect(() => {
    if (!resizing) return;
    const onMove = (e: MouseEvent) => {
      if (!dragRef.current) return;
      const next = Math.min(
        MAX_WIDTH,
        Math.max(MIN_WIDTH, Math.round(dragRef.current.rightEdge - e.clientX)),
      );
      setWidth(next);
    };
    const onUp = () => {
      setResizing(false);
      dragRef.current = null;
    };
    window.addEventListener("mousemove", onMove);
    window.addEventListener("mouseup", onUp);
    return () => {
      window.removeEventListener("mousemove", onMove);
      window.removeEventListener("mouseup", onUp);
    };
  }, [resizing]);

  useEffect(() => {
    if (resizing) {
      const prevCursor = document.body.style.cursor;
      const prevSelect = document.body.style.userSelect;
      document.body.style.cursor = "col-resize";
      document.body.style.userSelect = "none";
      return () => {
        document.body.style.cursor = prevCursor;
        document.body.style.userSelect = prevSelect;
      };
    }
  }, [resizing]);

  useEffect(() => {
    if (resizing) return;
    persistWidth(width);
  }, [resizing, width]);

  const onResizerMouseDown = (e: React.MouseEvent) => {
    if (!open) return;
    if (e.button !== 0) return;
    if (!asideRef.current) return;
    e.preventDefault();
    const rect = asideRef.current.getBoundingClientRect();
    dragRef.current = { rightEdge: rect.right };
    setResizing(true);
  };

  const resetWidth = () => setWidth(DEFAULT_WIDTH);

  return {
    width,
    setWidth,
    resizing,
    asideRef,
    onResizerMouseDown,
    resetWidth,
  };
}
