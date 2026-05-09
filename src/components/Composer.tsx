import { useEffect, useRef, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { useTranslation } from "react-i18next";
import { useSession } from "../store/session";
import { useSettings } from "../store/settings";
import { srcOf } from "../api/tauri";
import {
  ASPECT_RATIOS,
  IMAGE_SIZES,
  MODEL_PRESETS,
  RATIO_PIXEL_HINT,
  shortModelName,
} from "../config/generation";
import type { AttachmentDraft } from "../types";
import { ATELIER_DRAG_TYPE } from "./SessionGallery";

function nativeFilePath(file: File) {
  return (file as File & { path?: string }).path || "";
}

interface ComposerProps {
  onEditAttachment: (a: AttachmentDraft) => void;
  onOpenSettings: () => void;
  needsSetup: boolean;
}

export function Composer({ onEditAttachment, onOpenSettings, needsSetup }: ComposerProps) {
  const { t } = useTranslation();
  const composer = useSession((s) => s.composer);
  const setPrompt = useSession((s) => s.setPrompt);
  const setAspectRatio = useSession((s) => s.setAspectRatio);
  const setImageSize = useSession((s) => s.setImageSize);
  const addAttachments = useSession((s) => s.addAttachments);
  const addAttachmentsFromPaths = useSession((s) => s.addAttachmentsFromPaths);
  const addAttachmentFromPath = useSession((s) => s.addAttachmentFromPath);
  const removeAttachment = useSession((s) => s.removeAttachment);
  const send = useSession((s) => s.send);
  const interrupt = useSession((s) => s.interrupt);
  const busy = useSession((s) => s.busy);
  const settings = useSettings((s) => s.settings);
  const update = useSettings((s) => s.update);

  const taRef = useRef<HTMLTextAreaElement | null>(null);
  const paramsRef = useRef<HTMLDivElement | null>(null);
  const modelRef = useRef<HTMLDivElement | null>(null);

  const [paramsOpen, setParamsOpen] = useState(false);
  const [modelOpen, setModelOpen] = useState(false);
  const [modelDraft, setModelDraft] = useState("");
  const [dragOver, setDragOver] = useState(false);
  const dragDepth = useRef(0);

  useEffect(() => {
    setModelDraft(settings?.model || "");
  }, [settings?.model]);

  const hasPendingAttachments = composer.pendingAttachments.length > 0;
  const hasAttachments = composer.attachments.length > 0 || hasPendingAttachments;

  const modelLabel = shortModelName(settings?.model);
  const ratioLabel = composer.aspectRatio === "auto" ? t("composer.ratioAuto") : composer.aspectRatio;
  const sizeLabel = composer.imageSize === "auto" ? t("composer.sizeAuto") : composer.imageSize;
  const ratioHint =
    composer.aspectRatio === "auto"
      ? null
      : RATIO_PIXEL_HINT[composer.aspectRatio];

  useEffect(() => {
    const ta = taRef.current;
    if (!ta) return;
    ta.style.height = "auto";
    ta.style.height = Math.min(ta.scrollHeight, 200) + "px";
  }, [composer.prompt]);

  useEffect(() => {
    const onPaste = (e: ClipboardEvent) => {
      if (!e.clipboardData) return;
      const files = Array.from(e.clipboardData.files || []);
      if (files.length) {
        e.preventDefault();
        addAttachments(files);
      }
    };
    window.addEventListener("paste", onPaste);
    return () => window.removeEventListener("paste", onPaste);
  }, [addAttachments]);

  useEffect(() => {
    if (!paramsOpen) return;
    const onDoc = (e: MouseEvent) => {
      if (paramsRef.current && !paramsRef.current.contains(e.target as Node)) {
        setParamsOpen(false);
      }
    };
    window.addEventListener("mousedown", onDoc);
    return () => window.removeEventListener("mousedown", onDoc);
  }, [paramsOpen]);

  useEffect(() => {
    if (!modelOpen) return;
    const onDoc = (e: MouseEvent) => {
      if (modelRef.current && !modelRef.current.contains(e.target as Node)) {
        setModelOpen(false);
      }
    };
    window.addEventListener("mousedown", onDoc);
    return () => window.removeEventListener("mousedown", onDoc);
  }, [modelOpen]);

  const commitModel = (val: string) => {
    const v = val.trim();
    if (!v || v === settings?.model) return;
    update({ model: v });
  };

  const pickModel = (m: string) => {
    setModelDraft(m);
    setModelOpen(false);
    if (m !== settings?.model) update({ model: m });
  };

  const onSubmit = async () => {
    if (busy) return;
    if (hasPendingAttachments) return;
    if (!composer.prompt.trim()) return;
    await send();
  };
  const onSendButtonClick = () => {
    if (busy) {
      interrupt();
      return;
    }
    onSubmit();
  };
  const pickAttachments = async () => {
    const selected = await open({
      multiple: true,
      filters: [{ name: "Images", extensions: ["png", "jpg", "jpeg", "webp"] }],
    });
    const paths = Array.isArray(selected) ? selected : selected ? [selected] : [];
    if (paths.length) {
      addAttachmentsFromPaths(paths);
    }
  };

  const hasDragPayload = (e: React.DragEvent) => {
    const types = Array.from(e.dataTransfer?.types || []);
    return types.includes("Files") || types.includes(ATELIER_DRAG_TYPE);
  };

  const onDragEnter = (e: React.DragEvent) => {
    if (!hasDragPayload(e)) return;
    e.preventDefault();
    dragDepth.current += 1;
    setDragOver(true);
  };
  const onDragOver = (e: React.DragEvent) => {
    if (!hasDragPayload(e)) return;
    e.preventDefault();
    if (e.dataTransfer) e.dataTransfer.dropEffect = "copy";
  };
  const onDragLeave = (e: React.DragEvent) => {
    if (!hasDragPayload(e)) return;
    dragDepth.current -= 1;
    if (dragDepth.current <= 0) {
      dragDepth.current = 0;
      setDragOver(false);
    }
  };
  const onDrop = (e: React.DragEvent) => {
    if (!hasDragPayload(e)) return;
    e.preventDefault();
    e.stopPropagation();
    dragDepth.current = 0;
    setDragOver(false);

    const galleryPayload = e.dataTransfer?.getData(ATELIER_DRAG_TYPE);
    if (galleryPayload) {
      try {
        const parsed = JSON.parse(galleryPayload) as { id?: string; abs_path?: string };
        if (parsed.abs_path) {
          addAttachmentFromPath(parsed.abs_path);
        }
      } catch (err) {
        console.warn(err);
      }
      return;
    }

    const files = Array.from(e.dataTransfer?.files || []);
    if (!files.length) return;
    const paths = files.map(nativeFilePath).filter(Boolean);
    if (paths.length === files.length) {
      addAttachmentsFromPaths(paths);
    } else {
      addAttachments(files);
    }
  };

  return (
    <div className="composer-dock">
      <div
        className={`composer-card ${dragOver ? "drag-over" : ""}`}
        onDragEnter={onDragEnter}
        onDragOver={onDragOver}
        onDragLeave={onDragLeave}
        onDrop={onDrop}
      >
        {needsSetup && (
          <button
            type="button"
            className="setup-banner"
            onClick={onOpenSettings}
            title={t("composer.setupTitle")}
          >
            <span className="setup-banner-icon">
              <LockIcon />
            </span>
            <span className="setup-banner-text">{t("composer.setupRequired")}</span>
            <span className="setup-banner-cta">{t("composer.setupCta")}</span>
          </button>
        )}

        {hasAttachments && (
          <div className="composer-attachments">
            {composer.pendingAttachments.map((a) => (
              <div className="attachment pending" key={a.id} title={a.label}>
                <div className="attachment-placeholder" aria-hidden>
                  <span className="attachment-spinner" />
                </div>
                <span className="badge">{t("composer.uploading")}</span>
              </div>
            ))}
            {composer.attachments.map((a) => (
              <div className="attachment" key={a.image_id} title={a.rel_path}>
                <img src={srcOf(a.thumb_abs_path || a.abs_path)} alt="" />
                <button
                  type="button"
                  className="edit"
                  title={t("composer.editAttachment")}
                  onClick={() => onEditAttachment(a)}
                >
                  <PencilIcon />
                </button>
                <button
                  type="button"
                  className="remove"
                  title={t("composer.removeAttachment")}
                  onClick={() => removeAttachment(a.image_id)}
                >
                  ×
                </button>
                <span className="badge">
                  {(a.bytes ?? 0) > 0
                    ? formatBytes(a.bytes!, t)
                    : a.mime.replace("image/", "")}
                </span>
              </div>
            ))}
          </div>
        )}

        <textarea
          ref={taRef}
          rows={1}
          className="composer-textarea"
          placeholder={
            hasAttachments
              ? t("composer.placeholderWithAttachments")
              : t("composer.placeholderDefault")
          }
          value={composer.prompt}
          onChange={(e) => setPrompt(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && !e.shiftKey) {
              e.preventDefault();
              onSubmit();
            }
          }}
          disabled={busy}
        />

        <div className="composer-bar">
          <div className="composer-bar-left">
            <button
              type="button"
              className="composer-btn"
              title={t("composer.addImage")}
              onClick={pickAttachments}
            >
              <PlusIcon />
            </button>
            <div className="composer-params" ref={paramsRef}>
              <button
                type="button"
                className={`composer-pill ${paramsOpen ? "active" : ""}`}
                onClick={() => setParamsOpen((v) => !v)}
              >
                <SlidersIcon />
                <span>{ratioLabel} · {sizeLabel}</span>
                <CaretIcon />
              </button>
              {paramsOpen && (
                <div className="params-popover">
                  <div className="row">
                    <label className="field-label">{t("composer.paramsRatioLabel")}</label>
                    <div className="chips ratio-grid">
                      {ASPECT_RATIOS.map((r) => (
                        <button
                          key={r}
                          type="button"
                          className={`chip ${composer.aspectRatio === r ? "active" : ""}`}
                          onClick={() => {
                            setAspectRatio(r);
                            update({ default_aspect_ratio: r });
                          }}
                        >
                          {r === "auto" ? t("common.auto") : r}
                        </button>
                      ))}
                    </div>
                    <div className="hint">
                      {composer.aspectRatio === "auto"
                        ? t("composer.ratioHintAuto")
                        : t("composer.ratioHint", {
                            ratio: composer.aspectRatio,
                            pixels:
                              RATIO_PIXEL_HINT[composer.aspectRatio] || composer.aspectRatio,
                          })}
                    </div>
                  </div>

                  <div className="row">
                    <label className="field-label">{t("composer.paramsSizeLabel")}</label>
                    <div className="chips">
                      {IMAGE_SIZES.map((s) => (
                        <button
                          key={s}
                          type="button"
                          className={`chip ${composer.imageSize === s ? "active" : ""}`}
                          onClick={() => {
                            setImageSize(s);
                            update({ default_image_size: s });
                          }}
                        >
                          {s === "auto" ? t("common.auto") : s}
                        </button>
                      ))}
                    </div>
                    <div className="hint">{t("composer.sizeHint")}</div>
                  </div>
                </div>
              )}
            </div>
          </div>
          <div className="composer-bar-right">
            {ratioHint && <span className="composer-meta">{ratioHint}</span>}
            <div className="composer-model" ref={modelRef}>
              <button
                type="button"
                className={`composer-pill model ${modelOpen ? "active" : ""}`}
                onClick={() => setModelOpen((v) => !v)}
                title={settings?.model || t("composer.modelPickerTitle")}
              >
                <span>{modelLabel}</span>
                <CaretIcon />
              </button>
              {modelOpen && (
                <div className="model-popover" onMouseDown={(e) => e.stopPropagation()}>
                  <div className="model-popover-title">{t("composer.modelTitle")}</div>
                  <div className="model-popover-input">
                    <input
                      type="text"
                      value={modelDraft}
                      placeholder={t("composer.modelInputPlaceholder")}
                      spellCheck={false}
                      onChange={(e) => setModelDraft(e.target.value)}
                      onKeyDown={(e) => {
                        if (e.key === "Enter") {
                          e.preventDefault();
                          commitModel(modelDraft);
                          setModelOpen(false);
                        }
                        if (e.key === "Escape") {
                          setModelDraft(settings?.model || "");
                          setModelOpen(false);
                        }
                      }}
                    />
                  </div>
                  <div className="model-popover-list">
                    {MODEL_PRESETS.map((m) => (
                      <button
                        key={m}
                        type="button"
                        className={`model-popover-item ${m === settings?.model ? "active" : ""}`}
                        onClick={() => pickModel(m)}
                      >
                        <span className="model-popover-item-text">{m}</span>
                        {m === settings?.model && <CheckIcon />}
                      </button>
                    ))}
                  </div>
                </div>
              )}
            </div>
            <button
              className={`send-btn ${busy ? "busy" : ""}`}
              type="button"
              onClick={onSendButtonClick}
              disabled={!busy && (hasPendingAttachments || !composer.prompt.trim())}
              title={
                busy
                  ? t("composer.sendInterrupt")
                  : hasPendingAttachments
                  ? t("composer.sendUploading")
                  : hasAttachments
                  ? t("composer.sendEdit")
                  : t("composer.sendGenerate")
              }
              aria-label={
                busy
                  ? t("composer.sendInterrupt")
                  : hasPendingAttachments
                  ? t("composer.sendUploading")
                  : hasAttachments
                  ? t("composer.sendEdit")
                  : t("composer.sendGenerate")
              }
            >
              {busy ? (
                <>
                  <span className="send-spinner" aria-hidden />
                  <StopIcon />
                </>
              ) : (
                <ArrowUpIcon />
              )}
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}

function formatBytes(n: number, t: ReturnType<typeof useTranslation>["t"]) {
  if (n < 1024) return t("composer.bytesB", { n });
  if (n < 1024 * 1024) return t("composer.bytesKB", { n: (n / 1024).toFixed(0) });
  return t("composer.bytesMB", { n: (n / (1024 * 1024)).toFixed(1) });
}

function LockIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round">
      <rect x="4" y="11" width="16" height="10" rx="2" />
      <path d="M8 11V8a4 4 0 1 1 8 0v3" />
    </svg>
  );
}
function PlusIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round">
      <line x1="12" y1="5" x2="12" y2="19" />
      <line x1="5" y1="12" x2="19" y2="12" />
    </svg>
  );
}
function SlidersIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round">
      <line x1="4" y1="6" x2="14" y2="6" />
      <line x1="18" y1="6" x2="20" y2="6" />
      <circle cx="16" cy="6" r="2" />
      <line x1="4" y1="12" x2="8" y2="12" />
      <line x1="12" y1="12" x2="20" y2="12" />
      <circle cx="10" cy="12" r="2" />
      <line x1="4" y1="18" x2="14" y2="18" />
      <line x1="18" y1="18" x2="20" y2="18" />
      <circle cx="16" cy="18" r="2" />
    </svg>
  );
}
function CaretIcon() {
  return (
    <svg viewBox="0 0 12 12" width="10" height="10">
      <path
        d="M3 4.5 6 7.5l3-3"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.6"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}
function ArrowUpIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <line x1="12" y1="19" x2="12" y2="5" />
      <polyline points="5 12 12 5 19 12" />
    </svg>
  );
}
function StopIcon() {
  return (
    <svg className="send-stop-icon" viewBox="0 0 24 24" fill="currentColor" aria-hidden>
      <rect x="7" y="7" width="10" height="10" rx="2" />
    </svg>
  );
}
function PencilIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <path d="M12 20h9" />
      <path d="M16.5 3.5a2.121 2.121 0 0 1 3 3L7 19l-4 1 1-4Z" />
    </svg>
  );
}
function CheckIcon() {
  return (
    <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <polyline points="20 6 9 17 4 12" />
    </svg>
  );
}
