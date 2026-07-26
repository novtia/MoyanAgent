import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { srcOf } from "../../../../../api/tauri";
import {
  isMediaFileType,
  readerFileName,
  type ReaderMediaFileType,
} from "../../../../../store/reader";

export interface MediaReaderProps {
  path: string;
  fileType: ReaderMediaFileType;
}

/** Inline image / video / audio viewer for the reader editor pane. */
export function MediaReader({ path, fileType }: MediaReaderProps) {
  const { t } = useTranslation();
  const [failed, setFailed] = useState(false);
  const src = srcOf(path);
  const name = readerFileName(path);

  useEffect(() => {
    setFailed(false);
  }, [path, fileType]);

  if (!isMediaFileType(fileType) || !src) {
    return (
      <div className="reader-media-view is-empty">
        <p className="reader-media-status">{t("reader.mediaUnsupported")}</p>
      </div>
    );
  }

  if (failed) {
    return (
      <div className="reader-media-view is-empty">
        <p className="reader-media-status">{t("reader.mediaLoadFailed")}</p>
        <p className="reader-media-filename" title={path}>
          {name}
        </p>
      </div>
    );
  }

  if (fileType === "image") {
    return (
      <div className="reader-media-view reader-media-view--image">
        <img
          key={path}
          className="reader-media-image"
          src={src}
          alt={name}
          title={path}
          draggable={false}
          onError={() => setFailed(true)}
        />
      </div>
    );
  }

  if (fileType === "video") {
    return (
      <div className="reader-media-view reader-media-view--video">
        <video
          key={path}
          className="reader-media-video"
          controls
          preload="metadata"
          src={src}
          onError={() => setFailed(true)}
        />
      </div>
    );
  }

  return (
    <div className="reader-media-view reader-media-view--audio">
      <p className="reader-media-filename" title={path}>
        {name}
      </p>
      <audio
        key={path}
        className="reader-media-audio"
        controls
        preload="metadata"
        src={src}
        onError={() => setFailed(true)}
      />
    </div>
  );
}
