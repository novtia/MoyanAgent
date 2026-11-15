import { useEffect, useRef, useState } from "react";
import { DEFAULT_WIDTH } from "../constants";
import {
  clampPanelWidth,
  estimatePanelMaxWidth,
  panelMaxWidth,
  persistWidth,
  readStoredWidth,
} from "../storage/panelWidth";

export function usePanelResize(open: boolean) {
  const [preferred, setPreferred] = useState<number>(readStoredWidth);
  const [maxWidth, setMaxWidth] = useState<number>(estimatePanelMaxWidth);
  const [resizing, setResizing] = useState(false);
  const dragRef = useRef<{ rightEdge: number } | null>(null);
  const asideRef = useRef<HTMLElement | null>(null);
  const maxWidthRef = useRef(maxWidth);
  maxWidthRef.current = maxWidth;
  const width = clampPanelWidth(preferred, maxWidth);

  useEffect(() => {
    const row = asideRef.current?.parentElement;
    if (!row) return;
    const apply = () => setMaxWidth(panelMaxWidth(row.clientWidth));
    apply();
    const observer = new ResizeObserver(apply);
    observer.observe(row);
    return () => observer.disconnect();
  }, [open]);

  useEffect(() => {
    if (!resizing) return;
    const onMove = (e: MouseEvent) => {
      if (!dragRef.current) return;
      const next = clampPanelWidth(
        Math.round(dragRef.current.rightEdge - e.clientX),
        maxWidthRef.current,
      );
      setPreferred(next);
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
    persistWidth(preferred);
  }, [resizing, preferred]);

  const onResizerMouseDown = (e: React.MouseEvent) => {
    if (!open) return;
    if (e.button !== 0) return;
    if (!asideRef.current) return;
    e.preventDefault();
    const rect = asideRef.current.getBoundingClientRect();
    dragRef.current = { rightEdge: rect.right };
    setResizing(true);
  };

  const resetWidth = () => setPreferred(DEFAULT_WIDTH);

  return {
    width,
    maxWidth,
    setWidth: setPreferred,
    resizing,
    asideRef,
    onResizerMouseDown,
    resetWidth,
  };
}
