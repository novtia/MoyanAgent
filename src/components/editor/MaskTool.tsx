import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { api, srcOf } from "../../api/tauri";
import type { AttachmentDraft } from "../../types";

interface Props {
  target: AttachmentDraft;
  busy: boolean;
  setBusy: (b: boolean) => void;
  onApplied: (newDraft: AttachmentDraft) => void;
}

/**
 * Mask tool. The user paints over regions to be removed (made transparent).
 * Layers: <img> as the base + a transparent <canvas> overlay accepting paint
 * input (translucent red brush). On apply we build a black/white luma mask
 * (white = keep, black = remove) at the image's natural pixel resolution and
 * send it base64-encoded to Rust.
 */
export function MaskTool({ target, busy, setBusy, onApplied }: Props) {
  const { t } = useTranslation();
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const imgRef = useRef<HTMLImageElement | null>(null);
  const drawing = useRef(false);
  const lastPos = useRef<{ x: number; y: number } | null>(null);
  const [brush, setBrush] = useState<number>(36);
  const [mode, setMode] = useState<"erase" | "restore">("erase");
  const [imgLoaded, setImgLoaded] = useState(false);

  // Keep the overlay canvas the exact pixel size of the rendered <img>. When
  // the viewport (or image fit size) changes, preserve the user's strokes by
  // copying through a temporary canvas before resizing.
  useEffect(() => {
    if (!imgLoaded) return;
    const img = imgRef.current;
    const canvas = canvasRef.current;
    if (!img || !canvas) return;

    const sync = () => {
      const rect = img.getBoundingClientRect();
      const w = Math.max(1, Math.round(rect.width));
      const h = Math.max(1, Math.round(rect.height));
      if (canvas.width === w && canvas.height === h) return;

      // Snapshot existing strokes, then resize, then redraw scaled.
      const old = document.createElement("canvas");
      old.width = canvas.width || w;
      old.height = canvas.height || h;
      if (canvas.width && canvas.height) {
        old.getContext("2d")!.drawImage(canvas, 0, 0);
      }
      canvas.width = w;
      canvas.height = h;
      canvas.style.width = `${w}px`;
      canvas.style.height = `${h}px`;
      const ctx = canvas.getContext("2d")!;
      ctx.clearRect(0, 0, w, h);
      if (old.width && old.height) {
        ctx.drawImage(old, 0, 0, w, h);
      }
    };

    sync();
    const ro = new ResizeObserver(sync);
    ro.observe(img);
    return () => ro.disconnect();
  }, [imgLoaded]);

  const getPos = (e: React.PointerEvent) => {
    const c = canvasRef.current!;
    const r = c.getBoundingClientRect();
    return { x: e.clientX - r.left, y: e.clientY - r.top };
  };

  const drawSegment = (a: { x: number; y: number }, b: { x: number; y: number }) => {
    const ctx = canvasRef.current!.getContext("2d")!;
    ctx.lineCap = "round";
    ctx.lineJoin = "round";
    ctx.lineWidth = brush;
    if (mode === "erase") {
      ctx.globalCompositeOperation = "source-over";
      ctx.strokeStyle = "rgba(225, 29, 72, 0.6)";
    } else {
      ctx.globalCompositeOperation = "destination-out";
      ctx.strokeStyle = "rgba(0,0,0,1)";
    }
    ctx.beginPath();
    ctx.moveTo(a.x, a.y);
    ctx.lineTo(b.x, b.y);
    ctx.stroke();
  };

  const onDown = (e: React.PointerEvent) => {
    if (busy) return;
    drawing.current = true;
    lastPos.current = getPos(e);
    drawSegment(lastPos.current, lastPos.current);
    try {
      (e.target as HTMLElement).setPointerCapture(e.pointerId);
    } catch {
      /* not all browsers allow pointer capture on canvas; ignore */
    }
  };
  const onMove = (e: React.PointerEvent) => {
    if (!drawing.current) return;
    const p = getPos(e);
    if (lastPos.current) drawSegment(lastPos.current, p);
    lastPos.current = p;
  };
  const onUp = (e?: React.PointerEvent) => {
    drawing.current = false;
    lastPos.current = null;
    if (e) {
      try {
        (e.target as HTMLElement).releasePointerCapture(e.pointerId);
      } catch {
        /* ignore */
      }
    }
  };

  const reset = () => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    canvas.getContext("2d")!.clearRect(0, 0, canvas.width, canvas.height);
  };

  const apply = async () => {
    const overlay = canvasRef.current;
    const img = imgRef.current;
    if (!overlay || !img) return;
    const naturalW = img.naturalWidth;
    const naturalH = img.naturalHeight;
    if (!naturalW || !naturalH) return;

    // Build a luma mask at original image size. White = keep, Black = remove.
    const out = document.createElement("canvas");
    out.width = naturalW;
    out.height = naturalH;
    const octx = out.getContext("2d")!;
    octx.fillStyle = "#ffffff";
    octx.fillRect(0, 0, naturalW, naturalH);

    // Overlay was drawn in display pixels; threshold alpha to opaque B/W.
    const overlayCtx = overlay.getContext("2d")!;
    const overlayData = overlayCtx.getImageData(0, 0, overlay.width, overlay.height);
    const tmp = document.createElement("canvas");
    tmp.width = overlay.width;
    tmp.height = overlay.height;
    const tctx = tmp.getContext("2d")!;
    const tmpData = tctx.createImageData(overlay.width, overlay.height);
    for (let i = 0; i < overlayData.data.length; i += 4) {
      const a = overlayData.data[i + 3];
      if (a > 24) {
        tmpData.data[i] = 0;
        tmpData.data[i + 1] = 0;
        tmpData.data[i + 2] = 0;
        tmpData.data[i + 3] = 255;
      } else {
        tmpData.data[i] = 255;
        tmpData.data[i + 1] = 255;
        tmpData.data[i + 2] = 255;
        tmpData.data[i + 3] = 255;
      }
    }
    tctx.putImageData(tmpData, 0, 0);

    octx.imageSmoothingEnabled = true;
    octx.drawImage(tmp, 0, 0, naturalW, naturalH);

    const dataUrl = out.toDataURL("image/png");
    const base64 = dataUrl.split(",")[1] || "";
    if (!base64) {
      alert(t("editor.mask.generateError"));
      return;
    }
    setBusy(true);
    try {
      const r = await api.editImage(target.image_id, {
        type: "apply_mask",
        mask_png_base64: base64,
      });
      onApplied({
        image_id: r.id,
        rel_path: r.rel_path,
        thumb_rel_path: r.thumb_rel_path,
        abs_path: r.abs_path,
        thumb_abs_path: r.thumb_abs_path,
        mime: r.mime,
        width: r.width,
        height: r.height,
        bytes: r.bytes,
      });
    } catch (e) {
      console.error(e);
      alert(`${t("editor.mask.applyError")}: ${e}`);
    } finally {
      setBusy(false);
    }
  };

  return (
    <>
      <div className="editor-stage-image">
        <div className="editor-mask-wrap">
          <img
            ref={imgRef}
            src={srcOf(target.abs_path)}
            alt=""
            onLoad={() => setImgLoaded(true)}
            draggable={false}
            style={{
              maxWidth: "calc(100vw - 112px)",
              maxHeight: "calc(100vh - 220px)",
            }}
          />
          <canvas
            ref={canvasRef}
            className="mask-canvas"
            onPointerDown={onDown}
            onPointerMove={onMove}
            onPointerUp={onUp}
            onPointerLeave={() => onUp()}
            onPointerCancel={onUp}
          />
        </div>
      </div>

      <div className="editor-hint">{t("editor.mask.hint")}</div>

      <div className="editor-toolbar" role="toolbar" aria-label={t("editor.tabMask")}>
        <div className="editor-seg" role="group" aria-label="paint mode">
          <button
            type="button"
            className={`editor-chip${mode === "erase" ? " is-active" : ""}`}
            onClick={() => setMode("erase")}
            disabled={busy}
            aria-pressed={mode === "erase"}
          >
            {t("editor.mask.erase")}
          </button>
          <button
            type="button"
            className={`editor-chip${mode === "restore" ? " is-active" : ""}`}
            onClick={() => setMode("restore")}
            disabled={busy}
            aria-pressed={mode === "restore"}
          >
            {t("editor.mask.restore")}
          </button>
        </div>

        <span className="editor-toolbar-divider" aria-hidden="true" />

        <span className="editor-toolbar-label">{t("editor.mask.brush")}</span>
        <input
          type="range"
          className="editor-range"
          min={4}
          max={120}
          step={1}
          value={brush}
          onChange={(e) => setBrush(Number(e.target.value))}
          disabled={busy}
          aria-label={t("editor.mask.brush")}
        />

        <span className="editor-toolbar-divider" aria-hidden="true" />

        <div className="editor-toolbar-actions">
          <button
            className="editor-chip"
            type="button"
            onClick={reset}
            disabled={busy}
          >
            {t("editor.mask.clear")}
          </button>

          <button
            className={`editor-apply${busy ? " is-busy" : ""}`}
            disabled={busy}
            onClick={apply}
            type="button"
          >
            {t("editor.mask.apply")}
          </button>
        </div>
      </div>
    </>
  );
}
