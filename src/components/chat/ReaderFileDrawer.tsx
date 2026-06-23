import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { ReaderFileExplorer } from "./ReaderFileExplorer";

const HEIGHT_KEY = "atelier:reader-files-drawer-height";
const COLLAPSED_HEIGHT = 28;
const SNAP_THRESHOLD = 48;
const DEFAULT_HEIGHT = 240;
const MIN_EXPANDED = 120;
const MAX_RATIO = 0.7;

function readStoredHeight(): number {
  try {
    const raw = window.localStorage.getItem(HEIGHT_KEY);
    if (!raw) return COLLAPSED_HEIGHT;
    const n = Number.parseInt(raw, 10);
    if (!Number.isFinite(n) || n < SNAP_THRESHOLD) return COLLAPSED_HEIGHT;
    return n;
  } catch {
    return COLLAPSED_HEIGHT;
  }
}

export function ReaderFileDrawer() {
  const { t } = useTranslation();
  const drawerRef = useRef<HTMLDivElement>(null);
  const dragRef = useRef<{ startY: number; startHeight: number } | null>(null);
  const [height, setHeight] = useState(readStoredHeight);
  const [resizing, setResizing] = useState(false);
  const expanded = height >= SNAP_THRESHOLD;

  useEffect(() => {
    if (resizing) return;
    try {
      window.localStorage.setItem(HEIGHT_KEY, String(height));
    } catch {
      /* ignore */
    }
  }, [height, resizing]);

  useEffect(() => {
    if (!resizing) return;
    const onMove = (e: MouseEvent) => {
      if (!dragRef.current) return;
      const parentHeight =
        drawerRef.current?.parentElement?.getBoundingClientRect().height ?? 400;
      const maxHeight = Math.max(MIN_EXPANDED, Math.round(parentHeight * MAX_RATIO));
      const delta = dragRef.current.startY - e.clientY;
      const next = Math.min(
        maxHeight,
        Math.max(COLLAPSED_HEIGHT, Math.round(dragRef.current.startHeight + delta)),
      );
      setHeight(next);
    };
    const onUp = () => {
      setHeight((cur) => (cur < SNAP_THRESHOLD ? COLLAPSED_HEIGHT : cur));
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
    if (!resizing) return;
    const prevCursor = document.body.style.cursor;
    const prevSelect = document.body.style.userSelect;
    document.body.style.cursor = "row-resize";
    document.body.style.userSelect = "none";
    return () => {
      document.body.style.cursor = prevCursor;
      document.body.style.userSelect = prevSelect;
    };
  }, [resizing]);

  const onHandleMouseDown = useCallback(
    (e: React.MouseEvent) => {
      if (e.button !== 0) return;
      e.preventDefault();
      dragRef.current = { startY: e.clientY, startHeight: height };
      setResizing(true);
    },
    [height],
  );

  const onHandleDoubleClick = useCallback(() => {
    setHeight((cur) => (cur >= SNAP_THRESHOLD ? COLLAPSED_HEIGHT : DEFAULT_HEIGHT));
  }, []);

  return (
    <div
      ref={drawerRef}
      className={`reader-files-drawer${expanded ? " is-expanded" : " is-collapsed"}`}
      style={{ ["--reader-files-drawer-height" as string]: `${height}px` }}
    >
      <div
        className="reader-files-drawer-handle"
        role="separator"
        aria-orientation="horizontal"
        aria-label={t("fileExplorer.drawerHandle")}
        title={t("fileExplorer.drawerHandle")}
        onMouseDown={onHandleMouseDown}
        onDoubleClick={onHandleDoubleClick}
      >
        <span className="reader-files-drawer-grip" />
      </div>
      {expanded && (
        <div className="reader-files-drawer-body">
          <ReaderFileExplorer />
        </div>
      )}
    </div>
  );
}
