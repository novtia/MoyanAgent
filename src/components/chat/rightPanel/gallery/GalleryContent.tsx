import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type KeyboardEvent as ReactKeyboardEvent,
  type MouseEvent as ReactMouseEvent,
} from "react";
import { useTranslation } from "react-i18next";
import { save } from "@tauri-apps/plugin-dialog";
import { api, srcOf } from "../../../../api/tauri";
import { collectSessionGalleryMedia } from "../../../../sessionGallery";
import { useSession } from "../../../../store/session";
import type { ImageRefAbs } from "../../../../types";
import { openContextMenu } from "../../../context-menu";
import { dialog } from "../../../ui/Dialog";
import { toast } from "../../../ui/Toast";
import { ATELIER_DRAG_TYPE } from "./constants";
import {
  isModifierClick,
  resolveBulkIds,
  selectRange,
  toggleId,
} from "./selection";
import type { GalleryContentProps } from "./types";
import { useMasonryLayout } from "./useMasonryLayout";

function extForMime(mime: string): string {
  if (mime === "image/jpeg") return "jpg";
  if (mime === "image/webp") return "webp";
  if (mime === "image/gif") return "gif";
  if (mime === "image/png") return "png";
  if (mime === "video/mp4") return "mp4";
  if (mime === "video/webm") return "webm";
  if (mime === "video/quicktime") return "mov";
  if (mime.startsWith("video/")) return "mp4";
  if (mime.startsWith("image/")) return "png";
  return "bin";
}

/**
 * Masonry grid of the current session's images. Rendered inside a right-panel
 * tab; tiles support Ctrl/Shift multi-select, context menu (preview / pack
 * download / delete), click-to-preview, and drag into Composer / MessageList.
 */
export function GalleryContent({ open, onPreviewImage }: GalleryContentProps) {
  const { t } = useTranslation();
  const active = useSession((s) => s.active);
  const reloadActiveSession = useSession((s) => s.reloadActiveSession);
  const gridRef = useRef<HTMLDivElement | null>(null);
  const [innerWidth, setInnerWidth] = useState(0);
  const [selectedIds, setSelectedIds] = useState<string[]>([]);
  const [anchorId, setAnchorId] = useState<string | null>(null);

  const media = useMemo(() => collectSessionGalleryMedia(active), [active]);
  const mediaIds = useMemo(() => media.map((m) => m.id), [media]);
  const mediaById = useMemo(() => {
    const map = new Map<string, ImageRefAbs>();
    for (const m of media) map.set(m.id, m);
    return map;
  }, [media]);

  // Drop selection entries that no longer exist in the gallery.
  useEffect(() => {
    const alive = new Set(mediaIds);
    setSelectedIds((prev) => {
      const next = prev.filter((id) => alive.has(id));
      return next.length === prev.length ? prev : next;
    });
    setAnchorId((prev) => (prev && alive.has(prev) ? prev : null));
  }, [mediaIds]);

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

  const clearSelection = useCallback(() => {
    setSelectedIds([]);
    setAnchorId(null);
  }, []);

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

  const downloadBulk = useCallback(
    async (ids: string[]) => {
      if (ids.length === 0) return;
      try {
        if (ids.length === 1) {
          const img = mediaById.get(ids[0]);
          if (!img) return;
          const ext = extForMime(img.mime);
          const isVideo = img.mime.startsWith("video/");
          const dest = await save({
            defaultPath: `atelier-${Date.now()}.${ext}`,
            filters: [
              {
                name: isVideo ? "Video" : "Image",
                extensions: [ext],
              },
            ],
          });
          if (!dest) return;
          await api.exportMedia(img.id, dest as string);
        } else {
          const dest = await save({
            defaultPath: `atelier-gallery-${Date.now()}.zip`,
            filters: [{ name: "ZIP", extensions: ["zip"] }],
          });
          if (!dest) return;
          await api.exportMediaZip(ids, dest as string);
        }
        toast.success(
          ids.length > 1
            ? t("chat.galleryDownloadDone", { count: ids.length })
            : t("chat.galleryDownloadOneDone"),
        );
      } catch (err) {
        toast.error(t("chat.galleryDownloadFailed"), { description: String(err) });
      }
    },
    [mediaById, t],
  );

  const deleteBulk = useCallback(
    async (ids: string[]) => {
      if (ids.length === 0) return;
      const message =
        ids.length > 1
          ? t("chat.galleryDeleteSelectedConfirm", { count: ids.length })
          : t("chat.galleryDeleteOneConfirm");
      const ok = await dialog.confirm(message, {
        type: "danger",
        confirmLabel: t("common.delete"),
        title: t("chat.galleryDelete"),
      });
      if (!ok) return;
      try {
        await api.deleteMedia(ids);
        await reloadActiveSession();
        clearSelection();
        toast.success(
          ids.length > 1
            ? t("chat.galleryDeleteDone", { count: ids.length })
            : t("chat.galleryDeleteOneDone"),
        );
      } catch (err) {
        toast.error(t("chat.galleryDeleteFailed"), { description: String(err) });
      }
    },
    [clearSelection, reloadActiveSession, t],
  );

  const onTileClick = useCallback(
    (e: ReactMouseEvent, img: ImageRefAbs) => {
      e.stopPropagation();
      if (e.shiftKey) {
        e.preventDefault();
        const range = selectRange(mediaIds, anchorId, img.id);
        setSelectedIds(range);
        if (!anchorId) setAnchorId(img.id);
        return;
      }
      if (isModifierClick(e)) {
        e.preventDefault();
        setSelectedIds((prev) => toggleId(prev, img.id));
        setAnchorId(img.id);
        return;
      }
      setSelectedIds([img.id]);
      setAnchorId(img.id);
      onPreviewImage(img);
    },
    [anchorId, mediaIds, onPreviewImage],
  );

  const onTileContextMenu = useCallback(
    (e: ReactMouseEvent, img: ImageRefAbs) => {
      e.preventDefault();
      e.stopPropagation();

      let nextSelected = selectedIds;
      if (!selectedIds.includes(img.id)) {
        nextSelected = [img.id];
        setSelectedIds(nextSelected);
        setAnchorId(img.id);
      }
      const bulk = resolveBulkIds(img.id, nextSelected);
      const single = bulk.length === 1 ? mediaById.get(bulk[0]) : undefined;

      openContextMenu(
        e,
        [
          {
            id: "preview",
            label: t("chat.galleryPreview"),
            disabled: bulk.length !== 1 || !single,
            onSelect: () => {
              if (single) onPreviewImage(single);
            },
          },
          {
            id: "download",
            label:
              bulk.length > 1
                ? t("chat.galleryDownloadPack", { count: bulk.length })
                : t("chat.galleryDownload"),
            onSelect: () => void downloadBulk(bulk),
          },
          { type: "separator" },
          {
            id: "delete",
            label:
              bulk.length > 1
                ? t("chat.galleryDeleteSelected", { count: bulk.length })
                : t("chat.galleryDelete"),
            danger: true,
            onSelect: () => void deleteBulk(bulk),
          },
        ],
        { menuId: "session-gallery" },
      );
    },
    [deleteBulk, downloadBulk, mediaById, onPreviewImage, selectedIds, t],
  );

  const onGridClick = useCallback(
    (e: ReactMouseEvent) => {
      if (e.target === e.currentTarget || (e.target as HTMLElement).classList.contains("chat-gallery-canvas")) {
        clearSelection();
      }
    },
    [clearSelection],
  );

  const onKeyDown = useCallback(
    (e: ReactKeyboardEvent) => {
      if (e.key === "Escape" && selectedIds.length > 0) {
        e.stopPropagation();
        clearSelection();
      }
    },
    [clearSelection, selectedIds.length],
  );

  return (
    <div
      className="chat-gallery-grid"
      ref={gridRef}
      onClick={onGridClick}
      onKeyDown={onKeyDown}
      tabIndex={open ? 0 : -1}
    >
      {media.length === 0 ? (
        <div className="chat-gallery-empty">{t("chat.galleryEmpty")}</div>
      ) : (
        <div className="chat-gallery-canvas" style={{ height: layout.total }}>
          {layout.items.map(({ img, x, y, w, h }) => {
            const selected = selectedIds.includes(img.id);
            return (
              <button
                key={img.id}
                type="button"
                className={`chat-gallery-tile role-${img.role} ${
                  img.mime.startsWith("video/") ? "is-video" : ""
                }${selected ? " is-selected" : ""}`}
                style={{ transform: `translate(${x}px, ${y}px)`, width: w, height: h }}
                onClick={(e) => onTileClick(e, img)}
                onContextMenu={(e) => onTileContextMenu(e, img)}
                title={img.source_url || img.rel_path}
                tabIndex={open ? 0 : -1}
                draggable={!img.mime.startsWith("video/")}
                onDragStart={(e) => onTileDragStart(e, img)}
                aria-selected={selected}
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
            );
          })}
        </div>
      )}
    </div>
  );
}
