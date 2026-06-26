import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { useSession } from "../../store/session";
import { useReader } from "../../store/reader";
import { dialog } from "../ui";
import { MessageList } from "./messageList";
import { Composer } from "./Composer";
import { EmptyChat } from "./EmptyChat";
import { RightPanel } from "./RightPanel";
import { ChatFontPanel } from "./ChatFontPanel";
import type { AttachmentDraft, ImageRefAbs } from "../../types";

interface ChatViewProps {
  onEditAttachment: (a: AttachmentDraft) => void;
  onPreviewImage: (img: ImageRefAbs) => void;
  onOpenSettings: () => void;
  needsSetup: boolean;
}

export function ChatView({
  onEditAttachment,
  onPreviewImage,
  onOpenSettings,
  needsSetup,
}: ChatViewProps) {
  const { t } = useTranslation();
  const active = useSession((s) => s.active);
  const busy = useSession((s) => s.busy);
  const remove = useSession((s) => s.remove);

  const [moreOpen, setMoreOpen] = useState(false);
  const [galleryOpen, setGalleryOpen] = useState(false);
  const [fontOpen, setFontOpen] = useState(false);
  const moreRef = useRef<HTMLDivElement | null>(null);
  const fontRef = useRef<HTMLDivElement | null>(null);

  // Open the right panel whenever a document is sent to the reader (from a
  // Read tool result or the "open in reader" button on a tool card).
  const readerOpenSeq = useReader((s) => s.openSeq);
  const lastReaderSeq = useRef(readerOpenSeq);
  useEffect(() => {
    if (readerOpenSeq === lastReaderSeq.current) return;
    lastReaderSeq.current = readerOpenSeq;
    setGalleryOpen(true);
  }, [readerOpenSeq]);

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

  useEffect(() => {
    if (!fontOpen) return;
    const onDoc = (e: MouseEvent) => {
      if (fontRef.current && !fontRef.current.contains(e.target as Node)) {
        setFontOpen(false);
      }
    };
    window.addEventListener("mousedown", onDoc);
    return () => window.removeEventListener("mousedown", onDoc);
  }, [fontOpen]);

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
                      onClick={async () => {
                        if (!active) return;
                        const ok = await dialog.confirm(
                          t("chat.deleteSessionConfirm", { title: active.session.title }),
                          { type: "danger", confirmLabel: t("common.delete"), title: t("chat.deleteSession") },
                        );
                        if (ok) {
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
            <div className="chat-topbar-font" ref={fontRef}>
              <button
                type="button"
                className={`ghost-btn ${fontOpen ? "is-active" : ""}`}
                title={t("chat.fontSettings")}
                aria-pressed={fontOpen}
                onClick={() => setFontOpen((v) => !v)}
              >
                <FontIcon />
              </button>
              {fontOpen && <ChatFontPanel />}
            </div>
            <button
              type="button"
              className={`ghost-btn ${galleryOpen ? "is-active" : ""}`}
              title={t("rightPanel.toggle")}
              aria-pressed={galleryOpen}
              onClick={() => setGalleryOpen((v) => !v)}
            >
              <GalleryIcon />
            </button>
          </div>
        </div>

        {isEmpty ? (
          <EmptyChat
            onEditAttachment={onEditAttachment}
            onOpenSettings={onOpenSettings}
            needsSetup={needsSetup}
          />
        ) : (
          <>
            <MessageList onPreviewImage={onPreviewImage} />
            <Composer
              onEditAttachment={onEditAttachment}
              onOpenSettings={onOpenSettings}
              needsSetup={needsSetup}
            />
          </>
        )}
      </div>

      <RightPanel
        open={galleryOpen}
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

function FontIcon() {
  return (
    <svg
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.6"
      strokeLinecap="round"
      strokeLinejoin="round"
    >
      <path d="M4 20l5-14h2l5 14" />
      <path d="M6.5 14h6" />
      <path d="M16 20l2.5-7h1L22 20" />
      <path d="M17 17.5h3.5" />
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
