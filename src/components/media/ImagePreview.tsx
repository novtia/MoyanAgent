import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { srcOf, api } from "../../api/tauri";
import { save } from "@tauri-apps/plugin-dialog";
import type { ImageRefAbs } from "../../types";

interface ImagePreviewProps {
  items: ImageRefAbs[];
  initialIndex: number;
  onClose: () => void;
}

const MIN_SCALE = 0.05;
const MAX_SCALE = 12;
const ZOOM_STEP = 1.25; // button click multiplier
const KEY_PAN = 48; // px per arrow key

type Toast = { kind: "info" | "error"; text: string; id: number } | null;

export function ImagePreview({ items, initialIndex, onClose }: ImagePreviewProps) {
  const { t } = useTranslation();

  const [index, setIndex] = useState(() =>
    items.length ? Math.max(0, Math.min(initialIndex, items.length - 1)) : 0,
  );

  useEffect(() => {
    if (items.length === 0) return;
    setIndex((i) => Math.max(0, Math.min(i, items.length - 1)));
  }, [items.length]);

  const clampedIndex = items.length ? Math.max(0, Math.min(index, items.length - 1)) : 0;
  const current = items[clampedIndex];
  const absPath = current?.abs_path ?? "";
  const mime = current?.mime;
  const imageId = current?.id;
  const galleryCount = items.length;
  const canGoPrev = galleryCount > 1 && clampedIndex > 0;
  const canGoNext = galleryCount > 1 && clampedIndex < galleryCount - 1;

  const [scale, setScale] = useState(1);
  const [tx, setTx] = useState(0);
  const [ty, setTy] = useState(0);
  const [animating, setAnimating] = useState(false);
  const [natural, setNatural] = useState<{ w: number; h: number } | null>(null);
  const [stageSize, setStageSize] = useState<{ w: number; h: number } | null>(null);
  const [imageReady, setImageReady] = useState(false);
  const [toast, setToast] = useState<Toast>(null);

  const canvasRef = useRef<HTMLDivElement>(null);
  const animTimer = useRef<number | null>(null);
  const initialFit = useRef(false);
  const drag = useRef<{
    active: boolean;
    moved: boolean;
    onImage: boolean;
    sx: number;
    sy: number;
    tx0: number;
    ty0: number;
    pointerId: number;
  } | null>(null);

  const fitScale = useMemo(() => {
    if (!natural || !stageSize) return 1;
    // Leave a bit of breathing room so the image never kisses the chrome
    const margin = 80;
    const aw = Math.max(64, stageSize.w - margin * 2);
    const ah = Math.max(64, stageSize.h - margin * 2);
    return Math.min(aw / natural.w, ah / natural.h, 1);
  }, [natural, stageSize]);

  // Observe canvas size for cursor-centered zoom and fit calculations
  useEffect(() => {
    if (!canvasRef.current) return;
    const el = canvasRef.current;
    const rect = el.getBoundingClientRect();
    setStageSize({ w: rect.width, h: rect.height });
    const ro = new ResizeObserver((entries) => {
      const cr = entries[0]?.contentRect;
      if (cr) setStageSize({ w: cr.width, h: cr.height });
    });
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  // Smooth animated transitions for button-driven changes (not wheel/drag)
  const flashAnim = useCallback(() => {
    setAnimating(true);
    if (animTimer.current) window.clearTimeout(animTimer.current);
    animTimer.current = window.setTimeout(() => setAnimating(false), 280);
  }, []);

  useEffect(() => {
    return () => {
      if (animTimer.current) window.clearTimeout(animTimer.current);
    };
  }, []);

  useEffect(() => {
    if (!absPath) return;
    initialFit.current = false;
    setImageReady(false);
    setNatural(null);
    setScale(1);
    setTx(0);
    setTy(0);
  }, [absPath]);

  // Snap to fit on first load + each time stage / natural changes before user has interacted
  useEffect(() => {
    if (!natural || !stageSize) return;
    if (initialFit.current) return;
    initialFit.current = true;
    setScale(fitScale);
    setTx(0);
    setTy(0);
  }, [natural, stageSize, fitScale]);

  const fit = useCallback(() => {
    flashAnim();
    setScale(fitScale);
    setTx(0);
    setTy(0);
  }, [fitScale, flashAnim]);

  const actualSize = useCallback(() => {
    flashAnim();
    setScale(1);
    setTx(0);
    setTy(0);
  }, [flashAnim]);

  const goPrev = useCallback(() => {
    if (!canGoPrev) return;
    flashAnim();
    setIndex((i) => Math.max(0, i - 1));
  }, [canGoPrev, flashAnim]);

  const goNext = useCallback(() => {
    if (!canGoNext) return;
    flashAnim();
    setIndex((i) => Math.min(items.length - 1, i + 1));
  }, [canGoNext, flashAnim, items.length]);

  const zoomBy = useCallback((factor: number, anchor?: { x: number; y: number }) => {
    setScale((s) => {
      const next = Math.max(MIN_SCALE, Math.min(MAX_SCALE, s * factor));
      if (next === s) return s;
      const k = next / s;
      const ax = anchor?.x ?? 0;
      const ay = anchor?.y ?? 0;
      // Keep the screen point at (ax, ay) -- relative to canvas center -- pinned during the zoom.
      setTx((v) => ax * (1 - k) + v * k);
      setTy((v) => ay * (1 - k) + v * k);
      return next;
    });
  }, []);

  const zoomIn = useCallback(() => {
    flashAnim();
    zoomBy(ZOOM_STEP);
  }, [flashAnim, zoomBy]);

  const zoomOut = useCallback(() => {
    flashAnim();
    zoomBy(1 / ZOOM_STEP);
  }, [flashAnim, zoomBy]);

  // Whether the current view matches one of the canonical zoom modes.
  const isFit = Math.abs(scale - fitScale) < 0.001 && tx === 0 && ty === 0;
  const isActual = Math.abs(scale - 1) < 0.001 && tx === 0 && ty === 0;

  // Display percent is the on-screen size relative to natural pixels (i.e. 100% = 1:1).
  const percent = Math.round(scale * 100);

  // Keyboard shortcuts -- registered once, depends on stable callbacks
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.defaultPrevented) return;
      const k = e.key;
      if (k === "Escape") {
        onClose();
        return;
      }
      if (k === "+" || k === "=") {
        e.preventDefault();
        zoomIn();
        return;
      }
      if (k === "-" || k === "_") {
        e.preventDefault();
        zoomOut();
        return;
      }
      if (k === "0" || k === "f" || k === "F") {
        e.preventDefault();
        fit();
        return;
      }
      if (k === "1") {
        e.preventDefault();
        actualSize();
        return;
      }
      if (galleryCount > 1 && k === "ArrowLeft" && canGoPrev) {
        e.preventDefault();
        goPrev();
        return;
      }
      if (galleryCount > 1 && k === "ArrowRight" && canGoNext) {
        e.preventDefault();
        goNext();
        return;
      }
      if (k === "ArrowLeft") {
        e.preventDefault();
        flashAnim();
        setTx((v) => v + KEY_PAN);
        return;
      }
      if (k === "ArrowRight") {
        e.preventDefault();
        flashAnim();
        setTx((v) => v - KEY_PAN);
        return;
      }
      if (k === "ArrowUp") {
        e.preventDefault();
        flashAnim();
        setTy((v) => v + KEY_PAN);
        return;
      }
      if (k === "ArrowDown") {
        e.preventDefault();
        flashAnim();
        setTy((v) => v - KEY_PAN);
        return;
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [
    onClose,
    zoomIn,
    zoomOut,
    fit,
    actualSize,
    flashAnim,
    galleryCount,
    canGoPrev,
    canGoNext,
    goPrev,
    goNext,
  ]);

  // React's onWheel is passive on many roots — preventDefault only works with { passive: false }.
  const onWheelNative = useCallback(
    (e: WheelEvent) => {
      e.preventDefault();
      const el = canvasRef.current;
      if (!el) return;
      const rect = el.getBoundingClientRect();
      const ax = e.clientX - rect.left - rect.width / 2;
      const ay = e.clientY - rect.top - rect.height / 2;
      const factor = Math.exp(-e.deltaY * 0.0015);
      zoomBy(factor, { x: ax, y: ay });
    },
    [zoomBy],
  );

  useEffect(() => {
    const el = canvasRef.current;
    if (!el) return;
    el.addEventListener("wheel", onWheelNative, { passive: false });
    return () => el.removeEventListener("wheel", onWheelNative);
  }, [onWheelNative]);

  // Hit-test the image's current bounding box (in canvas-local coords)
  const isPointOnImage = useCallback(
    (clientX: number, clientY: number): boolean => {
      const rect = canvasRef.current?.getBoundingClientRect();
      if (!rect || !natural) return false;
      const cx = clientX - rect.left - rect.width / 2;
      const cy = clientY - rect.top - rect.height / 2;
      const halfW = (natural.w * scale) / 2;
      const halfH = (natural.h * scale) / 2;
      return (
        cx >= tx - halfW && cx <= tx + halfW && cy >= ty - halfH && cy <= ty + halfH
      );
    },
    [natural, scale, tx, ty]
  );

  const onPointerDown = (e: React.PointerEvent<HTMLDivElement>) => {
    if (e.button !== 0) return;
    drag.current = {
      active: true,
      moved: false,
      onImage: isPointOnImage(e.clientX, e.clientY),
      sx: e.clientX,
      sy: e.clientY,
      tx0: tx,
      ty0: ty,
      pointerId: e.pointerId,
    };
    try {
      e.currentTarget.setPointerCapture(e.pointerId);
    } catch {
      /* Safari may reject capture; pan still works via window listeners */
    }
  };

  const onPointerMove = (e: React.PointerEvent<HTMLDivElement>) => {
    const d = drag.current;
    if (!d || !d.active) return;
    const dx = e.clientX - d.sx;
    const dy = e.clientY - d.sy;
    if (!d.moved && Math.hypot(dx, dy) > 3) d.moved = true;
    if (d.moved) {
      setTx(d.tx0 + dx);
      setTy(d.ty0 + dy);
    }
  };

  const finishDrag = (e: React.PointerEvent<HTMLDivElement>) => {
    const d = drag.current;
    drag.current = null;
    try {
      e.currentTarget.releasePointerCapture(e.pointerId);
    } catch {
      /* ignore — capture may have already lapsed */
    }
    if (d && !d.moved && !d.onImage) {
      // Bare click on the dim background → close (image clicks are ignored)
      onClose();
    }
  };

  const showToast = (kind: "info" | "error", text: string) => {
    setToast({ kind, text, id: Date.now() });
    window.setTimeout(() => {
      setToast((cur) => (cur && cur.text === text ? null : cur));
    }, 1700);
  };

  const downloadAs = async () => {
    if (!imageId) return;
    const ext = mime === "image/jpeg" ? "jpg" : mime === "image/webp" ? "webp" : "png";
    const dest = await save({
      defaultPath: `atelier-${Date.now()}.${ext}`,
      filters: [{ name: "Image", extensions: [ext] }],
    });
    if (!dest) return;
    await api.exportImage(imageId, dest as string);
  };

  const copyImage = async () => {
    try {
      const url = srcOf(absPath);
      const blob = await (await fetch(url)).blob();
      if (
        navigator.clipboard &&
        (window as unknown as { ClipboardItem?: typeof ClipboardItem }).ClipboardItem &&
        ["image/png", "image/jpeg", "image/webp"].includes(blob.type)
      ) {
        await navigator.clipboard.write([new ClipboardItem({ [blob.type]: blob })]);
        showToast("info", t("preview.copied"));
      } else {
        throw new Error("clipboard not supported");
      }
    } catch (e) {
      console.warn(e);
      showToast("error", t("preview.copyFailed"));
    }
  };

  const onImgLoad = (e: React.SyntheticEvent<HTMLImageElement>) => {
    const img = e.currentTarget;
    setNatural({ w: img.naturalWidth, h: img.naturalHeight });
    setImageReady(true);
  };

  const dragging = !!drag.current?.moved;

  if (!current || !absPath) {
    return null;
  }

  return (
    <div className="preview-lightbox" role="dialog" aria-modal="true" aria-label={t("preview.title")}>
      <div
        ref={canvasRef}
        className={`preview-canvas${dragging ? " is-grabbing" : ""}`}
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={finishDrag}
        onPointerCancel={finishDrag}
      >
        {!imageReady && <div className="preview-spinner" aria-hidden="true" />}
        <img
          key={imageId || absPath}
          src={srcOf(absPath)}
          alt=""
          className={`${imageReady ? "is-ready" : "is-loading"}${animating ? " with-transition" : ""}`}
          style={{
            transform: `translate(calc(-50% + ${tx}px), calc(-50% + ${ty}px)) scale(${scale})`,
          }}
          draggable={false}
          onLoad={onImgLoad}
        />
      </div>

      {galleryCount > 1 && (
        <>
          <button
            type="button"
            className="preview-nav preview-nav--prev"
            onClick={(e) => {
              e.stopPropagation();
              goPrev();
            }}
            disabled={!canGoPrev}
            title={t("preview.prevImage")}
            aria-label={t("preview.prevImage")}
          >
            <svg viewBox="0 0 16 16" fill="none" aria-hidden="true">
              <path
                d="M10 3L5 8l5 5"
                stroke="currentColor"
                strokeWidth="1.6"
                strokeLinecap="round"
                strokeLinejoin="round"
              />
            </svg>
          </button>
          <button
            type="button"
            className="preview-nav preview-nav--next"
            onClick={(e) => {
              e.stopPropagation();
              goNext();
            }}
            disabled={!canGoNext}
            title={t("preview.nextImage")}
            aria-label={t("preview.nextImage")}
          >
            <svg viewBox="0 0 16 16" fill="none" aria-hidden="true">
              <path
                d="M6 3l5 5-5 5"
                stroke="currentColor"
                strokeWidth="1.6"
                strokeLinecap="round"
                strokeLinejoin="round"
              />
            </svg>
          </button>
        </>
      )}

      <div className="preview-topbar">
        <div className="preview-info" aria-live="polite">
          {natural ? (
            <>
              <span>
                <b>{natural.w}</b>
                <span className="preview-info-dim"> × </span>
                <b>{natural.h}</b>
              </span>
              <span className="preview-info-dot" />
              <span>{percent}%</span>
              {galleryCount > 1 && (
                <>
                  <span className="preview-info-dot" />
                  <span className="preview-info-count">
                    {clampedIndex + 1}/{galleryCount}
                  </span>
                </>
              )}
            </>
          ) : (
            <span className="preview-info-dim">…</span>
          )}
        </div>
        <button
          className="preview-icon-btn is-chrome is-close"
          onClick={onClose}
          type="button"
          title={t("preview.close")}
          aria-label={t("preview.close")}
        >
          <svg viewBox="0 0 16 16" fill="none" aria-hidden="true">
            <path
              d="M4 4l8 8M12 4l-8 8"
              stroke="currentColor"
              strokeWidth="1.6"
              strokeLinecap="round"
            />
          </svg>
        </button>
      </div>

      <div className="preview-toolbar" role="toolbar" aria-label={t("preview.title")}>
        <button
          className="preview-icon-btn"
          onClick={zoomOut}
          type="button"
          title={t("preview.zoomOut")}
          aria-label={t("preview.zoomOut")}
          disabled={scale <= MIN_SCALE + 1e-4}
        >
          <svg viewBox="0 0 16 16" fill="none" aria-hidden="true">
            <path d="M3.5 8h9" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" />
          </svg>
        </button>

        <button
          className="preview-zoom"
          onClick={fit}
          type="button"
          title={t("preview.fit")}
          aria-label={t("preview.fit")}
        >
          {percent}%
        </button>

        <button
          className="preview-icon-btn"
          onClick={zoomIn}
          type="button"
          title={t("preview.zoomIn")}
          aria-label={t("preview.zoomIn")}
          disabled={scale >= MAX_SCALE - 1e-4}
        >
          <svg viewBox="0 0 16 16" fill="none" aria-hidden="true">
            <path
              d="M8 3.5v9M3.5 8h9"
              stroke="currentColor"
              strokeWidth="1.6"
              strokeLinecap="round"
            />
          </svg>
        </button>

        <span className="preview-toolbar-divider" aria-hidden="true" />

        <div className="preview-seg" role="group">
          <button
            type="button"
            className={`preview-seg-btn${isFit ? " is-active" : ""}`}
            onClick={fit}
            title={t("preview.fit")}
            aria-pressed={isFit}
          >
            {t("preview.fit")}
          </button>
          <button
            type="button"
            className={`preview-seg-btn${isActual ? " is-active" : ""}`}
            onClick={actualSize}
            title={t("preview.actualSize")}
            aria-pressed={isActual}
          >
            1:1
          </button>
        </div>

        <span className="preview-toolbar-divider" aria-hidden="true" />

        <button
          className="preview-icon-btn"
          onClick={copyImage}
          type="button"
          title={t("preview.copyImage")}
          aria-label={t("preview.copyImage")}
        >
          <svg viewBox="0 0 16 16" fill="none" aria-hidden="true">
            <rect
              x="5"
              y="5"
              width="8"
              height="8"
              rx="1.6"
              stroke="currentColor"
              strokeWidth="1.4"
            />
            <path
              d="M11 5V3.6A1.6 1.6 0 0 0 9.4 2H4.6A1.6 1.6 0 0 0 3 3.6v4.8A1.6 1.6 0 0 0 4.6 10H5"
              stroke="currentColor"
              strokeWidth="1.4"
              strokeLinecap="round"
            />
          </svg>
        </button>

        {imageId && (
          <button
            className="preview-icon-btn is-primary"
            onClick={downloadAs}
            type="button"
            title={t("preview.saveAs")}
            aria-label={t("preview.saveAs")}
          >
            <svg viewBox="0 0 16 16" fill="none" aria-hidden="true">
              <path
                d="M8 2.5v8M4.6 7.4 8 10.8l3.4-3.4"
                stroke="currentColor"
                strokeWidth="1.6"
                strokeLinecap="round"
                strokeLinejoin="round"
              />
              <path
                d="M3 12.5v.4A1.6 1.6 0 0 0 4.6 14.5h6.8a1.6 1.6 0 0 0 1.6-1.6v-.4"
                stroke="currentColor"
                strokeWidth="1.6"
                strokeLinecap="round"
              />
            </svg>
          </button>
        )}
      </div>

      {toast && (
        <div
          key={toast.id}
          className={`preview-toast${toast.kind === "error" ? " is-error" : ""}`}
          role="status"
        >
          {toast.text}
        </div>
      )}
    </div>
  );
}
