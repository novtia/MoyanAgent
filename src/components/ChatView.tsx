import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { useSession } from "../store/session";
import { MessageList } from "./MessageList";
import { Composer } from "./Composer";
import { SessionGallery } from "./SessionGallery";
import type { AttachmentDraft, ImageRefAbs } from "../types";

interface ChatViewProps {
  onEditAttachment: (a: AttachmentDraft) => void;
  onPreviewImage: (img: ImageRefAbs) => void;
  onOpenSettings: () => void;
  needsSetup: boolean;
}

export function ChatView({ onEditAttachment, onPreviewImage, onOpenSettings, needsSetup }: ChatViewProps) {
  const { t } = useTranslation();
  const active = useSession((s) => s.active);
  const busy = useSession((s) => s.busy);
  const remove = useSession((s) => s.remove);

  const [moreOpen, setMoreOpen] = useState(false);
  const [galleryOpen, setGalleryOpen] = useState(false);
  const moreRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    if (!moreOpen) return;
    const onDoc = (e: MouseEvent) => {
      if (moreRef.current && !moreRef.current.contains(e.target as Node)) {
        setMoreOpen(false);
      }
    };
    window.addEventListener("mousedown", onDoc);
    return () => window.removeEventListener("mousedown", onDoc);
  }, [moreOpen]);

  const isEmpty = !active || active.messages.length === 0;
  const title = active?.session.title || t("chat.defaultTitle");

  return (
    <main className="chat">
      <div className="chat-main">
        <div className="chat-topbar">
          <div className="chat-topbar-left">
            <span className="chat-topbar-title" title={title}>
              {isEmpty ? "" : title}
            </span>
            {!isEmpty && (
              <div className="chat-topbar-more" ref={moreRef}>
                <button
                  type="button"
                  className="ghost-btn"
                  title={t("chat.moreTitle")}
                  onClick={() => setMoreOpen((v) => !v)}
                >
                  <DotsIcon />
                </button>
                {moreOpen && (
                  <div className="chat-more-menu">
                    <button
                      type="button"
                      className="chat-more-item danger"
                      onClick={() => {
                        if (
                          active &&
                          window.confirm(
                            t("chat.deleteSessionConfirm", { title: active.session.title }),
                          )
                        ) {
                          remove(active.session.id);
                          setMoreOpen(false);
                        }
                      }}
                    >
                      {t("chat.deleteSession")}
                    </button>
                  </div>
                )}
              </div>
            )}
            {!isEmpty && (
              <span
                className={`chat-status ${busy ? "busy" : ""}`}
                title={busy ? t("chat.statusGenerating") : t("chat.statusReady")}
              >
                <span className="dot" />
                {busy ? t("chat.statusGenerating") : t("chat.statusReady")}
              </span>
            )}
          </div>
          <div className="chat-topbar-right">
            {!isEmpty && (
              <button
                type="button"
                className={`ghost-btn ${galleryOpen ? "is-active" : ""}`}
                title={t("chat.galleryToggle")}
                aria-pressed={galleryOpen}
                onClick={() => setGalleryOpen((v) => !v)}
              >
                <GalleryIcon />
              </button>
            )}
          </div>
        </div>

        <MessageList onPreviewImage={onPreviewImage} />

        <Composer
          onEditAttachment={onEditAttachment}
          onOpenSettings={onOpenSettings}
          needsSetup={needsSetup}
        />
      </div>

      <SessionGallery
        open={galleryOpen && !isEmpty}
        onClose={() => setGalleryOpen(false)}
        onPreviewImage={onPreviewImage}
      />
    </main>
  );
}

function DotsIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="currentColor">
      <circle cx="5" cy="12" r="1.6" />
      <circle cx="12" cy="12" r="1.6" />
      <circle cx="19" cy="12" r="1.6" />
    </svg>
  );
}

function GalleryIcon() {
  return (
    <svg
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.6"
      strokeLinecap="round"
      strokeLinejoin="round"
    >
      <rect x="3" y="5" width="18" height="14" rx="2" />
      <line x1="15" y1="5" x2="15" y2="19" />
    </svg>
  );
}
