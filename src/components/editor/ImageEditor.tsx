import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import type { AttachmentDraft } from "../../types";
import { CropTool } from "./CropTool";
import { TransformTool } from "./TransformTool";
import { MaskTool } from "./MaskTool";

interface Props {
  target: AttachmentDraft;
  onClose: () => void;
  onApplied: (newDraft: AttachmentDraft) => void;
}

type Tab = "crop" | "transform" | "mask";

export function ImageEditor({ target, onClose, onApplied }: Props) {
  const { t } = useTranslation();
  const [tab, setTab] = useState<Tab>("crop");
  const [busy, setBusy] = useState(false);

  // Esc closes the editor — but never while a backend op is running, since
  // closing mid-flight would orphan the result. We also avoid the previous
  // behaviour of "click backdrop to close" because edits are easy to lose.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !busy) onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose, busy]);

  return (
    <div
      className="preview-lightbox"
      role="dialog"
      aria-modal="true"
      aria-label={t("editor.title")}
    >
      <div className="editor-tabs-floating" role="tablist" aria-label={t("editor.title")}>
        <button
          type="button"
          role="tab"
          aria-selected={tab === "crop"}
          className={`editor-tabs-floating-tab${tab === "crop" ? " is-active" : ""}`}
          onClick={() => setTab("crop")}
          disabled={busy}
        >
          {t("editor.tabCrop")}
        </button>
        <button
          type="button"
          role="tab"
          aria-selected={tab === "transform"}
          className={`editor-tabs-floating-tab${tab === "transform" ? " is-active" : ""}`}
          onClick={() => setTab("transform")}
          disabled={busy}
        >
          {t("editor.tabTransform")}
        </button>
        <button
          type="button"
          role="tab"
          aria-selected={tab === "mask"}
          className={`editor-tabs-floating-tab${tab === "mask" ? " is-active" : ""}`}
          onClick={() => setTab("mask")}
          disabled={busy}
        >
          {t("editor.tabMask")}
        </button>
      </div>

      <button
        className="preview-icon-btn is-chrome is-close editor-close-fab"
        onClick={onClose}
        type="button"
        title={t("editor.close")}
        aria-label={t("editor.close")}
        disabled={busy}
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

      {tab === "crop" && (
        <CropTool target={target} busy={busy} setBusy={setBusy} onApplied={onApplied} />
      )}
      {tab === "transform" && (
        <TransformTool target={target} busy={busy} setBusy={setBusy} onApplied={onApplied} />
      )}
      {tab === "mask" && (
        <MaskTool target={target} busy={busy} setBusy={setBusy} onApplied={onApplied} />
      )}
    </div>
  );
}
