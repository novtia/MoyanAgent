import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { useSession } from "../../store/session";
import { useRightPanel } from "../../store/rightPanel";
import { dialog } from "../ui";
import { MessageList } from "./messageList";
import { ChatFindBar } from "./messageList/ChatFindBar";
import { Composer } from "./Composer";
import { EmptyChat } from "./EmptyChat";
import { RightPanel } from "./rightPanel";
import { ChatFontPanel } from "./ChatFontPanel";
import { ChatSessionBreadcrumb } from "./ChatSessionBreadcrumb";
import type { AttachmentDraft, ImageRefAbs } from "../../types";
import { useChatFind } from "../../store/chatFind";

interface ChatViewProps {
  onOpenMenu?: () => void;
  onEditAttachment: (a: AttachmentDraft) => void;
  onPreviewImage: (img: ImageRefAbs) => void;
  onOpenSettings: () => void;
  needsSetup: boolean;
}

export function ChatView({
  onOpenMenu,
  onEditAttachment,
  onPreviewImage,
  onOpenSettings,
  needsSetup,
}: ChatViewProps) {
  const { t } = useTranslation();
  // Subscribe to the session record and an emptiness flag rather than `active`:
  // streaming swaps `active` every frame, and this tree has no reason to
  // re-render for it.
  const session = useSession((s) => s.active?.session ?? null);
  const isEmpty = useSession((s) => !s.active || s.active.messages.length === 0);
  const busy = useSession((s) => s.busy);
  const generationPhase = useSession((s) =>
    s.activeId ? s.generationPhaseBySession[s.activeId] : undefined,
  );
  const remove = useSession((s) => s.remove);
  const chatFindSessionId = useChatFind((s) => s.sessionId);
  const closeChatFind = useChatFind((s) => s.close);

  // Drop find bar when leaving the session it was opened for.
  useEffect(() => {
    const activeId = session?.id ?? null;
    if (chatFindSessionId && activeId && chatFindSessionId !== activeId) {
      closeChatFind();
    }
    if (chatFindSessionId && !activeId) {
      closeChatFind();
    }
  }, [session?.id, chatFindSessionId, closeChatFind]);

  const [moreOpen, setMoreOpen] = useState(false);
  const galleryOpen = useRightPanel((s) => s.open);
  const setGalleryOpen = useRightPanel((s) => s.setOpen);
  const [fontOpen, setFontOpen] = useState(false);
  const moreRef = useRef<HTMLDivElement | null>(null);
  const fontRef = useRef<HTMLDivElement | null>(null);

  // Expand the panel when a file is opened through an explicit intent (file
  // tree, find result, tool card). Passive restores do not bump revealSeq, so
  // switching sessions never pops the panel open on its own.
  const revealSeq = useRightPanel((s) => s.revealSeq);
  const lastRevealSeq = useRef(revealSeq);
  useEffect(() => {
    if (revealSeq === lastRevealSeq.current) return;
    lastRevealSeq.current = revealSeq;
    setGalleryOpen(true);
  }, [revealSeq, setGalleryOpen]);

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

  const isTemporary = !!session?.is_temporary;
  const title = session?.title || t("chat.defaultTitle");
  const moreMenu = !isEmpty ? (
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
              if (!session) return;
              const ok = await dialog.confirm(
                t("chat.deleteSessionConfirm", { title }),
                { type: "danger", confirmLabel: t("common.delete"), title: t("chat.deleteSession") },
              );
              if (ok) {
                remove(session.id);
                setMoreOpen(false);
              }
            }}
          >
            {t("chat.deleteSession")}
          </button>
        </div>
      )}
    </div>
  ) : null;

  return (
    <main className="chat">
      <div className="chat-main">
        <div className="chat-topbar">
          <div className="chat-topbar-left">
            {onOpenMenu && (
              <button
                type="button"
                className="ghost-btn chat-menu-btn"
                title={t("chat.openMenu")}
                aria-label={t("chat.openMenu")}
                onClick={onOpenMenu}
              >
                <MenuIcon />
              </button>
            )}
            {isEmpty || onOpenMenu ? null : <ChatSessionBreadcrumb />}
            {!onOpenMenu && moreMenu}
            {!isEmpty && !onOpenMenu && (
              <span
                className={`chat-status ${busy ? "busy" : ""}`}
                title={
                  busy && generationPhase === "polling"
                    ? t("chat.statusPolling")
                    : busy
                      ? t("chat.statusGenerating")
                      : t("chat.statusReady")
                }
              >
                <span className="dot" />
                {busy && generationPhase === "polling"
                  ? t("chat.statusPolling")
                  : busy
                    ? t("chat.statusGenerating")
                    : t("chat.statusReady")}
              </span>
            )}
          </div>
          <span className="chat-topbar-session">{title}</span>
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
            <ChatFindBar />
            {!isTemporary && (
              <Composer
                onEditAttachment={onEditAttachment}
                onOpenSettings={onOpenSettings}
                needsSetup={needsSetup}
              />
            )}
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

function MenuIcon() {
  return (
    <svg
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
    >
      <line x1="4" y1="7" x2="20" y2="7" />
      <line x1="4" y1="12" x2="20" y2="12" />
      <line x1="4" y1="17" x2="20" y2="17" />
    </svg>
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
