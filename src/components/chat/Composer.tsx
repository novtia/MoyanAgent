import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { useSession } from "../../store/session";
import { useSettings } from "../../store/settings";
import { useProject } from "../../store/project";
import { api, srcOf } from "../../api/tauri";
import { toast } from "../ui/Toast";
import {
  ASPECT_RATIOS,
  IMAGE_SIZES,
  RATIO_PIXEL_HINT,
  shortModelName,
} from "../../config/generation";
import type { AttachmentDraft, ModelServiceModel } from "../../types";
import {
  ComposerEditor,
  normalizeMentionPath,
  isWithinProject,
  type ComposerEditorHandle,
} from "./mention";
import { ComposerFileTree } from "./ComposerFileTree";
import { READER_FILE_DRAG_TYPE } from "./ReaderFileExplorer";
import { ATELIER_DRAG_TYPE } from "./SessionGallery";

function nativeFilePath(file: File) {
  return (file as File & { path?: string }).path || "";
}

const IMAGE_EXT_RE = /\.(png|jpe?g|webp|gif|bmp|svg)$/i;

function isImageFile(file: File): boolean {
  return file.type.startsWith("image/") || IMAGE_EXT_RE.test(file.name);
}

/** Popover uses `bottom: calc(100% + POPOVER_GAP)` — its bottom sits this many px above the anchor box top. */
const MODEL_POPOVER_GAP = 8;

/** Min space (px) above topbar for upward popover; otherwise open downward. */
const MODEL_POPOVER_MIN_SPACE_ABOVE = 100;

/** Reasoning effort options for the composer thinking picker. Empty = provider default (high). */
const THINKING_EFFORTS = [
  { value: "", labelKey: "composer.thinkingDefault" },
  { value: "low", labelKey: "composer.thinkingLow" },
  { value: "medium", labelKey: "composer.thinkingMedium" },
  { value: "high", labelKey: "composer.thinkingHigh" },
  { value: "max", labelKey: "composer.thinkingMax" },
] as const;

function scrollableAncestors(el: HTMLElement | null): HTMLElement[] {
  const out: HTMLElement[] = [];
  let node = el?.parentElement ?? null;
  while (node) {
    const oy = getComputedStyle(node).overflowY;
    if (/(auto|scroll|overlay)/.test(oy)) out.push(node);
    node = node.parentElement;
  }
  return out;
}

interface ComposerProps {
  onEditAttachment: (a: AttachmentDraft) => void;
  onOpenSettings: () => void;
  needsSetup: boolean;
}

export function Composer({ onEditAttachment, onOpenSettings, needsSetup }: ComposerProps) {
  const { t } = useTranslation();
  // Subscribe to composer fields granularly (not the whole `composer` object)
  // so that typing — which replaces the composer object on every keystroke —
  // doesn't re-render this heavy component. The prompt text itself lives in the
  // isolated `ComposerEditor` leaf below.
  const attachments = useSession((s) => s.composer.attachments);
  const pendingAttachments = useSession((s) => s.composer.pendingAttachments);
  const aspectRatio = useSession((s) => s.composer.aspectRatio);
  const imageSize = useSession((s) => s.composer.imageSize);
  const thinkingEnabled = useSession((s) => s.composer.thinkingEnabled);
  const thinkingEffort = useSession((s) => s.composer.thinkingEffort);
  const chatMode = useSession((s) => s.composer.chatMode);
  const promptEmpty = useSession((s) => s.composer.prompt.trim().length === 0);
  const setAspectRatio = useSession((s) => s.setAspectRatio);
  const setImageSize = useSession((s) => s.setImageSize);
  const setThinkingEnabled = useSession((s) => s.setThinkingEnabled);
  const setThinkingEffort = useSession((s) => s.setThinkingEffort);
  const addAttachments = useSession((s) => s.addAttachments);
  const addAttachmentsFromPaths = useSession((s) => s.addAttachmentsFromPaths);
  const addAttachmentFromPath = useSession((s) => s.addAttachmentFromPath);
  const removeAttachment = useSession((s) => s.removeAttachment);
  const send = useSession((s) => s.send);
  const interrupt = useSession((s) => s.interrupt);
  const busy = useSession((s) => s.busy);
  const active = useSession((s) => s.active);
  const activeId = useSession((s) => s.activeId);
  const refreshList = useSession((s) => s.refreshList);
  const reloadActiveSession = useSession((s) => s.reloadActiveSession);
  const setChatMode = useSession((s) => s.setChatMode);
  const settings = useSettings((s) => s.settings);
  const update = useSettings((s) => s.update);
  const projects = useProject((s) => s.projects);

  const projectRoot = useMemo(() => {
    const projectId = active?.session.project_id ?? null;
    if (!projectId) return null;
    return projects.find((p) => p.id === projectId)?.path?.trim() || null;
  }, [active, projects]);

  const editorRef = useRef<ComposerEditorHandle | null>(null);
  const paramsRef = useRef<HTMLDivElement | null>(null);
  const thinkingRef = useRef<HTMLDivElement | null>(null);
  const modeRef = useRef<HTMLDivElement | null>(null);
  const modelRef = useRef<HTMLDivElement | null>(null);
  const mentionRef = useRef<HTMLDivElement | null>(null);

  const [paramsOpen, setParamsOpen] = useState(false);
  const [mentionOpen, setMentionOpen] = useState(false);
  const [thinkingOpen, setThinkingOpen] = useState(false);
  const [modeOpen, setModeOpen] = useState(false);
  const [modelOpen, setModelOpen] = useState(false);
  const [modelPopoverMaxPx, setModelPopoverMaxPx] = useState(480);
  const [modelPopoverBelow, setModelPopoverBelow] = useState(false);
  const [dragOver, setDragOver] = useState(false);
  const dragDepth = useRef(0);

  const hasPendingAttachments = pendingAttachments.length > 0;
  const hasAttachments = attachments.length > 0 || hasPendingAttachments;

  const enabledProviders = useMemo(
    () =>
      (settings?.model_services ?? []).filter(
        (p) => p.enabled !== false && p.models.length > 0,
      ),
    [settings?.model_services],
  );
  const modelName = shortModelName(settings?.model);
  const modelLabel = modelName.length > 12 ? `${modelName.slice(0, 12)}…` : modelName;
  const ratioLabel = aspectRatio === "auto" ? t("composer.ratioAuto") : aspectRatio;
  const sizeLabel = imageSize === "auto" ? t("composer.sizeAuto") : imageSize;

  const activeCapabilities = useMemo(() => {
    const provider = (settings?.model_services ?? []).find(
      (p) => p.id === settings?.active_provider_id,
    );
    const model = provider?.models?.find((m) => m.id === settings?.model);
    return model?.capabilities ?? [];
  }, [settings?.model_services, settings?.active_provider_id, settings?.model]);
  const showImageParams = activeCapabilities.includes("image");
  const showThinking = activeCapabilities.includes("reasoning");

  const thinkingLabel = thinkingEnabled
    ? thinkingEffort.trim() || t("composer.thinkingDefault")
    : t("composer.thinkingOff");

  const applyThinking = (enabled: boolean, effort: string) => {
    setThinkingEnabled(enabled);
    setThinkingEffort(effort);
    void update({
      default_thinking_enabled: enabled,
      default_thinking_effort: effort,
    });
    setThinkingOpen(false);
  };

  useEffect(() => {
    const onFocusComposer = () => {
      editorRef.current?.focus();
    };
    window.addEventListener("atelier:focus-composer", onFocusComposer);
    return () =>
      window.removeEventListener("atelier:focus-composer", onFocusComposer);
  }, []);

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
    if (!thinkingOpen) return;
    const onDoc = (e: MouseEvent) => {
      if (thinkingRef.current && !thinkingRef.current.contains(e.target as Node)) {
        setThinkingOpen(false);
      }
    };
    window.addEventListener("mousedown", onDoc);
    return () => window.removeEventListener("mousedown", onDoc);
  }, [thinkingOpen]);

  useEffect(() => {
    if (!modeOpen) return;
    const onDoc = (e: MouseEvent) => {
      if (modeRef.current && !modeRef.current.contains(e.target as Node)) {
        setModeOpen(false);
      }
    };
    window.addEventListener("mousedown", onDoc);
    return () => window.removeEventListener("mousedown", onDoc);
  }, [modeOpen]);

  useEffect(() => {
    if (!mentionOpen) return;
    const onDoc = (e: MouseEvent) => {
      if (mentionRef.current && !mentionRef.current.contains(e.target as Node)) {
        setMentionOpen(false);
      }
    };
    window.addEventListener("mousedown", onDoc);
    return () => window.removeEventListener("mousedown", onDoc);
  }, [mentionOpen]);

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

  useLayoutEffect(() => {
    if (!modelOpen) return;
    const root = modelRef.current;
    if (!root) return;

    const updateMaxHeight = () => {
      const topbar = document.querySelector(".chat-topbar");
      const topbarBottom = topbar?.getBoundingClientRect().bottom ?? 0;
      const marginBelowTopbar = 8;
      const topLimit = topbar ? topbarBottom + marginBelowTopbar : marginBelowTopbar;
      const r = root.getBoundingClientRect();
      const anchorTop = r.top;
      const popoverBottom = anchorTop - MODEL_POPOVER_GAP;
      const rawAbove = Math.floor(popoverBottom - topLimit);

      if (rawAbove >= MODEL_POPOVER_MIN_SPACE_ABOVE) {
        setModelPopoverBelow(false);
        const capped = Math.min(480, Math.max(0, rawAbove));
        setModelPopoverMaxPx(capped);
        return;
      }

      const shell =
        (document.querySelector(".chat-main") as HTMLElement | null) ?? document.documentElement;
      const bottomLimit = shell.getBoundingClientRect().bottom;
      const marginAboveBottom = 12;
      const anchorBottom = r.bottom;
      const popoverTop = anchorBottom + MODEL_POPOVER_GAP;
      const rawBelow = Math.floor(bottomLimit - marginAboveBottom - popoverTop);
      setModelPopoverBelow(true);
      setModelPopoverMaxPx(Math.min(480, Math.max(0, rawBelow)));
    };

    updateMaxHeight();

    const scrollNodes = scrollableAncestors(root);
    for (const n of scrollNodes) {
      n.addEventListener("scroll", updateMaxHeight, { passive: true });
    }
    window.addEventListener("resize", updateMaxHeight);
    return () => {
      for (const n of scrollNodes) {
        n.removeEventListener("scroll", updateMaxHeight);
      }
      window.removeEventListener("resize", updateMaxHeight);
    };
  }, [modelOpen]);

  const pickModel = async (providerId: string, model: ModelServiceModel) => {
    setModelOpen(false);
    const modelId = model.id;
    if (
      providerId !== settings?.active_provider_id ||
      modelId !== settings?.model
    ) {
      await update({ active_provider_id: providerId, model: modelId });
    }
    if (activeId) {
      try {
        await api.setSessionModel(activeId, modelId, model.context_window ?? null);
        await refreshList();
        await reloadActiveSession();
      } catch (e) {
        console.warn(e);
      }
    }
  };

  const onSubmit = async () => {
    if (busy) return;
    if (hasPendingAttachments) return;
    if (!useSession.getState().composer.prompt.trim()) return;
    await send();
  };
  const onSendButtonClick = () => {
    if (busy) {
      interrupt();
      return;
    }
    onSubmit();
  };
  const toggleMentionPanel = () => {
    if (!projectRoot || !activeId) {
      toast.error(t("composer.noProjectForMention"));
      return;
    }
    setMentionOpen((v) => !v);
  };
  const pickMention = (absPath: string, isDir: boolean) => {
    editorRef.current?.insertMention(absPath, isDir);
  };

  const hasDragPayload = (e: React.DragEvent) => {
    const types = Array.from(e.dataTransfer?.types || []);
    return (
      types.includes("Files") ||
      types.includes(ATELIER_DRAG_TYPE) ||
      types.includes(READER_FILE_DRAG_TYPE)
    );
  };

  const insertMentionPath = (rawPath: string): boolean => {
    const p = normalizeMentionPath(rawPath);
    if (projectRoot && !isWithinProject(p, projectRoot)) return false;
    editorRef.current?.insertMention(p);
    return true;
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

    // Files dragged from the reader file explorer become @ mentions.
    // Files dragged from the reader file explorer are already project-internal
    // and trusted — insert them directly without re-validating the path.
    const readerPayload = e.dataTransfer?.getData(READER_FILE_DRAG_TYPE);
    if (readerPayload) {
      try {
        const items = JSON.parse(readerPayload) as Array<
          string | { path: string; isDir?: boolean }
        >;
        for (const it of items) {
          if (typeof it === "string") {
            editorRef.current?.insertMention(it);
          } else if (it && it.path) {
            editorRef.current?.insertMention(it.path, it.isDir);
          }
        }
      } catch (err) {
        console.warn(err);
      }
      return;
    }

    const files = Array.from(e.dataTransfer?.files || []);
    if (!files.length) return;

    const images = files.filter(isImageFile);
    const others = files.filter((f) => !isImageFile(f));

    if (images.length) {
      const imgPaths = images.map(nativeFilePath).filter(Boolean);
      if (imgPaths.length === images.length) {
        addAttachmentsFromPaths(imgPaths);
      } else {
        addAttachments(images);
      }
    }

    if (others.length) {
      let outside = 0;
      for (const f of others) {
        const p = nativeFilePath(f);
        if (!p) continue;
        if (!insertMentionPath(p)) {
          outside += 1;
        }
      }
      if (outside) toast.error(t("composer.mentionOutsideProject"));
    }
  };

  return (
    <div className="composer-dock">
      <div
        className={`composer-card ${dragOver ? "drag-over" : ""}`}
        data-local-file-dropzone="true"
        data-drop-hint={t("composer.dropHint")}
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
            {pendingAttachments.map((a) => (
              <div className="attachment pending" key={a.id} title={a.label}>
                <div className="attachment-placeholder" aria-hidden>
                  <span className="attachment-spinner" />
                </div>
                <span className="badge">{t("composer.uploading")}</span>
              </div>
            ))}
            {attachments.map((a) => (
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

        <ComposerEditor
          ref={editorRef}
          placeholder={
            hasAttachments
              ? t("composer.placeholderWithAttachments")
              : t("composer.placeholderDefault")
          }
          onSubmit={onSubmit}
          disabled={busy}
        />

        <div className="composer-bar">
          <div className="composer-bar-left">
            <div className="composer-mode-wrap" ref={modeRef}>
              <button
                type="button"
                className={`composer-pill composer-mode-pill ${chatMode === "plan" ? "is-plan" : ""} ${modeOpen ? "active" : ""}`}
                title={t("composer.modePickerTitle")}
                onClick={() => setModeOpen((v) => !v)}
              >
                <span className="composer-mode-label">
                  {chatMode === "plan" ? t("composer.modePlan") : t("composer.modeAgent")}
                </span>
                <CaretIcon />
              </button>
              {modeOpen && (
                <div
                  className="composer-mode-popover"
                  role="listbox"
                  aria-label={t("composer.modePickerTitle")}
                  onMouseDown={(e) => e.stopPropagation()}
                >
                  <button
                    type="button"
                    role="option"
                    className={`composer-mode-option ${chatMode === "agent" ? "active" : ""}`}
                    onClick={() => {
                      void setChatMode("agent");
                      setModeOpen(false);
                    }}
                  >
                    <span className="composer-mode-option-title">{t("composer.modeAgent")}</span>
                    <span className="composer-mode-option-desc">{t("composer.modeAgentHint")}</span>
                  </button>
                  <button
                    type="button"
                    role="option"
                    className={`composer-mode-option ${chatMode === "plan" ? "active" : ""}`}
                    onClick={() => {
                      void setChatMode("plan");
                      setModeOpen(false);
                    }}
                  >
                    <span className="composer-mode-option-title">{t("composer.modePlan")}</span>
                    <span className="composer-mode-option-desc">{t("composer.modePlanHint")}</span>
                  </button>
                </div>
              )}
            </div>
            <div className="composer-mention-wrap" ref={mentionRef}>
              <button
                type="button"
                className={`composer-btn composer-add-btn ${mentionOpen ? "active" : ""}`}
                title={t("composer.addFileMention")}
                onClick={toggleMentionPanel}
              >
                <AtIcon />
              </button>
              {mentionOpen && projectRoot && activeId && (
                <div className="composer-mention-popover" role="dialog" aria-label={t("composer.addFileMention")}>
                  <div className="composer-mention-popover-title">{t("composer.addFileMention")}</div>
                  <div className="composer-mention-popover-body">
                    <ComposerFileTree
                      sessionId={activeId}
                      projectRoot={projectRoot}
                      onPick={pickMention}
                    />
                  </div>
                </div>
              )}
            </div>
            {showThinking && (
              <div className="composer-thinking" ref={thinkingRef}>
                <button
                  type="button"
                  className={`composer-pill ${thinkingEnabled ? "is-thinking" : ""} ${thinkingOpen ? "active" : ""}`}
                  title={t("composer.thinkingTitle")}
                  onClick={() => setThinkingOpen((v) => !v)}
                >
                  <BrainIcon />
                  <span>{thinkingLabel}</span>
                  <CaretIcon />
                </button>
                {thinkingOpen && (
                  <div
                    className="composer-mode-popover"
                    role="listbox"
                    aria-label={t("composer.thinkingTitle")}
                    onMouseDown={(e) => e.stopPropagation()}
                  >
                    <button
                      type="button"
                      role="option"
                      className={`composer-mode-option ${!thinkingEnabled ? "active" : ""}`}
                      onClick={() => applyThinking(false, "")}
                    >
                      <span className="composer-mode-option-title">{t("composer.thinkingOff")}</span>
                    </button>
                    {THINKING_EFFORTS.map((opt) => {
                      const isActive =
                        thinkingEnabled &&
                        (thinkingEffort.trim() || "") === opt.value;
                      return (
                        <button
                          key={opt.value || "default"}
                          type="button"
                          role="option"
                          className={`composer-mode-option ${isActive ? "active" : ""}`}
                          onClick={() => applyThinking(true, opt.value)}
                        >
                          <span className="composer-mode-option-title">
                            {t(opt.labelKey)}
                          </span>
                        </button>
                      );
                    })}
                  </div>
                )}
              </div>
            )}
            {showImageParams && (
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
                          className={`chip ${aspectRatio === r ? "active" : ""}`}
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
                      {aspectRatio === "auto"
                        ? t("composer.ratioHintAuto")
                        : t("composer.ratioHint", {
                            ratio: aspectRatio,
                            pixels:
                              RATIO_PIXEL_HINT[aspectRatio] || aspectRatio,
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
                          className={`chip ${imageSize === s ? "active" : ""}`}
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
            )}
          </div>
          <div className="composer-bar-right">
            <div className="composer-ring-model-cluster">
              {active && (
                <ContextRing
                  used={active.session.context_window_used}
                  limit={active.session.context_window}
                />
              )}
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
                  <div
                    className={`model-popover ${modelPopoverBelow ? "model-popover-below" : ""}`}
                    style={{ maxHeight: modelPopoverMaxPx }}
                    onMouseDown={(e) => e.stopPropagation()}
                  >
                    <div className="model-popover-title">{t("composer.modelTitle")}</div>
                    {enabledProviders.length === 0 ? (
                      <div className="model-popover-empty">{t("composer.modelPickerEmpty")}</div>
                    ) : (
                      <div className="model-popover-body">
                        {enabledProviders.map((provider) => (
                          <div key={provider.id} className="model-popover-group">
                            <div className="model-popover-group-title">{provider.name}</div>
                            <div className="model-popover-list">
                              {provider.models.map((modelRow) => {
                                const m = modelRow.id;
                                const isActive =
                                  provider.id === settings?.active_provider_id &&
                                  m === settings?.model;
                                return (
                                  <button
                                    key={`${provider.id}:${m}`}
                                    type="button"
                                    className={`model-popover-item ${isActive ? "active" : ""}`}
                                    onClick={() => pickModel(provider.id, modelRow)}
                                  >
                                    <span className="model-popover-item-text">
                                      {modelRow.name || shortModelName(m)}
                                    </span>
                                    {isActive && <CheckIcon />}
                                  </button>
                                );
                              })}
                            </div>
                          </div>
                        ))}
                      </div>
                    )}
                  </div>
                )}
              </div>
            </div>
            <button
              className={`send-btn ${busy ? "busy" : ""}`}
              type="button"
              onClick={onSendButtonClick}
              disabled={!busy && (hasPendingAttachments || promptEmpty)}
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

function ContextRing({ used, limit }: { used: number; limit: number | null }) {
  const { t } = useTranslation();
  const nf = useMemo(() => new Intl.NumberFormat(undefined), []);

  /** Fixed viewBox; displayed size follows `.composer-context-ring` (1em × 1em). */
  const vb = 24;
  const c = vb / 2;
  const stroke = 2;
  const r = c - stroke / 2;
  const circumference = 2 * Math.PI * r;

  // Three display states:
  //   A) limit known → show ring arc + percentage
  //   B) limit unknown, used > 0 → show raw token count (no arc)
  //   C) limit unknown, used = 0 → "not set" placeholder
  const hasLimit = limit != null && limit > 0;
  const hasUsed = used > 0;

  const ratioRaw = hasLimit ? used / limit! : null;
  const arcRatio = ratioRaw != null ? Math.min(Math.max(ratioRaw, 0), 1) : 0;
  // When limit is unknown but tokens exist, draw a faint quarter-arc as
  // a visual hint that data is available.
  const arcRatioDisplay = ratioRaw != null ? arcRatio : (hasUsed ? 0.12 : 0);
  const dash = circumference * arcRatioDisplay;

  let fillModifier = "";
  if (ratioRaw != null && ratioRaw > 1) fillModifier = " is-over";
  else if (ratioRaw != null && ratioRaw >= 0.85) fillModifier = " is-warn";
  else if (ratioRaw == null && hasUsed) fillModifier = " is-dim";

  const pctInt = ratioRaw != null ? Math.round(Math.min(ratioRaw * 100, 9999)) : null;

  const tooltip = (() => {
    if (hasLimit && pctInt != null) {
      // State A: limit known
      return (
        <>
          <div className="composer-context-ring-tooltip-strong">
            {t("composer.contextRingPct", { pct: pctInt })}
          </div>
          <div className="composer-context-ring-tooltip-muted">
            {t("composer.contextRingTokens", {
              used: nf.format(used),
              limit: nf.format(limit!),
            })}
          </div>
        </>
      );
    }
    if (hasUsed) {
      // State B: no limit but we have actual token data
      return (
        <>
          <div className="composer-context-ring-tooltip-strong">
            {nf.format(used)} tokens
          </div>
          <div className="composer-context-ring-tooltip-muted">
            {t("composer.contextRingUsedNoLimit", { used: nf.format(used) })}
          </div>
        </>
      );
    }
    // State C: nothing to show
    return (
      <div className="composer-context-ring-tooltip-muted">
        {t("composer.contextRingUnknown")}
      </div>
    );
  })();

  return (
    <div className="composer-context-ring" aria-label={t("composer.contextRingAria")}>
      <svg className="composer-context-ring-svg" viewBox={`0 0 ${vb} ${vb}`} aria-hidden>
        <circle
          className="composer-context-ring-track"
          cx={c}
          cy={c}
          r={r}
          fill="none"
          strokeWidth={stroke}
        />
        <circle
          className={`composer-context-ring-fill${fillModifier}`}
          cx={c}
          cy={c}
          r={r}
          fill="none"
          strokeWidth={stroke}
          strokeLinecap="round"
          strokeDasharray={`${dash} ${circumference}`}
          transform={`rotate(-90 ${c} ${c})`}
        />
      </svg>
      <div className="composer-context-ring-tooltip" role="tooltip">
        {tooltip}
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
function AtIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round">
      <circle cx="12" cy="12" r="4" />
      <path d="M16 8v5a3 3 0 0 0 6 0v-1a10 10 0 1 0-3.92 7.94" />
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
function BrainIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round">
      <path d="M9 3a3 3 0 0 0-3 3 3 3 0 0 0-1.5 5.6A3 3 0 0 0 6 17a3 3 0 0 0 3 3Z" />
      <path d="M15 3a3 3 0 0 1 3 3 3 3 0 0 1 1.5 5.6A3 3 0 0 1 18 17a3 3 0 0 1-3 3Z" />
      <path d="M9 3v17M15 3v17" />
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
