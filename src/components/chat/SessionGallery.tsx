import { useLayoutEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { useSession } from "../../store/session";
import { srcOf } from "../../api/tauri";
import { collectSessionGalleryImages } from "../../sessionGallery";
import type { ImageRefAbs } from "../../types";

export const ATELIER_DRAG_TYPE = "application/x-atelier-image";

const MASONRY_GAP = 6;
const MASONRY_MIN_COL = 120;

interface GalleryContentProps {
  open: boolean;
  onPreviewImage: (img: ImageRefAbs) => void;
}

/**
 * Masonry grid of the current session's images. Rendered inside a right-panel
 * tab; tiles are clickable (preview) and draggable (drop into Composer /
 * MessageList as attachments).
 */
export function GalleryContent({ open, onPreviewImage }: GalleryContentProps) {
  const { t } = useTranslation();
  const active = useSession((s) => s.active);
  const gridRef = useRef<HTMLDivElement | null>(null);
  const [innerWidth, setInnerWidth] = useState(0);

  const images = useMemo(() => collectSessionGalleryImages(active), [active]);

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

  return (
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
  );
}
