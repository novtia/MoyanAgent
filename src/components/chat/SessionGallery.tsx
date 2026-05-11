import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { useSession } from "../../store/session";
import { srcOf } from "../../api/tauri";
import { collectSessionGalleryImages } from "../../sessionGallery";
import type { ImageRefAbs } from "../../types";

export const ATELIER_DRAG_TYPE = "application/x-atelier-image";

interface SessionGalleryProps {
  open: boolean;
  onClose: () => void;
  onPreviewImage: (img: ImageRefAbs) => void;
}

const MIN_WIDTH = 240;
const MAX_WIDTH = 640;
const DEFAULT_WIDTH = 320;
const STORAGE_KEY = "atelier:chat-gallery-width";

const MASONRY_GAP = 6;
const MASONRY_MIN_COL = 120;

function readStoredWidth() {
  if (typeof window === "undefined") return DEFAULT_WIDTH;
  const raw = window.localStorage.getItem(STORAGE_KEY);
  if (!raw) return DEFAULT_WIDTH;
  const v = parseInt(raw, 10);
  if (!Number.isFinite(v)) return DEFAULT_WIDTH;
  return Math.min(MAX_WIDTH, Math.max(MIN_WIDTH, v));
}

export function SessionGallery({ open, onClose, onPreviewImage }: SessionGalleryProps) {
  const { t } = useTranslation();
  const active = useSession((s) => s.active);

  const [width, setWidth] = useState<number>(readStoredWidth);
  const [resizing, setResizing] = useState(false);
  const dragRef = useRef<{ rightEdge: number } | null>(null);
  const asideRef = useRef<HTMLElement | null>(null);
  const gridRef = useRef<HTMLDivElement | null>(null);
  const [innerWidth, setInnerWidth] = useState(0);

  const images = useMemo(() => collectSessionGalleryImages(active), [active]);

  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, onClose]);

  useLayoutEffect(() => {
    const el = gridRef.current;
    if (!el) return;
    const measure = () => {
      const cs = window.getComputedStyle(el);
      const px = parseFloat(cs.paddingLeft || "0") + parseFloat(cs.paddingRight || "0");
      setInnerWidth(Math.max(0, el.clientWidth - px));
    };
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  const layout = useMemo(() => {
    if (innerWidth <= 0 || images.length === 0) {
      return { items: [] as Array<{ img: ImageRefAbs; x: number; y: number; w: number; h: number }>, total: 0 };
    }
    const cols = Math.max(1, Math.floor((innerWidth + MASONRY_GAP) / (MASONRY_MIN_COL + MASONRY_GAP)));
    const colW = (innerWidth - MASONRY_GAP * (cols - 1)) / cols;
    const heights = new Array(cols).fill(0);
    const items = images.map((img) => {
      const aspect = img.width && img.height && img.width > 0 ? img.height / img.width : 1;
      const h = Math.max(40, Math.round(colW * aspect));
      let idx = 0;
      let min = heights[0];
      for (let i = 1; i < cols; i++) {
        if (heights[i] < min) {
          min = heights[i];
          idx = i;
        }
      }
      const x = idx * (colW + MASONRY_GAP);
      const y = heights[idx];
      heights[idx] = y + h + MASONRY_GAP;
      return { img, x, y, w: colW, h };
    });
    const total = Math.max(0, ...heights) - MASONRY_GAP;
    return { items, total };
  }, [images, innerWidth]);

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
    try {
      window.localStorage.setItem(STORAGE_KEY, String(width));
    } catch {
      /* ignore */
    }
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

  const onTileDragStart = (e: React.DragEvent<HTMLButtonElement>, img: ImageRefAbs) => {
    if (!e.dataTransfer) return;
    e.dataTransfer.effectAllowed = "copy";
    const payload = JSON.stringify({ id: img.id, abs_path: img.abs_path });
    e.dataTransfer.setData(ATELIER_DRAG_TYPE, payload);
    e.dataTransfer.setData("text/plain", img.abs_path);
    const imgEl = e.currentTarget.querySelector("img");
    if (imgEl) {
      try {
        e.dataTransfer.setDragImage(imgEl, 24, 24);
      } catch {
        /* ignore */
      }
    }
  };

  const style = { ["--chat-gallery-width" as string]: `${width}px` } as React.CSSProperties;

  return (
    <aside
      ref={asideRef}
      className={`chat-gallery ${open ? "open" : ""} ${resizing ? "is-resizing" : ""}`}
      aria-hidden={!open}
      aria-label={t("chat.galleryTitle")}
      style={style}
    >
      <div
        className="chat-gallery-resizer"
        role="separator"
        aria-orientation="vertical"
        title={t("chat.galleryResize")}
        onMouseDown={onResizerMouseDown}
        onDoubleClick={() => setWidth(DEFAULT_WIDTH)}
      />

      <div className="chat-gallery-inner">
        <div className="chat-gallery-header">
          <span className="chat-gallery-title">
            {t("chat.galleryTitle")}
            <span className="chat-gallery-count">{images.length}</span>
          </span>
          <button
            type="button"
            className="ghost-btn"
            title={t("common.close")}
            onClick={onClose}
            tabIndex={open ? 0 : -1}
          >
            <CloseIcon />
          </button>
        </div>

        <div className="chat-gallery-grid" ref={gridRef}>
          {images.length === 0 ? (
            <div className="chat-gallery-empty">{t("chat.galleryEmpty")}</div>
          ) : (
            <div className="chat-gallery-canvas" style={{ height: layout.total }}>
              {layout.items.map(({ img, x, y, w, h }) => (
                <button
                  key={img.id}
                  type="button"
                  className={`chat-gallery-tile role-${img.role}`}
                  style={{ transform: `translate(${x}px, ${y}px)`, width: w, height: h }}
                  onClick={() => onPreviewImage(img)}
                  title={img.rel_path}
                  tabIndex={open ? 0 : -1}
                  draggable
                  onDragStart={(e) => onTileDragStart(e, img)}
                >
                  <img
                    src={srcOf(img.thumb_abs_path || img.abs_path)}
                    alt=""
                    loading="lazy"
                    decoding="async"
                    draggable={false}
                  />
                </button>
              ))}
            </div>
          )}
        </div>
      </div>
    </aside>
  );
}

function CloseIcon() {
  return (
    <svg
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.6"
      strokeLinecap="round"
      strokeLinejoin="round"
    >
      <path d="M6 6 18 18" />
      <path d="m18 6-12 12" />
    </svg>
  );
}
