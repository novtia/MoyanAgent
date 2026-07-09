import { useTranslation } from "react-i18next";
import { save } from "@tauri-apps/plugin-dialog";
import { api, srcOf } from "../../../api/tauri";
import type { PlateActionsProps } from "./types";
import { CopyIcon, DownloadIcon, ZoomIcon } from "./icons";

export function PlateActions({
  img,
  onPreview,
  showDivider,
}: PlateActionsProps) {
  const { t } = useTranslation();
  const isVideo = img.mime.startsWith("video/");
  const downloadAs = async () => {
    const ext = isVideo
      ? img.mime === "video/quicktime"
        ? "mov"
        : "mp4"
      : img.mime === "image/jpeg"
        ? "jpg"
        : img.mime === "image/webp"
          ? "webp"
          : "png";
    const dest = await save({
      defaultPath: `atelier-${Date.now()}.${ext}`,
      filters: [{ name: isVideo ? "Video" : "Image", extensions: [ext] }],
    });
    if (!dest) return;
    await api.exportMedia(img.id, dest as string);
  };
  const copyImage = async () => {
    try {
      const url = srcOf(img.abs_path);
      const blob = await (await fetch(url)).blob();
      if (
        navigator.clipboard &&
        (window as any).ClipboardItem &&
        ["image/png", "image/jpeg", "image/webp"].includes(blob.type)
      ) {
        await navigator.clipboard.write([new ClipboardItem({ [blob.type]: blob })]);
      } else {
        throw new Error("clipboard not supported");
      }
    } catch (e) {
      console.warn(e);
    }
  };
  return (
    <>
      <button type="button" className="msg-action" onClick={onPreview}>
        <ZoomIcon />
        <span>{t("message.actionPreview")}</span>
      </button>
      <button type="button" className="msg-action" onClick={downloadAs}>
        <DownloadIcon />
        <span>{t("message.actionDownload")}</span>
      </button>
      {!isVideo && (
        <button type="button" className="msg-action" onClick={copyImage}>
          <CopyIcon />
          <span>{t("message.actionCopyImage")}</span>
        </button>
      )}
      {showDivider && <span className="divider" aria-hidden />}
    </>
  );
}
