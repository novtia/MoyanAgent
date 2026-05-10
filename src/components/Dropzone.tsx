import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { useSession } from "../store/session";

export function Dropzone() {
  const { t } = useTranslation();
  const addAttachments = useSession((s) => s.addAttachments);
  const [active, setActive] = useState(false);

  useEffect(() => {
    let depth = 0;
    const hasFiles = (e: DragEvent) => {
      if (!e.dataTransfer) return false;
      return Array.from(e.dataTransfer.types || []).includes("Files");
    };
    const isLocalFileDropzone = (e: DragEvent) =>
      e.composedPath().some(
        (target) =>
          target instanceof HTMLElement &&
          target.dataset.localFileDropzone === "true",
      );
    const onEnter = (e: DragEvent) => {
      if (!hasFiles(e)) return;
      if (isLocalFileDropzone(e)) {
        depth = 0;
        setActive(false);
        return;
      }
      e.preventDefault();
      depth++;
      setActive(true);
    };
    const onOver = (e: DragEvent) => {
      if (!hasFiles(e)) return;
      if (isLocalFileDropzone(e)) {
        depth = 0;
        setActive(false);
        return;
      }
      e.preventDefault();
      if (e.dataTransfer) e.dataTransfer.dropEffect = "copy";
    };
    const onLeave = (e: DragEvent) => {
      if (!hasFiles(e)) return;
      if (isLocalFileDropzone(e)) {
        setActive(false);
        return;
      }
      depth--;
      if (depth <= 0) {
        depth = 0;
        setActive(false);
      }
    };
    const onDrop = (e: DragEvent) => {
      if (!hasFiles(e)) return;
      if (isLocalFileDropzone(e)) {
        depth = 0;
        setActive(false);
        return;
      }
      e.preventDefault();
      depth = 0;
      setActive(false);
      const files = Array.from(e.dataTransfer?.files || []);
      if (files.length) addAttachments(files);
    };
    window.addEventListener("dragenter", onEnter);
    window.addEventListener("dragover", onOver);
    window.addEventListener("dragleave", onLeave);
    window.addEventListener("drop", onDrop);
    return () => {
      window.removeEventListener("dragenter", onEnter);
      window.removeEventListener("dragover", onOver);
      window.removeEventListener("dragleave", onLeave);
      window.removeEventListener("drop", onDrop);
    };
  }, [addAttachments]);

  return (
    <div className={`dropzone ${active ? "active" : ""}`}>
      <div className="dropzone-inner">
        <div className="dropzone-icon">
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
            <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" />
            <polyline points="17 8 12 3 7 8" />
            <line x1="12" y1="3" x2="12" y2="15" />
          </svg>
        </div>
        <h3>{t("dropzone.title")}</h3>
        <p>{t("dropzone.hint")}</p>
      </div>
    </div>
  );
}
