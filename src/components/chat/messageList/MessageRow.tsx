import {
  memo,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { useTranslation } from "react-i18next";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { api, srcOf } from "../../../api/tauri";
import { dialog, toast } from "../../ui";
import { useProject } from "../../../store/project";
import { useSession } from "../../../store/session";
import { ATELIER_DRAG_TYPE } from "../SessionGallery";
import { READER_FILE_DRAG_TYPE } from "../ReaderFileExplorer";
import {
  MentionEditor,
  MentionText,
  isWithinProject,
  normalizeMentionPath,
  parseMentionPaths,
  type MentionEditorHandle,
} from "../mention";
import type { AssistantBlock, AttachmentDraft, ImageRefAbs } from "../../../types";
import { AssistantContent } from "./AssistantContent";
import { MessageTokenUsage } from "./MessageTokenUsage";
import { PlateActions } from "./PlateActions";
import { ThinkingBlock } from "./ThinkingBlock";
import type { MessageRowProps } from "./types";
import {
  copyText,
  fileToBytes,
  isImageFile,
  nativeFilePath,
  nowStamp,
} from "./utils";
import {
  CopyIcon,
  EditIcon,
  PlusIcon,
  QuoteIcon,
  ResendIcon,
  TrashIcon,
} from "./icons";

function MessageRowImpl({ m, onPreviewImage, focused }: MessageRowProps) {
  const { t } = useTranslation();
  const inputs = useMemo(() => m.images.filter((i) => i.role === "input"), [m.images]);
  const outputs = m.images.filter((i) => i.role === "output");
  const resendMessage = useSession((s) => s.resendMessage);
  const deleteMessage = useSession((s) => s.deleteMessage);
  const editMessage = useSession((s) => s.editMessage);
  const quoteMessage = useSession((s) => s.quoteMessage);
  const busy = useSession((s) => s.busy);
  const active = useSession((s) => s.active);

  const isUser = m.role === "user";
  const isAssistant = m.role === "assistant";
  const isError = m.role === "error";
  const busyBySession = useSession((s) => s.busyBySession);
  const isStreamingDraft =
    m.id.startsWith("tmp-assistant-") && !!busyBySession[m.session_id];
  const hasText = !!(m.text && m.text.trim());
  const canEditUser = isUser && (hasText || inputs.length > 0);
  const canEditAssistant = isAssistant && !isStreamingDraft;
  const canEdit = canEditUser || canEditAssistant;
  const canQuote = hasText || inputs.length > 0 || outputs.length > 0;
  const blocks = isAssistant ? m.params?.blocks : undefined;
  const useBlocksRendering = Array.isArray(blocks) && blocks.length > 0;
  const blocksText = useMemo(() => {
    if (!Array.isArray(blocks)) return "";
    return blocks
      .filter((b): b is Extract<AssistantBlock, { type: "text" }> => b.type === "text")
      .map((b) => b.content)
      .join("")
      .trim();
  }, [blocks]);
  // Agent messages render text from `blocks`; manual edits only update `m.text`.
  // When blocks carry no text (e.g. tool-only replies), surface edited `m.text`.
  const showMessageText =
    hasText && (!useBlocksRendering || (m.text || "").trim() !== blocksText);
  const thinkingContent =
    !useBlocksRendering &&
    isAssistant &&
    typeof m.params?.thinking_content === "string"
      ? m.params.thinking_content.trim()
      : "";

  const MAX_EDIT_IMAGES = 8;

  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(m.text || "");
  const [draftMentions, setDraftMentions] = useState<string[]>([]);
  const [draftImages, setDraftImages] = useState<ImageRefAbs[]>(inputs);
  const [picking, setPicking] = useState(false);
  const [editDragOver, setEditDragOver] = useState(false);
  const editDragDepth = useRef(0);
  const addedDraftIdsRef = useRef<Set<string>>(new Set());
  const taRef = useRef<HTMLTextAreaElement | null>(null);
  const mentionEditorRef = useRef<MentionEditorHandle | null>(null);

  const projects = useProject((s) => s.projects);
  const projectRoot = useMemo(() => {
    const projectId = active?.session.project_id ?? null;
    if (!projectId) return null;
    return projects.find((p) => p.id === projectId)?.path?.trim() || null;
  }, [active, projects]);

  const onMentionEditorChange = (text: string, paths: string[]) => {
    setDraft(text);
    setDraftMentions(paths);
  };

  // Auto-grow the assistant edit textarea (user edits use the mention editor).
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
    const seed = (m.text || "").trim() || blocksText || "";
    setDraft(seed);
    setDraftMentions(isUser ? parseMentionPaths(seed) : []);
    setDraftImages(isUser ? inputs : []);
    addedDraftIdsRef.current = new Set();
    setEditing(true);
  };
  const cancelEdit = async () => {
    const orphans = Array.from(addedDraftIdsRef.current);
    addedDraftIdsRef.current = new Set();
    setDraft(m.text || "");
    setDraftMentions(isUser ? parseMentionPaths(m.text || "") : []);
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
    return (
      types.includes("Files") ||
      types.includes(ATELIER_DRAG_TYPE) ||
      types.includes(READER_FILE_DRAG_TYPE)
    );
  };

  const insertMentionPath = (rawPath: string): boolean => {
    const p = normalizeMentionPath(rawPath);
    if (projectRoot && !isWithinProject(p, projectRoot)) return false;
    mentionEditorRef.current?.insertMention(p);
    return true;
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

    // Files dragged from the reader file explorer become @ mentions; they're
    // project-internal and trusted, so insert them without re-validating.
    const readerPayload = e.dataTransfer?.getData(READER_FILE_DRAG_TYPE);
    if (readerPayload) {
      try {
        const items = JSON.parse(readerPayload) as Array<
          string | { path: string; isDir?: boolean }
        >;
        for (const it of items) {
          if (typeof it === "string") {
            mentionEditorRef.current?.insertMention(it);
          } else if (it && it.path) {
            mentionEditorRef.current?.insertMention(it.path, it.isDir);
          }
        }
      } catch (err) {
        console.warn(err);
      }
      return;
    }

    const files = Array.from(e.dataTransfer?.files || []);
    if (!files.length) return;

    const images = files.filter(isImageFile);
    const others = files.filter((f) => !isImageFile(f));

    if (images.length) ingestFiles(images);

    if (others.length) {
      let outside = 0;
      for (const f of others) {
        const p = nativeFilePath(f);
        if (!p) continue;
        if (!insertMentionPath(p)) outside += 1;
      }
      if (outside) toast.error(t("composer.mentionOutsideProject"));
    }
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
    const ok = await dialog.confirm(t("message.deleteConfirm"), { type: "danger", confirmLabel: t("common.delete") });
    if (!ok) return;
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
              suppressText={showMessageText}
            />
          )}

          {!editing && showMessageText && !isError && (
            <div className="text">
              {isUser ? <MentionText text={m.text || ""} /> : m.text}
            </div>
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
              {isUser ? (
                <div
                  className="bubble-edit-editor-wrap"
                  onKeyDown={(e) => {
                    if (e.key === "Escape") {
                      e.preventDefault();
                      cancelEdit();
                    } else if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
                      e.preventDefault();
                      saveEdit();
                    }
                  }}
                >
                  <MentionEditor
                    ref={mentionEditorRef}
                    className="bubble-edit-editor"
                    value={draft}
                    mentions={draftMentions}
                    onChange={onMentionEditorChange}
                    placeholder={t("composer.placeholderDefault")}
                    autoFocus
                  />
                </div>
              ) : (
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
              )}
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

          {isAssistant && !isStreamingDraft && !editing && (
            <MessageTokenUsage usage={m.params?.usage} />
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

export const MessageRow = memo(MessageRowImpl, (prev, next) => {
  return (
    prev.m === next.m &&
    prev.focused === next.focused &&
    prev.onPreviewImage === next.onPreviewImage
  );
});
