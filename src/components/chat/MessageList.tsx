import {
  useEffect,
  useId,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { useTranslation } from "react-i18next";
import { useSession } from "../../store/session";
import { srcOf, api } from "../../api/tauri";
import { open as openDialog, save } from "@tauri-apps/plugin-dialog";
import { ATELIER_DRAG_TYPE } from "./SessionGallery";
import type {
  AssistantBlock,
  AttachmentDraft,
  ImageRefAbs,
  MessageAbs,
} from "../../types";

function nativeFilePath(file: File) {
  return (file as File & { path?: string }).path || "";
}

async function fileToBytes(file: File): Promise<Uint8Array> {
  const buf = await file.arrayBuffer();
  return new Uint8Array(buf);
}

interface MessageListProps {
  onPreviewImage: (img: ImageRefAbs) => void;
}

interface MessageRowProps {
  m: MessageAbs;
  onPreviewImage: (img: ImageRefAbs) => void;
  focused: boolean;
}

interface PlateActionsProps {
  img: ImageRefAbs;
  onPreview: () => void;
  showDivider?: boolean;
}

export function MessageList({ onPreviewImage }: MessageListProps) {
  const { t } = useTranslation();
  const active = useSession((s) => s.active);
  const busy = useSession((s) => s.busy);
  const ref = useRef<HTMLDivElement | null>(null);
  const [focusedMessageId, setFocusedMessageId] = useState<string | null>(null);
  const messages = active?.messages || [];
  const lastMessageTextLength =
    messages.length > 0 ? messages[messages.length - 1].text?.length ?? 0 : 0;
  const lastMessageThinkingLength =
    messages.length > 0
      ? messages[messages.length - 1].params?.thinking_content?.length ?? 0
      : 0;
  const lastMessageBlocksLength =
    messages.length > 0
      ? messages[messages.length - 1].params?.blocks?.length ?? 0
      : 0;
  const hasStreamingAssistant = messages.some((m) => m.id.startsWith("tmp-assistant-"));

  // Track whether the user is near the bottom of the scroll container.
  // When they scroll up to read history we stop forcing scroll-to-bottom.
  const isNearBottomRef = useRef(true);
  const prevMessagesLengthRef = useRef(messages.length);

  // Listen for manual scrolls to update "near bottom" state.
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const onScroll = () => {
      // Within 150 px of the bottom is considered "near bottom".
      isNearBottomRef.current =
        el.scrollHeight - el.scrollTop - el.clientHeight < 150;
    };
    el.addEventListener("scroll", onScroll, { passive: true });
    return () => el.removeEventListener("scroll", onScroll);
  }, []);

  // Always scroll to bottom when switching sessions.
  useEffect(() => {
    if (!ref.current) return;
    ref.current.scrollTop = ref.current.scrollHeight;
    isNearBottomRef.current = true;
    prevMessagesLengthRef.current = messages.length;
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [active?.session.id]);

  // Smart auto-scroll during streaming and when new messages arrive.
  useEffect(() => {
    if (!ref.current) return;
    const messagesGrew = messages.length > prevMessagesLengthRef.current;
    prevMessagesLengthRef.current = messages.length;
    // Always scroll when a new message is added (user just sent, or session
    // reloaded). During streaming only scroll when the user is already near
    // the bottom so we don't hijack their scroll position.
    if (messagesGrew || isNearBottomRef.current) {
      ref.current.scrollTop = ref.current.scrollHeight;
    }
  }, [
    messages.length,
    lastMessageTextLength,
    lastMessageThinkingLength,
    lastMessageBlocksLength,
    busy,
  ]);

  useEffect(() => {
    const onFocusMessage = (event: Event) => {
      const messageId = (event as CustomEvent<{ messageId?: string }>).detail?.messageId;
      if (!messageId) return;
      setFocusedMessageId(messageId);
      window.setTimeout(() => setFocusedMessageId((id) => (id === messageId ? null : id)), 1600);
    };
    window.addEventListener("atelier:focus-message", onFocusMessage);
    return () => window.removeEventListener("atelier:focus-message", onFocusMessage);
  }, []);

  useEffect(() => {
    if (!focusedMessageId || !ref.current) return;
    const selector = `[data-message-id="${focusedMessageId.replace(/["\\]/g, "\\$&")}"]`;
    const node = ref.current.querySelector<HTMLElement>(selector);
    node?.scrollIntoView({ block: "center", behavior: "smooth" });
  }, [focusedMessageId, active?.session.id, active?.messages.length]);

  const isEmpty = messages.length === 0 && !busy;

  return (
    <div className={`messages ${isEmpty ? "is-empty" : ""}`} ref={ref}>
      {isEmpty && (
        <div className="hero">
          <h1 className="hero-title">{t("chat.heroTitle")}</h1>
        </div>
      )}

      {!isEmpty && (
        <div className="messages-inner">
          {messages.map((m, index) => (
            <MessageRow
              key={`${m.id}:${index}`}
              m={m}
              onPreviewImage={onPreviewImage}
              focused={focusedMessageId === m.id}
            />
          ))}
          {busy && !hasStreamingAssistant && <DevelopingRow />}
        </div>
      )}
    </div>
  );
}

function nowStamp(ts: number) {
  const d = new Date(ts);
  return d.toTimeString().slice(0, 8);
}

async function copyText(text: string) {
  if (!text) return;
  try {
    await navigator.clipboard.writeText(text);
  } catch (e) {
    console.warn(e);
  }
}

function ThinkingIcon() {
  return (
    <svg
      className="msg-thinking-icon"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden
    >
      <line x1="12" x2="12" y1="2" y2="6" />
      <line x1="12" x2="12" y1="18" y2="22" />
      <line x1="4.93" x2="7.76" y1="4.93" y2="7.76" />
      <line x1="16.24" x2="19.07" y1="16.24" y2="19.07" />
      <line x1="2" x2="6" y1="12" y2="12" />
      <line x1="18" x2="22" y1="12" y2="12" />
      <line x1="4.93" x2="7.76" y1="19.07" y2="16.24" />
      <line x1="16.24" x2="19.07" y1="7.76" y2="4.93" />
    </svg>
  );
}

function ThinkingChevronIcon() {
  return (
    <svg
      className="msg-thinking-chevron"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden
    >
      <polyline points="6 9 12 15 18 9" />
    </svg>
  );
}

function ThinkingBlock({
  content,
  streaming,
}: {
  content: string;
  streaming: boolean;
}) {
  const { t } = useTranslation();
  const uid = useId();
  const panelId = `${uid}-thinking-panel`;
  const [open, setOpen] = useState(streaming);
  const userToggledRef = useRef(false);
  const prevStreamingRef = useRef(streaming);

  useEffect(() => {
    // Auto-collapse when streaming finishes, unless the user manually toggled.
    if (prevStreamingRef.current && !streaming && !userToggledRef.current) {
      setOpen(false);
    }
    prevStreamingRef.current = streaming;
  }, [streaming]);

  const handleToggle = () => {
    userToggledRef.current = true;
    setOpen((v) => !v);
  };

  return (
    <div
      className={`msg-thinking ${open ? "is-open" : ""} ${
        streaming ? "is-streaming" : ""
      }`}
    >
      <div
        className="msg-thinking-header"
        aria-expanded={open}
        aria-controls={panelId}
        title={t("message.thinkingHint")}
        onClick={handleToggle}
        role="button"
        tabIndex={0}
        onKeyDown={(e) => {
          if (e.key === "Enter" || e.key === " ") {
            e.preventDefault();
            handleToggle();
          }
        }}
      >
        <ThinkingIcon />
        <span className="msg-thinking-label">
          {streaming
            ? t("message.thinkingStreaming")
            : t("message.thinkingToggle")}
        </span>
        <ThinkingChevronIcon />
      </div>
      <div
        id={panelId}
        className="msg-thinking-panel"
        role="region"
        aria-hidden={!open}
      >
        <div className="msg-thinking-panel-inner">
          <div className="msg-thinking-content">{content}</div>
        </div>
      </div>
    </div>
  );
}

/**
 * Render the ordered `blocks` of an assistant message. Each block type
 * (thinking / text / tool_use) is rendered inline in the exact order the
 * stream delivered it, so an agent loop with multiple turns surfaces as
 * `[thinking?, text?, tool A, tool B, thinking?, text?, ...]`.
 *
 * `isStreaming` mirrors `tmp-assistant-*` state: only the *trailing*
 * thinking block of a still-streaming message gets the spinner/streaming
 * label; earlier thinking blocks (already followed by text or a tool
 * card) auto-collapse like a completed reasoning section.
 */
function AssistantContent({
  blocks,
  isStreaming,
}: {
  blocks: AssistantBlock[];
  isStreaming: boolean;
}) {
  const lastThinkingIdx = useMemo(() => {
    for (let i = blocks.length - 1; i >= 0; i--) {
      if (blocks[i].type === "thinking") return i;
    }
    return -1;
  }, [blocks]);
  return (
    <>
      {blocks.map((block, i) => {
        if (block.type === "thinking") {
          const trailing = i === lastThinkingIdx;
          return (
            <ThinkingBlock
              key={`thinking:${i}`}
              content={block.content}
              streaming={isStreaming && trailing}
            />
          );
        }
        if (block.type === "text") {
          if (!block.content) return null;
          return (
            <div key={`text:${i}`} className="text">
              {block.content}
            </div>
          );
        }
        return <ToolCallBlock key={`tool:${block.id}:${i}`} block={block} />;
      })}
    </>
  );
}

function ToolCallIcon({ status }: { status: "pending" | "success" | "error" }) {
  if (status === "pending") {
    return (
      <svg
        className="tool-call-icon"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        strokeWidth="2"
        strokeLinecap="round"
        strokeLinejoin="round"
        aria-hidden
      >
        <circle cx="12" cy="12" r="9" />
        <path d="M12 7v5l3 2" />
      </svg>
    );
  }
  if (status === "error") {
    return (
      <svg
        className="tool-call-icon"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        strokeWidth="2"
        strokeLinecap="round"
        strokeLinejoin="round"
        aria-hidden
      >
        <circle cx="12" cy="12" r="9" />
        <line x1="9" y1="9" x2="15" y2="15" />
        <line x1="15" y1="9" x2="9" y2="15" />
      </svg>
    );
  }
  return (
    <svg
      className="tool-call-icon"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden
    >
      <circle cx="12" cy="12" r="9" />
      <polyline points="8 12 11 15 16 9" />
    </svg>
  );
}

/** Short, single-line preview of the tool's input arguments for the header. */
function summarizeToolInput(input: unknown): string {
  if (input == null) return "";
  if (typeof input === "string") return input;
  if (typeof input === "number" || typeof input === "boolean") return String(input);
  if (Array.isArray(input)) {
    return input
      .map((v) => summarizeToolInput(v))
      .filter(Boolean)
      .join(", ");
  }
  if (typeof input === "object") {
    const o = input as Record<string, unknown>;
    // Heuristic: surface the most identifying field first.
    const primary =
      o.path ?? o.file_path ?? o.command ?? o.query ?? o.prompt ?? o.name;
    if (typeof primary === "string" && primary.trim()) return primary;
    const entries = Object.entries(o).slice(0, 2);
    return entries
      .map(([k, v]) => {
        const s =
          typeof v === "string"
            ? v
            : typeof v === "number" || typeof v === "boolean"
              ? String(v)
              : JSON.stringify(v);
        return `${k}: ${s.length > 40 ? s.slice(0, 40) + "?" : s}`;
      })
      .join("  ");
  }
  return "";
}

function ToolCallBlock({
  block,
}: {
  block: Extract<AssistantBlock, { type: "tool_use" }>;
}) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const status = block.status;
  const statusLabel =
    status === "pending"
      ? t("message.toolCallRunning")
      : status === "error"
        ? t("message.toolCallError")
        : t("message.toolCallDone");
  const summary = useMemo(() => summarizeToolInput(block.input), [block.input]);
  const hasDetail =
    (block.input !== undefined && block.input !== null) ||
    block.output !== undefined;
  const inputJson = useMemo(
    () => safeJsonStringify(block.input),
    [block.input],
  );
  const outputJson = useMemo(
    () => safeJsonStringify(block.output),
    [block.output],
  );

  return (
    <div
      className={`tool-call-block ${status} ${open ? "is-open" : ""}`}
    >
      <button
        type="button"
        className="tool-call-summary"
        aria-expanded={open}
        title={t("message.toolCallToggle")}
        onClick={() => hasDetail && setOpen((v) => !v)}
        disabled={!hasDetail}
      >
        <ToolCallIcon status={status} />
        <span className="tool-call-name">{block.tool}</span>
        {summary && <span className="tool-call-args">{summary}</span>}
        <span className="tool-call-spacer" aria-hidden />
        <span className={`tool-call-badge ${status}`}>{statusLabel}</span>
        {hasDetail && <ThinkingChevronIcon />}
      </button>
      {open && hasDetail && (
        <div className="tool-call-detail">
          {inputJson && (
            <>
              <div className="tool-call-detail-label">
                {t("message.toolCallInput")}
              </div>
              <pre className="tool-call-detail-body">{inputJson}</pre>
            </>
          )}
          {outputJson && (
            <>
              <div className="tool-call-detail-label">
                {t("message.toolCallOutput")}
              </div>
              <pre className="tool-call-detail-body">{outputJson}</pre>
            </>
          )}
        </div>
      )}
    </div>
  );
}

function safeJsonStringify(v: unknown): string {
  if (v === undefined) return "";
  try {
    if (typeof v === "string") return v;
    return JSON.stringify(v, null, 2);
  } catch {
    return String(v);
  }
}

function MessageRow({ m, onPreviewImage, focused }: MessageRowProps) {
  const { t } = useTranslation();
  const inputs = useMemo(() => m.images.filter((i) => i.role === "input"), [m.images]);
  const outputs = m.images.filter((i) => i.role === "output");
  const resendMessage = useSession((s) => s.resendMessage);
  const deleteMessage = useSession((s) => s.deleteMessage);
  const editMessage = useSession((s) => s.editMessage);
  const quoteMessage = useSession((s) => s.quoteMessage);
  const busy = useSession((s) => s.busy);

  const isUser = m.role === "user";
  const isAssistant = m.role === "assistant";
  const isError = m.role === "error";
  const isStreamingDraft = m.id.startsWith("tmp-assistant-");
  const hasText = !!(m.text && m.text.trim());
  const canEditUser = isUser && (hasText || inputs.length > 0);
  const canEditAssistant = isAssistant && hasText && !isStreamingDraft;
  const canEdit = canEditUser || canEditAssistant;
  const canQuote = hasText || inputs.length > 0 || outputs.length > 0;
  const blocks = isAssistant ? m.params?.blocks : undefined;
  const useBlocksRendering = Array.isArray(blocks) && blocks.length > 0;
  const thinkingContent =
    !useBlocksRendering &&
    isAssistant &&
    typeof m.params?.thinking_content === "string"
      ? m.params.thinking_content.trim()
      : "";

  const MAX_EDIT_IMAGES = 8;

  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(m.text || "");
  const [draftImages, setDraftImages] = useState<ImageRefAbs[]>(inputs);
  const [picking, setPicking] = useState(false);
  const [editDragOver, setEditDragOver] = useState(false);
  const editDragDepth = useRef(0);
  const addedDraftIdsRef = useRef<Set<string>>(new Set());
  const taRef = useRef<HTMLTextAreaElement | null>(null);

  useLayoutEffect(() => {
    if (!editing) return;
    const ta = taRef.current;
    if (!ta) return;
    ta.style.height = "auto";
    ta.style.height = ta.scrollHeight + "px";
    ta.focus();
    const len = ta.value.length;
    ta.setSelectionRange(len, len);
  }, [editing]);

  useEffect(() => {
    if (!editing) return;
    const ta = taRef.current;
    if (!ta) return;
    ta.style.height = "auto";
    ta.style.height = ta.scrollHeight + "px";
  }, [draft, editing]);

  const cleanupAddedDrafts = async (ids: Iterable<string>) => {
    for (const id of ids) {
      try {
        await api.removeAttachmentDraft(id);
      } catch (e) {
        console.warn(e);
      }
    }
  };

  const beginEdit = () => {
    if (!canEdit) return;
    setDraft(m.text || "");
    setDraftImages(isUser ? inputs : []);
    addedDraftIdsRef.current = new Set();
    setEditing(true);
  };
  const cancelEdit = async () => {
    const orphans = Array.from(addedDraftIdsRef.current);
    addedDraftIdsRef.current = new Set();
    setDraft(m.text || "");
    setDraftImages(isUser ? inputs : []);
    setEditing(false);
    if (orphans.length) await cleanupAddedDrafts(orphans);
  };

  const draftToImageRef = (d: AttachmentDraft): ImageRefAbs => ({
    id: d.image_id,
    role: "input",
    rel_path: d.rel_path,
    thumb_rel_path: d.thumb_rel_path,
    abs_path: d.abs_path,
    thumb_abs_path: d.thumb_abs_path,
    mime: d.mime,
    width: d.width,
    height: d.height,
    bytes: d.bytes,
    ord: 0,
  });

  const ingestPaths = async (paths: string[]) => {
    const room = MAX_EDIT_IMAGES - draftImages.length;
    if (room <= 0) return;
    const toIngest = paths.slice(0, room);
    for (const p of toIngest) {
      try {
        const d = await api.addAttachmentFromPath(m.session_id, p);
        const ref = draftToImageRef(d);
        addedDraftIdsRef.current.add(ref.id);
        setDraftImages((arr) => [...arr, ref]);
      } catch (e) {
        console.warn(e);
      }
    }
  };

  const ingestFiles = async (files: File[]) => {
    const room = MAX_EDIT_IMAGES - draftImages.length;
    if (room <= 0) return;
    const toIngest = files.slice(0, room);
    for (const file of toIngest) {
      try {
        const path = nativeFilePath(file);
        const d = path
          ? await api.addAttachmentFromPath(m.session_id, path)
          : await api.addAttachmentFromBytes(
              m.session_id,
              file.name || "image",
              await fileToBytes(file),
            );
        const ref = draftToImageRef(d);
        addedDraftIdsRef.current.add(ref.id);
        setDraftImages((arr) => [...arr, ref]);
      } catch (e) {
        console.warn(e);
      }
    }
  };

  const ingestExistingImage = async (absPath: string) => {
    if (!absPath) return;
    if (draftImages.length >= MAX_EDIT_IMAGES) return;
    try {
      const d = await api.addAttachmentFromPath(m.session_id, absPath);
      const ref = draftToImageRef(d);
      addedDraftIdsRef.current.add(ref.id);
      setDraftImages((arr) => [...arr, ref]);
    } catch (e) {
      console.warn(e);
    }
  };

  const addDraftImage = async () => {
    if (picking) return;
    if (draftImages.length >= MAX_EDIT_IMAGES) return;
    setPicking(true);
    try {
      const selected = await openDialog({
        multiple: true,
        filters: [{ name: "Images", extensions: ["png", "jpg", "jpeg", "webp"] }],
      });
      const paths = Array.isArray(selected) ? selected : selected ? [selected] : [];
      if (paths.length) await ingestPaths(paths as string[]);
    } finally {
      setPicking(false);
    }
  };

  const removeDraftImage = async (id: string) => {
    setDraftImages((arr) => arr.filter((i) => i.id !== id));
    if (addedDraftIdsRef.current.has(id)) {
      addedDraftIdsRef.current.delete(id);
      try {
        await api.removeAttachmentDraft(id);
      } catch (e) {
        console.warn(e);
      }
    }
  };

  const editHasDragPayload = (e: React.DragEvent) => {
    const types = Array.from(e.dataTransfer?.types || []);
    return types.includes("Files") || types.includes(ATELIER_DRAG_TYPE);
  };
  const onEditDragEnter = (e: React.DragEvent) => {
    if (!isUser || !editing || !editHasDragPayload(e)) return;
    e.preventDefault();
    editDragDepth.current += 1;
    setEditDragOver(true);
  };
  const onEditDragOver = (e: React.DragEvent) => {
    if (!isUser || !editing || !editHasDragPayload(e)) return;
    e.preventDefault();
    if (e.dataTransfer) e.dataTransfer.dropEffect = "copy";
  };
  const onEditDragLeave = (e: React.DragEvent) => {
    if (!isUser || !editing || !editHasDragPayload(e)) return;
    editDragDepth.current -= 1;
    if (editDragDepth.current <= 0) {
      editDragDepth.current = 0;
      setEditDragOver(false);
    }
  };
  const onEditDrop = (e: React.DragEvent) => {
    if (!isUser || !editing || !editHasDragPayload(e)) return;
    e.preventDefault();
    e.stopPropagation();
    editDragDepth.current = 0;
    setEditDragOver(false);

    const galleryPayload = e.dataTransfer?.getData(ATELIER_DRAG_TYPE);
    if (galleryPayload) {
      try {
        const parsed = JSON.parse(galleryPayload) as { abs_path?: string };
        if (parsed.abs_path) ingestExistingImage(parsed.abs_path);
      } catch (err) {
        console.warn(err);
      }
      return;
    }

    const files = Array.from(e.dataTransfer?.files || []);
    if (!files.length) return;
    ingestFiles(files);
  };

  const imageIdsEqual = (a: ImageRefAbs[], b: ImageRefAbs[]) => {
    if (a.length !== b.length) return false;
    for (let i = 0; i < a.length; i++) if (a[i].id !== b[i].id) return false;
    return true;
  };

  const saveEdit = async () => {
    const next = draft.trim();
    const imagesChanged = !imageIdsEqual(draftImages, inputs);
    if (!next && draftImages.length === 0) {
      await cancelEdit();
      return;
    }
    if (next === (m.text || "").trim() && !imagesChanged) {
      setEditing(false);
      return;
    }
    await editMessage(
      m.id,
      next,
      imagesChanged ? draftImages.map((i) => i.id) : undefined,
    );
    addedDraftIdsRef.current = new Set();
    setEditing(false);
  };
  const onResend = async () => {
    if (busy) return;
    await resendMessage(m.id);
  };
  const onDelete = async () => {
    if (!window.confirm(t("message.deleteConfirm"))) return;
    await deleteMessage(m.id);
  };
  const onCopy = () => {
    copyText(m.text || "");
  };

  return (
    <div
      className={`msg ${m.role} ${editing ? "is-editing" : ""} ${focused ? "is-focused" : ""}`}
      data-message-id={m.id}
    >
      <div className="msg-col">
        <div className="bubble">
          {!isUser && !(editing && isAssistant) && (
            <span className="stamp">
              {isStreamingDraft
                ? t("message.stampGenerating", { time: nowStamp(m.created_at) })
                : isAssistant
                ? t("message.stampImage", { time: nowStamp(m.created_at) })
                : t("message.stampFailure", { time: nowStamp(m.created_at) })}
            </span>
          )}

          {!editing && !useBlocksRendering && thinkingContent ? (
            <ThinkingBlock
              content={thinkingContent}
              streaming={isStreamingDraft}
            />
          ) : null}

          {!editing && inputs.length > 0 && (
            <div className="attached">
              {inputs.map((img, i) => (
                <img
                  key={`in:${img.id}:${i}`}
                  src={srcOf(img.thumb_abs_path || img.abs_path)}
                  title={img.rel_path}
                  onClick={() => onPreviewImage(img)}
                />
              ))}
            </div>
          )}

          {!editing && useBlocksRendering && (
            <AssistantContent
              blocks={blocks as AssistantBlock[]}
              isStreaming={isStreamingDraft}
            />
          )}

          {!editing && !useBlocksRendering && hasText && !isError && (
            <div className="text">{m.text}</div>
          )}
          {!editing && isError && <div className="text mono">{m.text}</div>}

          {editing && (
            <div
              className={`bubble-edit ${editDragOver ? "drag-over" : ""}`}
              data-local-file-dropzone="true"
              onDragEnter={onEditDragEnter}
              onDragOver={onEditDragOver}
              onDragLeave={onEditDragLeave}
              onDrop={onEditDrop}
            >
              {isUser && (draftImages.length > 0 || picking) && (
                <div className="bubble-edit-images">
                  {draftImages.map((img, i) => (
                    <div
                      className="bubble-edit-image"
                      key={`draft:${img.id}:${i}`}
                      title={img.rel_path}
                    >
                      <img
                        src={srcOf(img.thumb_abs_path || img.abs_path)}
                        alt=""
                        draggable={false}
                      />
                      <button
                        type="button"
                        className="bubble-edit-image-remove"
                        title={t("message.editRemoveImage")}
                        onClick={() => removeDraftImage(img.id)}
                      >
                        ?
                      </button>
                    </div>
                  ))}
                </div>
              )}
              <textarea
                ref={taRef}
                className="bubble-edit-textarea field-input field-input--bare"
                value={draft}
                onChange={(e) => setDraft(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Escape") {
                    e.preventDefault();
                    cancelEdit();
                  } else if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
                    e.preventDefault();
                    saveEdit();
                  }
                }}
              />
              <div className="bubble-edit-actions">
                {isUser ? (
                  <button
                    type="button"
                    className="bubble-edit-icon-btn"
                    title={t("message.editAddImage")}
                    onClick={addDraftImage}
                    disabled={picking || draftImages.length >= MAX_EDIT_IMAGES}
                  >
                    <PlusIcon />
                  </button>
                ) : (
                  <span className="bubble-edit-icon-placeholder" aria-hidden />
                )}
                <span className="bubble-edit-spacer" />
                <button
                  type="button"
                  className="bubble-edit-btn ghost"
                  onClick={cancelEdit}
                >
                  {t("message.editCancel")}
                </button>
                <button
                  type="button"
                  className="bubble-edit-btn primary"
                  onClick={saveEdit}
                  disabled={
                    (draft.trim() === (m.text || "").trim() &&
                      imageIdsEqual(draftImages, inputs)) ||
                    (!draft.trim() && draftImages.length === 0)
                  }
                >
                  {t("message.editSend")}
                </button>
              </div>
            </div>
          )}

          {outputs.length > 0 && (!editing || isAssistant) && (
            <div className="outputs">
              {outputs.map((img, i) => (
                <div
                  className="plate"
                  key={`out:${img.id}:${i}`}
                  onClick={() => onPreviewImage(img)}
                >
                  <img src={srcOf(img.abs_path)} alt="generated" />
                </div>
              ))}
            </div>
          )}
        </div>

        {!editing && !isStreamingDraft && (
          <div className="msg-action-bar">
            {outputs.map((img, i) => (
              <PlateActions
                key={`plate:${img.id}:${i}`}
                img={img}
                onPreview={() => onPreviewImage(img)}
                showDivider={outputs.length > 1 && i < outputs.length - 1}
              />
            ))}
            {outputs.length > 0 && (hasText || isUser) && (
              <span className="divider" aria-hidden />
            )}
            {hasText && (
              <button type="button" className="msg-action" onClick={onCopy}>
                <CopyIcon />
                <span>{t("message.actionCopy")}</span>
              </button>
            )}
            {canEdit && (
              <button
                type="button"
                className="msg-action"
                onClick={beginEdit}
              >
                <EditIcon />
                <span>{t("message.actionEdit")}</span>
              </button>
            )}
            {isUser && hasText && (
              <button
                type="button"
                className="msg-action"
                onClick={onResend}
                disabled={busy}
              >
                <ResendIcon />
                <span>{t("message.actionResend")}</span>
              </button>
            )}
            {canQuote && (
              <button
                type="button"
                className="msg-action"
                onClick={() => quoteMessage(m)}
                title={t("message.quoteTitle")}
              >
                <QuoteIcon />
                <span>{t("message.actionQuote")}</span>
              </button>
            )}
            <button
              type="button"
              className="msg-action danger"
              onClick={onDelete}
            >
              <TrashIcon />
              <span>{t("message.actionDelete")}</span>
            </button>
          </div>
        )}
      </div>
    </div>
  );
}

function PlateActions({
  img,
  onPreview,
  showDivider,
}: PlateActionsProps) {
  const { t } = useTranslation();
  const downloadAs = async () => {
    const ext = img.mime === "image/jpeg" ? "jpg" : img.mime === "image/webp" ? "webp" : "png";
    const dest = await save({
      defaultPath: `atelier-${Date.now()}.${ext}`,
      filters: [{ name: "Image", extensions: [ext] }],
    });
    if (!dest) return;
    await api.exportImage(img.id, dest as string);
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
      <button type="button" className="msg-action" onClick={copyImage}>
        <CopyIcon />
        <span>{t("message.actionCopyImage")}</span>
      </button>
      {showDivider && <span className="divider" aria-hidden />}
    </>
  );
}

function DevelopingRow() {
  const { t } = useTranslation();
  return (
    <div className="msg assistant">
      <div className="msg-col">
        <div className="bubble">
          <span className="stamp">{t("message.stampGenerating", { time: nowStamp(Date.now()) })}</span>
          <div className="developing">
            <span className="dots">
              <span className="dot" />
              <span className="dot" />
              <span className="dot" />
            </span>
            <span>{t("message.generatingText")}</span>
          </div>
        </div>
      </div>
    </div>
  );
}

function CopyIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round">
      <rect x="9" y="9" width="11" height="11" rx="2" />
      <path d="M5 15V5a2 2 0 0 1 2-2h10" />
    </svg>
  );
}
function ZoomIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round">
      <circle cx="11" cy="11" r="7" />
      <path d="m20 20-3.5-3.5" />
      <path d="M11 8v6M8 11h6" />
    </svg>
  );
}
function DownloadIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round">
      <path d="M12 3v12" />
      <polyline points="7 10 12 15 17 10" />
      <path d="M5 19h14" />
    </svg>
  );
}
function EditIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round">
      <path d="M12 20h9" />
      <path d="M16.5 3.5a2.121 2.121 0 0 1 3 3L7 19l-4 1 1-4Z" />
    </svg>
  );
}
function ResendIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round">
      <path d="M21 12a9 9 0 1 1-3-6.7" />
      <polyline points="21 4 21 9 16 9" />
    </svg>
  );
}
function TrashIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round">
      <polyline points="3 6 5 6 21 6" />
      <path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2" />
    </svg>
  );
}
function PlusIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round">
      <line x1="12" y1="5" x2="12" y2="19" />
      <line x1="5" y1="12" x2="19" y2="12" />
    </svg>
  );
}
function QuoteIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round">
      <path d="M7 9H5a2 2 0 0 0-2 2v7h6v-5a2 2 0 0 0-2-2V9Z" />
      <path d="M17 9h-2a2 2 0 0 0-2 2v7h6v-5a2 2 0 0 0-2-2V9Z" />
    </svg>
  );
}
