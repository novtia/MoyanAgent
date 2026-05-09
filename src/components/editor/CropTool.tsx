import { useState } from "react";
import Cropper, { type Area } from "react-easy-crop";
import { useTranslation } from "react-i18next";
import { api, srcOf } from "../../api/tauri";
import type { AttachmentDraft } from "../../types";

interface Props {
  target: AttachmentDraft;
  busy: boolean;
  setBusy: (b: boolean) => void;
  onApplied: (newDraft: AttachmentDraft) => void;
}

const ASPECTS: { key: string; labelKey?: string; label?: string; value: number | undefined }[] = [
  { key: "free", labelKey: "editor.crop.aspectFree", value: undefined },
  { key: "1:1", label: "1:1", value: 1 },
  { key: "4:3", label: "4:3", value: 4 / 3 },
  { key: "3:4", label: "3:4", value: 3 / 4 },
  { key: "16:9", label: "16:9", value: 16 / 9 },
  { key: "9:16", label: "9:16", value: 9 / 16 },
];

export function CropTool({ target, busy, setBusy, onApplied }: Props) {
  const { t } = useTranslation();
  const [crop, setCrop] = useState({ x: 0, y: 0 });
  const [zoom, setZoom] = useState(1);
  const [aspect, setAspect] = useState<number | undefined>(undefined);
  const [croppedArea, setCroppedArea] = useState<Area | null>(null);

  const apply = async () => {
    if (!croppedArea) return;
    setBusy(true);
    try {
      const result = await api.editImage(target.image_id, {
        type: "crop",
        x: Math.round(croppedArea.x),
        y: Math.round(croppedArea.y),
        width: Math.round(croppedArea.width),
        height: Math.round(croppedArea.height),
      });
      onApplied({
        image_id: result.id,
        rel_path: result.rel_path,
        thumb_rel_path: result.thumb_rel_path,
        abs_path: result.abs_path,
        thumb_abs_path: result.thumb_abs_path,
        mime: result.mime,
        width: result.width,
        height: result.height,
        bytes: result.bytes,
      });
    } catch (e) {
      console.error(e);
      alert(`${t("editor.crop.error")}: ${e}`);
    } finally {
      setBusy(false);
    }
  };

  return (
    <>
      <div className="editor-stage-full">
        <Cropper
          image={srcOf(target.abs_path)}
          crop={crop}
          zoom={zoom}
          aspect={aspect}
          minZoom={1}
          maxZoom={6}
          onCropChange={setCrop}
          onZoomChange={setZoom}
          onCropComplete={(_, area) => setCroppedArea(area)}
          objectFit="contain"
        />
      </div>

      <div className="editor-toolbar" role="toolbar" aria-label={t("editor.tabCrop")}>
        <div className="editor-seg" role="group" aria-label="aspect ratio">
          {ASPECTS.map((a) => (
            <button
              key={a.key}
              type="button"
              className={`editor-chip${aspect === a.value ? " is-active" : ""}`}
              onClick={() => setAspect(a.value)}
              disabled={busy}
              aria-pressed={aspect === a.value}
            >
              {a.labelKey ? t(a.labelKey) : a.label}
            </button>
          ))}
        </div>

        <span className="editor-toolbar-divider" aria-hidden="true" />

        <span className="editor-toolbar-label">{t("editor.crop.zoom")}</span>
        <input
          type="range"
          className="editor-range"
          min={1}
          max={6}
          step={0.01}
          value={zoom}
          onChange={(e) => setZoom(Number(e.target.value))}
          disabled={busy}
          aria-label={t("editor.crop.zoom")}
        />

        <span className="editor-toolbar-divider" aria-hidden="true" />

        <div className="editor-toolbar-actions">
          <button
            className={`editor-apply${busy ? " is-busy" : ""}`}
            disabled={busy || !croppedArea}
            onClick={apply}
            type="button"
          >
            {t("editor.crop.apply")}
          </button>
        </div>
      </div>
    </>
  );
}
