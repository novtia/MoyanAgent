import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { api, srcOf } from "../../api/tauri";
import type { AttachmentDraft, ImageRefAbs } from "../../types";

interface Props {
  target: AttachmentDraft;
  busy: boolean;
  setBusy: (b: boolean) => void;
  onApplied: (newDraft: AttachmentDraft) => void;
}

/**
 * A pending transform op — we accumulate these locally and only flush them
 * to the Rust backend when the user clicks "Apply". This makes rotation and
 * flips feel instant (CSS transforms) instead of doing a full
 * decode-rotate-encode-save round-trip on every click.
 */
type TOp =
  | { type: "rotate"; deg: 90 | 180 | 270 }
  | { type: "flip"; horizontal: boolean };

/**
 * Canonical normalised state. The 8-element dihedral group of a rectangle's
 * orientations can always be written as `R(θ) ∘ FlipX^f`, with θ in
 * {0,90,180,270} and f in {0,1}. We collapse the user's op stack into this
 * form for both the CSS preview and the minimal backend op sequence.
 */
type Norm = { rotation: 0 | 90 | 180 | 270; flipped: boolean };

function applyOp(s: Norm, op: TOp): Norm {
  let { rotation, flipped } = s;
  if (op.type === "rotate") {
    // M' = R(α) · M = R(α) · R(θ) · ScaleX^f = R(α+θ) · ScaleX^f
    rotation = (((rotation + op.deg) % 360 + 360) % 360) as Norm["rotation"];
    return { rotation, flipped };
  }
  if (op.horizontal) {
    // ScaleX · R(θ) · ScaleX^f
    //   = R(-θ) · ScaleX · ScaleX^f
    //   = R(-θ) · ScaleX^(1 - f)
    rotation = (((360 - rotation) % 360 + 360) % 360) as Norm["rotation"];
    return { rotation, flipped: !flipped };
  }
  // Vertical flip: ScaleY = R(180) · ScaleX
  // ScaleY · M = R(180) · R(-θ) · ScaleX^(1 - f) = R(180 - θ) · ScaleX^(1 - f)
  rotation = (((180 - rotation) % 360 + 360) % 360) as Norm["rotation"];
  return { rotation, flipped: !flipped };
}

const IDENTITY: Norm = { rotation: 0, flipped: false };

function normalize(ops: TOp[]): Norm {
  return ops.reduce(applyOp, IDENTITY);
}

function toDraft(r: ImageRefAbs): AttachmentDraft {
  return {
    image_id: r.id,
    rel_path: r.rel_path,
    thumb_rel_path: r.thumb_rel_path,
    abs_path: r.abs_path,
    thumb_abs_path: r.thumb_abs_path,
    mime: r.mime,
    width: r.width,
    height: r.height,
    bytes: r.bytes,
  };
}

// Curved-arrow rotate icon, mirrored via CSS transform for the CCW variant.
function RotateIcon({ ccw = false }: { ccw?: boolean }) {
  return (
    <svg
      viewBox="0 0 16 16"
      fill="none"
      aria-hidden="true"
      style={ccw ? { transform: "scaleX(-1)" } : undefined}
    >
      <path
        d="M3 8a5 5 0 1 1 1.46 3.54"
        stroke="currentColor"
        strokeWidth="1.5"
        strokeLinecap="round"
      />
      <path
        d="M3 4.6V8h3.4"
        stroke="currentColor"
        strokeWidth="1.5"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}

function FlipIcon({ vertical = false }: { vertical?: boolean }) {
  return (
    <svg
      viewBox="0 0 16 16"
      fill="none"
      aria-hidden="true"
      style={vertical ? { transform: "rotate(90deg)" } : undefined}
    >
      <path
        d="M8 2v12"
        stroke="currentColor"
        strokeWidth="1.4"
        strokeLinecap="round"
        strokeDasharray="1.5 1.6"
      />
      <path
        d="M3 5l-1 3 1 3z"
        fill="currentColor"
        stroke="currentColor"
        strokeWidth="1"
        strokeLinejoin="round"
      />
      <path
        d="M13 5l1 3-1 3z"
        fill="currentColor"
        stroke="currentColor"
        strokeWidth="1"
        strokeLinejoin="round"
      />
    </svg>
  );
}

export function TransformTool({ target, busy, setBusy, onApplied }: Props) {
  const { t } = useTranslation();
  const [ops, setOps] = useState<TOp[]>([]);
  const [resizeW, setResizeW] = useState<number>(target.width || 1024);
  const [resizeH, setResizeH] = useState<number>(target.height || 1024);
  const [imgNatural, setImgNatural] = useState<{ w: number; h: number } | null>(
    null
  );
  const stageRef = useRef<HTMLDivElement>(null);
  const [stageBox, setStageBox] = useState<{ w: number; h: number }>({ w: 0, h: 0 });

  // After Apply succeeds the parent swaps in a fresh `target` — reset all
  // local pending state so the user starts clean on the new image.
  useEffect(() => {
    setOps([]);
    setResizeW(target.width || 1024);
    setResizeH(target.height || 1024);
    setImgNatural(null);
  }, [target.image_id, target.width, target.height]);

  // Track stage size so we can dynamically size the <img> to fit the rotated
  // bounding box without overflow. Without this, rotating a wide image 90°
  // would clip against the stage borders.
  useEffect(() => {
    const el = stageRef.current;
    if (!el) return;
    const rect = el.getBoundingClientRect();
    setStageBox({ w: rect.width, h: rect.height });
    const ro = new ResizeObserver((entries) => {
      const cr = entries[0]?.contentRect;
      if (cr) setStageBox({ w: cr.width, h: cr.height });
    });
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  const norm = useMemo(() => normalize(ops), [ops]);
  const cssTransform = useMemo(() => {
    if (norm.rotation === 0 && !norm.flipped) return undefined;
    const parts: string[] = [];
    if (norm.rotation !== 0) parts.push(`rotate(${norm.rotation}deg)`);
    if (norm.flipped) parts.push("scaleX(-1)");
    return parts.join(" ");
  }, [norm]);

  const naturalW = imgNatural?.w ?? target.width ?? 0;
  const naturalH = imgNatural?.h ?? target.height ?? 0;
  const isSideways = norm.rotation === 90 || norm.rotation === 270;
  const effW = isSideways ? naturalH : naturalW;
  const effH = isSideways ? naturalW : naturalH;
  // Fit the rotated visual into the stage.
  const fitScale =
    naturalW > 0 && naturalH > 0 && stageBox.w > 0 && stageBox.h > 0
      ? Math.min(stageBox.w / effW, stageBox.h / effH, 1)
      : 0;
  const displayW = effW * fitScale;
  const displayH = effH * fitScale;
  // Sizing the <img> element BEFORE rotation: when the final rendered visual
  // is sideways the element itself needs to be sized "the other way".
  const imgPxW = isSideways ? displayH : displayW;
  const imgPxH = isSideways ? displayW : displayH;

  const resizeChanged =
    resizeW > 0 &&
    resizeH > 0 &&
    (resizeW !== (target.width ?? 0) || resizeH !== (target.height ?? 0));
  const hasOpsEffect = norm.rotation !== 0 || norm.flipped;
  const hasUndoable = ops.length > 0 || resizeChanged;
  const hasPendingApply = hasOpsEffect || resizeChanged;

  const addOp = (op: TOp) => setOps((prev) => [...prev, op]);
  const reset = () => {
    setOps([]);
    setResizeW(target.width || 1024);
    setResizeH(target.height || 1024);
  };

  const apply = async () => {
    if (!hasPendingApply) return;
    setBusy(true);
    let curId = target.image_id;
    let last: ImageRefAbs | null = null;
    try {
      // From the math above, baking `R(θ) · ScaleX^f` onto the pixels means
      // applying flipH first (if any), then a single rotate. We collapse the
      // user's whole op stack into at most 1 flip + 1 rotate + 1 resize.
      if (norm.flipped) {
        last = await api.editImage(curId, { type: "flip", horizontal: true });
        curId = last.id;
      }
      if (norm.rotation !== 0) {
        last = await api.editImage(curId, {
          type: "rotate",
          degrees: norm.rotation,
        });
        curId = last.id;
      }
      if (resizeChanged) {
        last = await api.editImage(curId, {
          type: "resize",
          width: resizeW,
          height: resizeH,
        });
      }
      if (last) {
        onApplied(toDraft(last));
        setOps([]);
      }
    } catch (e) {
      console.error(e);
      alert(`${t("editor.transform.applyError")}: ${e}`);
    } finally {
      setBusy(false);
    }
  };

  const onImgLoad = (e: React.SyntheticEvent<HTMLImageElement>) => {
    const img = e.currentTarget;
    setImgNatural({ w: img.naturalWidth, h: img.naturalHeight });
  };

  return (
    <>
      <div className="editor-stage-image" ref={stageRef}>
        <img
          src={srcOf(target.abs_path)}
          alt=""
          draggable={false}
          onLoad={onImgLoad}
          style={{
            width: imgPxW > 0 ? `${imgPxW}px` : "auto",
            height: imgPxH > 0 ? `${imgPxH}px` : "auto",
            maxWidth: "100%",
            maxHeight: "100%",
            transform: cssTransform,
            transition:
              "transform 0.32s var(--ease-pop), width 0.32s var(--ease-pop), height 0.32s var(--ease-pop)",
            willChange: "transform",
          }}
        />
      </div>

      <div className="editor-toolbar" role="toolbar" aria-label={t("editor.tabTransform")}>
        <div className="editor-seg" role="group" aria-label="rotate">
          <button
            className="editor-chip"
            disabled={busy}
            type="button"
            title={t("editor.transform.rotate90")}
            onClick={() => addOp({ type: "rotate", deg: 90 })}
          >
            <RotateIcon /> 90°
          </button>
          <button
            className="editor-chip"
            disabled={busy}
            type="button"
            title={t("editor.transform.rotate180")}
            onClick={() => addOp({ type: "rotate", deg: 180 })}
          >
            <RotateIcon /> 180°
          </button>
          <button
            className="editor-chip"
            disabled={busy}
            type="button"
            title={t("editor.transform.rotate270")}
            onClick={() => addOp({ type: "rotate", deg: 270 })}
          >
            <RotateIcon ccw /> 90°
          </button>
        </div>

        <span className="editor-toolbar-divider" aria-hidden="true" />

        <button
          className="editor-chip is-icon-only"
          disabled={busy}
          type="button"
          title={t("editor.transform.flipH")}
          aria-label={t("editor.transform.flipH")}
          onClick={() => addOp({ type: "flip", horizontal: true })}
        >
          <FlipIcon />
        </button>
        <button
          className="editor-chip is-icon-only"
          disabled={busy}
          type="button"
          title={t("editor.transform.flipV")}
          aria-label={t("editor.transform.flipV")}
          onClick={() => addOp({ type: "flip", horizontal: false })}
        >
          <FlipIcon vertical />
        </button>

        <span className="editor-toolbar-divider" aria-hidden="true" />

        <div
          className={`editor-resolution${busy ? " is-disabled" : ""}`}
          role="group"
          aria-label={`${t("editor.transform.width")} × ${t("editor.transform.height")}`}
        >
          <div className="editor-resolution-field">
            <span className="editor-resolution-label" aria-hidden="true">
              W
            </span>
            <input
              type="number"
              className="editor-resolution-input"
              value={resizeW || ""}
              min={1}
              onChange={(e) => setResizeW(Number(e.target.value) || 0)}
              disabled={busy}
              aria-label={t("editor.transform.width")}
            />
          </div>
          <span className="editor-resolution-x" aria-hidden="true">
            ×
          </span>
          <div className="editor-resolution-field">
            <input
              type="number"
              className="editor-resolution-input"
              value={resizeH || ""}
              min={1}
              onChange={(e) => setResizeH(Number(e.target.value) || 0)}
              disabled={busy}
              aria-label={t("editor.transform.height")}
            />
            <span className="editor-resolution-label" aria-hidden="true">
              H
            </span>
          </div>
        </div>

        <span className="editor-toolbar-divider" aria-hidden="true" />

        <div className="editor-toolbar-actions">
          {hasUndoable && (
            <button
              className="editor-chip"
              disabled={busy}
              type="button"
              onClick={reset}
              title={t("editor.transform.reset")}
            >
              {t("editor.transform.reset")}
            </button>
          )}
          <button
            className={`editor-apply${busy ? " is-busy" : ""}`}
            disabled={busy || !hasPendingApply}
            onClick={apply}
            type="button"
            aria-label={t("editor.transform.apply")}
          >
            {t("editor.transform.apply")}
          </button>
        </div>
      </div>
    </>
  );
}
