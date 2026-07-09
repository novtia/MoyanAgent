import { useEffect, useRef, useState } from "react";
import { save } from "@tauri-apps/plugin-dialog";
import { useTranslation } from "react-i18next";
import { api, srcOf } from "../../api/tauri";
import type { ImageRefAbs } from "../../types";
import { toast } from "../ui/Toast";

interface VideoPreviewProps {
  item: ImageRefAbs;
  onClose: () => void;
}

export function VideoPreview({ item, onClose }: VideoPreviewProps) {
  const { t } = useTranslation();
  const dialogRef = useRef<HTMLDivElement | null>(null);
  const videoRef = useRef<HTMLVideoElement | null>(null);
  const [failed, setFailed] = useState(false);

  useEffect(() => {
    dialogRef.current?.focus();
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        onClose();
        return;
      }
      const video = videoRef.current;
      if (!video) return;
      if (event.key === " ") {
        event.preventDefault();
        if (video.paused) void video.play();
        else video.pause();
      } else if (event.key === "ArrowLeft") {
        event.preventDefault();
        video.currentTime = Math.max(0, video.currentTime - 5);
      } else if (event.key === "ArrowRight") {
        event.preventDefault();
        video.currentTime = Math.min(
          Number.isFinite(video.duration) ? video.duration : video.currentTime + 5,
          video.currentTime + 5,
        );
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [onClose]);

  const saveAs = async () => {
    try {
      const ext = item.mime === "video/quicktime" ? "mov" : "mp4";
      const destination = await save({
        defaultPath: `atelier-${Date.now()}.${ext}`,
        filters: [{ name: "Video", extensions: [ext] }],
      });
      if (destination) {
        await api.exportMedia(item.id, destination as string);
      }
    } catch (error) {
      console.error(error);
      toast.error(t("preview.saveFailed"));
    }
  };

  const enterSystemFullscreen = async () => {
    const video = videoRef.current;
    if (!video) return;
    try {
      await video.requestFullscreen();
    } catch (error) {
      console.warn(error);
    }
  };

  return (
    <div
      ref={dialogRef}
      className="preview-lightbox video-preview-lightbox"
      role="dialog"
      aria-modal="true"
      aria-label={t("preview.videoTitle")}
      tabIndex={-1}
      onMouseDown={(event) => {
        if (event.target === event.currentTarget) onClose();
      }}
    >
      <div
        className="video-preview-stage"
        onMouseDown={(event) => {
          if (event.target === event.currentTarget) onClose();
        }}
      >
        {failed ? (
          <button
            type="button"
            className="video-preview-error"
            onClick={() => {
              setFailed(false);
              videoRef.current?.load();
            }}
          >
            {t("preview.videoRetry")}
          </button>
        ) : null}
        <video
          ref={videoRef}
          key={item.id}
          src={srcOf(item.abs_path)}
          controls
          autoPlay
          playsInline
          preload="metadata"
          onError={() => setFailed(true)}
        />
      </div>

      <div className="preview-topbar">
        <div className="preview-info">
          <b>{t("preview.videoTitle")}</b>
          <span className="preview-info-dot" />
          <span className="preview-info-dim">{item.mime}</span>
        </div>
        <button
          type="button"
          className="preview-icon-btn is-close is-chrome"
          onClick={onClose}
          title={t("preview.close")}
          aria-label={t("preview.close")}
        >
          <CloseIcon />
        </button>
      </div>

      <div className="preview-toolbar video-preview-toolbar">
        <button
          type="button"
          className="preview-icon-btn"
          onClick={enterSystemFullscreen}
          title={t("preview.systemFullscreen")}
          aria-label={t("preview.systemFullscreen")}
        >
          <FullscreenIcon />
        </button>
        <span className="preview-toolbar-divider" />
        <button
          type="button"
          className="preview-icon-btn is-primary"
          onClick={saveAs}
          title={t("preview.saveAs")}
          aria-label={t("preview.saveAs")}
        >
          <DownloadIcon />
        </button>
      </div>
    </div>
  );
}

function CloseIcon() {
  return (
    <svg viewBox="0 0 16 16" fill="none" aria-hidden>
      <path d="m4 4 8 8M12 4l-8 8" stroke="currentColor" strokeWidth="1.6" />
    </svg>
  );
}

function FullscreenIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8">
      <path d="M8 3H3v5M16 3h5v5M8 21H3v-5M16 21h5v-5" />
    </svg>
  );
}

function DownloadIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8">
      <path d="M12 3v12m0 0 4-4m-4 4-4-4M5 21h14" />
    </svg>
  );
}
