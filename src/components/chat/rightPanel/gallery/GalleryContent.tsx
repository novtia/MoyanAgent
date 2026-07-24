import { useLayoutEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { useSession } from "../../../../store/session";
import { srcOf } from "../../../../api/tauri";
import { collectSessionGalleryMedia } from "../../../../sessionGallery";
import type { ImageRefAbs } from "../../../../types";
import { ATELIER_DRAG_TYPE } from "./constants";
import type { GalleryContentProps } from "./types";
import { useMasonryLayout } from "./useMasonryLayout";

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

  const media = useMemo(() => collectSessionGalleryMedia(active), [active]);

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

  const layout = useMasonryLayout(media, innerWidth);

  const onTileDragStart = (e: React.DragEvent<HTMLButtonElement>, img: ImageRefAbs) => {
    if (img.mime.startsWith("video/")) {
      e.preventDefault();
      return;
    }
    if (!e.dataTransfer) return;
    e.dataTransfer.effectAllowed = "copy";
    const payload = JSON.stringify({ id: img.id, abs_path: img.abs_path, mime: img.mime });
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
      {media.length === 0 ? (
        <div className="chat-gallery-empty">{t("chat.galleryEmpty")}</div>
      ) : (
        <div className="chat-gallery-canvas" style={{ height: layout.total }}>
          {layout.items.map(({ img, x, y, w, h }) => (
            <button
              key={img.id}
              type="button"
              className={`chat-gallery-tile role-${img.role} ${
                img.mime.startsWith("video/") ? "is-video" : ""
              }`}
              style={{ transform: `translate(${x}px, ${y}px)`, width: w, height: h }}
              onClick={() => onPreviewImage(img)}
              title={img.source_url || img.rel_path}
              tabIndex={open ? 0 : -1}
              draggable={!img.mime.startsWith("video/")}
              onDragStart={(e) => onTileDragStart(e, img)}
            >
              {img.mime.startsWith("video/") ? (
                <>
                  <video
                    src={srcOf(img.abs_path)}
                    muted
                    playsInline
                    preload="metadata"
                  />
                  <span className="chat-gallery-video-badge">
                    <span aria-hidden>▶</span>
                    {t("chat.galleryVideo")}
                  </span>
                </>
              ) : (
                <img
                  src={srcOf(img.thumb_abs_path || img.abs_path)}
                  alt=""
                  loading="lazy"
                  decoding="async"
                  draggable={false}
                />
              )}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}
