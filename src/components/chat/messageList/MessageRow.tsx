import {
  memo,
  useCallback,
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
import { ComposerFileTree } from "../ComposerFileTree";
import {
  MentionEditor,
  MentionIcon,
  MentionText,
  isWithinProject,
  mediaMentionDisplayLabel,
  mediaMentionKindFromMime,
  mediaMentionLabel,
  normalizeMentionPath,
  parseMentionPaths,
  type MentionEditorHandle,
  type MentionTriggerAnchor,
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
  const inputMediaByPath = useMemo(() => {
    const counts = { image: 0, audio: 0, video: 0 };
    const media: Record<string, { previewSrc?: string }> = {};
    for (const item of inputs) {
      const kind = mediaMentionKindFromMime(item.mime);
      if (!kind) continue;
      counts[kind] += 1;
      const label = mediaMentionLabel(kind, counts[kind]);
      media[label] =
        kind === "image"
          ? {
              previewSrc: srcOf(item.thumb_abs_path || item.abs_path),
            }
          : {};
    }
    return media;
  }, [inputs]);
  const outputs = m.images.filter((i) => i.role === "output");
  const hasVideoOutput = outputs.some((item) => item.mime.startsWith("video/"));
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
  const canQuote =
    hasText ||
    inputs.some((item) => item.mime.startsWith("image/")) ||
    outputs.some((item) => item.mime.startsWith("image/"));
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

  const MAX_EDIT_MEDIA =
    m.params?.video_mode === "text"
      ? 0
      : m.params?.video_mode === "first_frame"
        ? 1
        : m.params?.video_mode === "first_last"
          ? 2
          : m.params?.video_mode === "reference"
            ? 15
            : 8;

  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(m.text || "");
  const [draftMentions, setDraftMentions] = useState<string[]>([]);
  const [draftMedia, setDraftMedia] = useState<ImageRefAbs[]>(inputs);
  const draftMediaByPath = useMemo(() => {
    const counts = { image: 0, audio: 0, video: 0 };
    const media: Record<string, { previewSrc?: string }> = {};
    for (const item of draftMedia) {
      const kind = mediaMentionKindFromMime(item.mime);
      if (!kind) continue;
      counts[kind] += 1;
      const label = mediaMentionLabel(kind, counts[kind]);
      media[label] =
        kind === "image"
          ? {
              previewSrc: srcOf(item.thumb_abs_path || item.abs_path),
            }
          : {};
    }
    return media;
  }, [draftMedia]);
  const [editMentionAnchor, setEditMentionAnchor] =
    useState<MentionTriggerAnchor | null>(null);
  const [picking, setPicking] = useState(false);
  const [editDragOver, setEditDragOver] = useState(false);
  const editDragDepth = useRef(0);
  const addedDraftIdsRef = useRef<Set<string>>(new Set());
  const autoMentionedDraftIdsRef = useRef<Set<string>>(new Set());
  const taRef = useRef<HTMLTextAreaElement | null>(null);
  const mentionEditorRef = useRef<MentionEditorHandle | null>(null);
  const editMentionPanelRef = useRef<HTMLDivElement | null>(null);

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
  const onEditMentionTrigger = useCallback(
    (anchor: MentionTriggerAnchor | null) => {
      setEditMentionAnchor(anchor);
    },
    [],
  );

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

  useEffect(() => {
    if (!editing || !editMentionAnchor) return;
    const onMouseDown = (event: MouseEvent) => {
      if (
        editMentionPanelRef.current &&
        !editMentionPanelRef.current.contains(event.target as Node)
      ) {
        setEditMentionAnchor(null);
      }
    };
    window.addEventListener("mousedown", onMouseDown);
    return () => window.removeEventListener("mousedown", onMouseDown);
  }, [editing, editMentionAnchor]);

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
    setDraftMedia(isUser ? inputs : []);
    autoMentionedDraftIdsRef.current = new Set(
      isUser ? inputs.map((item) => item.id) : [],
    );
    setEditMentionAnchor(null);
    addedDraftIdsRef.current = new Set();
    setEditing(true);
  };
  const cancelEdit = async () => {
    const orphans = Array.from(addedDraftIdsRef.current);
    addedDraftIdsRef.current = new Set();
    setDraft(m.text || "");
    setDraftMentions(isUser ? parseMentionPaths(m.text || "") : []);
    setDraftMedia(isUser ? inputs : []);
    autoMentionedDraftIdsRef.current = new Set(
      isUser ? inputs.map((item) => item.id) : [],
    );
    setEditMentionAnchor(null);
    setEditing(false);
    if (orphans.length) await cleanupAddedDrafts(orphans);
  };

  const draftToMediaRef = (d: AttachmentDraft): ImageRefAbs => ({
    id: d.image_id,
    role: "input",
    rel_path: d.rel_path,
    thumb_rel_path: d.thumb_rel_path,
    abs_path: d.abs_path,
    thumb_abs_path: d.thumb_abs_path,
    mime: d.mime,
    media_role: d.media_role,
    source_url: d.source_url,
    width: d.width,
    height: d.height,
    bytes: d.bytes,
    ord: 0,
  });

  const canAcceptDraftMedia = (
    mime: string,
    current: ImageRefAbs[] = draftMedia,
  ) => {
    const mode = m.params?.video_mode;
    const imageCount = current.filter((item) =>
      item.mime.startsWith("image/"),
    ).length;
    const audioCount = current.filter((item) =>
      item.mime.startsWith("audio/"),
    ).length;
    if (mime.startsWith("video/")) return false;
    if (mode === "text") return false;
    if (mode === "first_frame" || mode === "first_last") {
      return (
        mime.startsWith("image/") &&
        imageCount < (mode === "first_frame" ? 1 : 2)
      );
    }
    if (mode === "reference") {
      if (mime.startsWith("image/")) return imageCount < 9;
      if (mime.startsWith("audio/")) return audioCount < 3;
      return false;
    }
    return mime.startsWith("image/") && current.length < MAX_EDIT_MEDIA;
  };

  const mediaMentionForDraft = (item: ImageRefAbs) => {
    const kind = mediaMentionKindFromMime(item.mime);
    if (!kind) return null;
    const ordinal = draftMedia
      .filter((media) => mediaMentionKindFromMime(media.mime) === kind)
      .findIndex((media) => media.id === item.id);
    if (ordinal < 0) return null;
    return mediaMentionLabel(kind, ordinal + 1);
  };

  const ingestPaths = async (paths: string[]) => {
    const room = MAX_EDIT_MEDIA - draftMedia.length;
    if (room <= 0) return;
    const toIngest = paths.slice(0, room);
    const workingMedia = [...draftMedia];
    for (const p of toIngest) {
      try {
        const d = await api.addAttachmentFromPath(m.session_id, p);
        const ref = draftToMediaRef(d);
        if (!canAcceptDraftMedia(ref.mime, workingMedia)) {
          await api.removeAttachmentDraft(ref.id).catch(() => {});
          continue;
        }
        addedDraftIdsRef.current.add(ref.id);
        workingMedia.push(ref);
        setDraftMedia((arr) => [...arr, ref]);
      } catch (e) {
        console.warn(e);
      }
    }
  };

  const ingestFiles = async (files: File[]) => {
    const room = MAX_EDIT_MEDIA - draftMedia.length;
    if (room <= 0) return;
    const toIngest = files.slice(0, room);
    const workingMedia = [...draftMedia];
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
        const ref = draftToMediaRef(d);
        if (!canAcceptDraftMedia(ref.mime, workingMedia)) {
          await api.removeAttachmentDraft(ref.id).catch(() => {});
          continue;
        }
        addedDraftIdsRef.current.add(ref.id);
        workingMedia.push(ref);
        setDraftMedia((arr) => [...arr, ref]);
      } catch (e) {
        console.warn(e);
      }
    }
  };

  const ingestExistingMedia = async (absPath: string) => {
    if (!absPath) return;
    if (draftMedia.length >= MAX_EDIT_MEDIA) return;
    try {
      const d = await api.addAttachmentFromPath(m.session_id, absPath);
      const ref = draftToMediaRef(d);
      if (!canAcceptDraftMedia(ref.mime)) {
        await api.removeAttachmentDraft(ref.id).catch(() => {});
        return;
      }
      addedDraftIdsRef.current.add(ref.id);
      setDraftMedia((arr) => [...arr, ref]);
    } catch (e) {
      console.warn(e);
    }
  };

  const addDraftMedia = async () => {
    if (picking) return;
    if (draftMedia.length >= MAX_EDIT_MEDIA) return;
    mentionEditorRef.current?.rememberSelection();
    setPicking(true);
    try {
      const extensions =
        m.params?.video_mode === "reference"
          ? ["png", "jpg", "jpeg", "webp", "bmp", "gif", "tif", "tiff", "wav", "mp3"]
          : ["png", "jpg", "jpeg", "webp", "bmp", "gif", "tif", "tiff"];
      const selected = await openDialog({
        multiple: true,
        filters: [{ name: t("message.editAddMedia"), extensions }],
      });
      const paths = Array.isArray(selected) ? selected : selected ? [selected] : [];
      if (paths.length) await ingestPaths(paths as string[]);
    } finally {
      setPicking(false);
    }
  };

  const removeDraftMedia = async (id: string, removeMention = true) => {
    const item = draftMedia.find((media) => media.id === id);
    if (removeMention && item) {
      const mention = mediaMentionForDraft(item);
      if (mention) {
        mentionEditorRef.current?.removeAllMentions(mention, false);
      }
    }
    setDraftMedia((arr) => arr.filter((media) => media.id !== id));
    if (addedDraftIdsRef.current.has(id)) {
      addedDraftIdsRef.current.delete(id);
      try {
        await api.removeAttachmentDraft(id);
      } catch (e) {
        console.warn(e);
      }
    }
  };

  useEffect(() => {
    if (!editing || !isUser) return;
    const currentIds = new Set(draftMedia.map((item) => item.id));
    for (const id of autoMentionedDraftIdsRef.current) {
      if (!currentIds.has(id)) {
        autoMentionedDraftIdsRef.current.delete(id);
      }
    }
    for (const item of draftMedia) {
      if (autoMentionedDraftIdsRef.current.has(item.id)) continue;
      autoMentionedDraftIdsRef.current.add(item.id);
      const mention = mediaMentionForDraft(item);
      if (mention && !draftMentions.includes(mention)) {
        mentionEditorRef.current?.insertMention(mention);
      }
    }
  }, [draftMedia, draftMentions, editing, isUser]);

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
    mentionEditorRef.current?.rememberSelection();
    editDragDepth.current = 0;
    setEditDragOver(false);

    const galleryPayload = e.dataTransfer?.getData(ATELIER_DRAG_TYPE);
    if (galleryPayload) {
      try {
        const parsed = JSON.parse(galleryPayload) as { abs_path?: string };
        if (parsed.abs_path) ingestExistingMedia(parsed.abs_path);
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

    const media = files.filter(
      (file) =>
        isImageFile(file) ||
        (m.params?.video_mode === "reference" &&
          (file.type.startsWith("audio/") ||
            /\.(wav|mp3)$/i.test(file.name))),
    );
    const others = files.filter((file) => !media.includes(file));

    if (media.length) ingestFiles(media);

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

  const mediaIdsEqual = (a: ImageRefAbs[], b: ImageRefAbs[]) => {
    if (a.length !== b.length) return false;
    for (let i = 0; i < a.length; i++) if (a[i].id !== b[i].id) return false;
    return true;
  };

  const saveEdit = async () => {
    const next = draft.trim();
    const mediaChanged = !mediaIdsEqual(draftMedia, inputs);
    if (!next && draftMedia.length === 0) {
      await cancelEdit();
      return;
    }
    if (next === (m.text || "").trim() && !mediaChanged) {
      setEditMentionAnchor(null);
      setEditing(false);
      return;
    }
    await editMessage(
      m.id,
      next,
      mediaChanged ? draftMedia.map((item) => item.id) : undefined,
    );
    addedDraftIdsRef.current = new Set();
    setEditMentionAnchor(null);
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
  const mediaAliasForInput = (item: ImageRefAbs) => {
    const kind = mediaMentionKindFromMime(item.mime);
    if (!kind) return null;
    const ordinal = inputs
      .filter((input) => mediaMentionKindFromMime(input.mime) === kind)
      .findIndex((input) => input.id === item.id);
    return ordinal >= 0 ? mediaMentionLabel(kind, ordinal + 1) : null;
  };
  const displayMediaMention = (value: string | null) =>
    value ? `@${mediaMentionDisplayLabel(value)}` : "";
  const pickEditMediaMention = (item: ImageRefAbs) => {
    const mention = mediaMentionForDraft(item);
    if (!mention) return;
    mentionEditorRef.current?.replaceMentionTrigger(mention);
    setEditMentionAnchor(null);
  };
  const pickEditFileMention = (absPath: string, isDir: boolean) => {
    mentionEditorRef.current?.replaceMentionTrigger(absPath, isDir);
    setEditMentionAnchor(null);
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
                ? t(hasVideoOutput ? "message.stampVideo" : "message.stampImage", {
                    time: nowStamp(m.created_at),
                  })
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
              {inputs.map((item, i) =>
                item.mime.startsWith("image/") ? (
                  <button
                    type="button"
                    className="attached-image-card"
                    key={`in:${item.id}:${i}`}
                    title={item.rel_path}
                    onClick={() => onPreviewImage(item)}
                  >
                    <img
                      src={srcOf(item.thumb_abs_path || item.abs_path)}
                      alt=""
                    />
                  </button>
                ) : item.mime.startsWith("audio/") ? (
                  <div className="attached-audio" key={`in:${item.id}:${i}`}>
                    <span>{displayMediaMention(mediaAliasForInput(item))}</span>
                    <audio controls preload="metadata" src={srcOf(item.abs_path)} />
                  </div>
                ) : (
                  <a
                    className="attached-video-reference"
                    key={`in:${item.id}:${i}`}
                    href={item.source_url || undefined}
                    target="_blank"
                    rel="noreferrer"
                    title={item.source_url || t("message.referenceVideo")}
                  >
                    <span className="attached-video-reference-icon" aria-hidden>
                      ▶
                    </span>
                    <span>{displayMediaMention(mediaAliasForInput(item))}</span>
                    <small>{item.source_url}</small>
                  </a>
                ),
              )}
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
              {isUser ? (
                <MentionText
                  text={m.text || ""}
                  mediaByPath={inputMediaByPath}
                />
              ) : (
                m.text
              )}
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
              {isUser && (draftMedia.length > 0 || picking) && (
                <div className="bubble-edit-images">
                  {draftMedia.map((item, i) => (
                    <div
                      className={`bubble-edit-image ${
                        item.mime.startsWith("image/")
                          ? ""
                          : "bubble-edit-media"
                      }`}
                      key={`draft:${item.id}:${i}`}
                      title={item.source_url || item.rel_path}
                    >
                      {item.mime.startsWith("image/") ? (
                        <img
                          src={srcOf(item.thumb_abs_path || item.abs_path)}
                          alt=""
                          draggable={false}
                        />
                      ) : (
                        <div className="bubble-edit-media-label">
                          {displayMediaMention(mediaMentionForDraft(item))}
                        </div>
                      )}
                      <button
                        type="button"
                        className="bubble-edit-image-remove"
                        title={t("message.editRemoveMedia")}
                        onClick={() => removeDraftMedia(item.id)}
                      >
                        ×
                      </button>
                    </div>
                  ))}
                </div>
              )}
              {isUser ? (
                <div
                  ref={editMentionPanelRef}
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
                  onPaste={(event) => {
                    const files = Array.from(
                      event.clipboardData.files ?? [],
                    );
                    if (files.length) {
                      event.preventDefault();
                      event.stopPropagation();
                      void ingestFiles(files);
                    }
                  }}
                >
                  <MentionEditor
                    ref={mentionEditorRef}
                    className="bubble-edit-editor"
                    value={draft}
                    mentions={draftMentions}
                    onChange={onMentionEditorChange}
                    onMentionTrigger={onEditMentionTrigger}
                    mediaByPath={draftMediaByPath}
                    placeholder={t("composer.placeholderDefault")}
                    autoFocus
                  />
                  {editMentionAnchor && (
                    <div
                      className="composer-mention-popover is-caret bubble-edit-mention-popover"
                      role="dialog"
                      aria-label={t("composer.mentionPickerTitle")}
                      style={{
                        left:
                          Math.max(
                            12,
                            Math.min(
                              editMentionAnchor.left,
                              window.innerWidth - 332,
                            ),
                          ) -
                          (editMentionPanelRef.current?.getBoundingClientRect()
                            .left ?? 0),
                        top:
                          editMentionAnchor.bottom +
                          8 -
                          (editMentionPanelRef.current?.getBoundingClientRect()
                            .top ?? 0),
                        bottom: "auto",
                        maxHeight: Math.max(
                          72,
                          window.innerHeight - editMentionAnchor.bottom - 12,
                        ),
                      }}
                    >
                      <div className="composer-mention-popover-title">
                        {t("composer.mentionPickerTitle")}
                      </div>
                      <div className="composer-mention-popover-body">
                        <section className="composer-mention-section">
                          <div className="composer-mention-section-title">
                            {t("composer.mentionUploadedMedia")}
                          </div>
                          {draftMedia.length > 0 ? (
                            <div className="composer-mention-media-list">
                              {draftMedia.map((item) => {
                                const mention = mediaMentionForDraft(item);
                                if (!mention) return null;
                                const name =
                                  item.rel_path.split(/[\\/]/).pop() ||
                                  mediaMentionDisplayLabel(mention);
                                return (
                                  <button
                                    type="button"
                                    className="composer-mention-media-item"
                                    key={item.id}
                                    title={name}
                                    onClick={() =>
                                      pickEditMediaMention(item)
                                    }
                                  >
                                    {item.mime.startsWith("image/") ? (
                                      <img
                                        src={srcOf(
                                          item.thumb_abs_path ||
                                            item.abs_path,
                                        )}
                                        alt=""
                                        draggable={false}
                                      />
                                    ) : (
                                      <span className="composer-mention-media-icon">
                                        <MentionIcon path={mention} />
                                      </span>
                                    )}
                                    <span className="composer-mention-media-copy">
                                      <strong>
                                        @{mediaMentionDisplayLabel(mention)}
                                      </strong>
                                      <small>{name}</small>
                                    </span>
                                  </button>
                                );
                              })}
                            </div>
                          ) : (
                            <div className="composer-mention-status">
                              {t("composer.mentionNoUploadedMedia")}
                            </div>
                          )}
                        </section>

                        {projectRoot && (
                          <section className="composer-mention-section">
                            <div className="composer-mention-section-title">
                              {t("composer.mentionProjectFiles")}
                            </div>
                            <ComposerFileTree
                              sessionId={m.session_id}
                              projectRoot={projectRoot}
                              onPick={pickEditFileMention}
                            />
                          </section>
                        )}
                      </div>
                    </div>
                  )}
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
                    title={t("message.editAddMedia")}
                    onMouseDown={() =>
                      mentionEditorRef.current?.rememberSelection()
                    }
                    onClick={addDraftMedia}
                    disabled={picking || draftMedia.length >= MAX_EDIT_MEDIA}
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
                      mediaIdsEqual(draftMedia, inputs)) ||
                    (!draft.trim() && draftMedia.length === 0)
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
                img.mime.startsWith("video/") ? (
                  <InlineVideoPlate
                    key={`out:${img.id}:${i}`}
                    item={img}
                    onPreview={() => onPreviewImage(img)}
                  />
                ) : (
                  <div
                    className="plate"
                    key={`out:${img.id}:${i}`}
                    onClick={() => onPreviewImage(img)}
                  >
                    <img src={srcOf(img.abs_path)} alt="generated" />
                  </div>
                )
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
            {isUser && (hasText || inputs.length > 0) && (
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

function InlineVideoPlate({
  item,
  onPreview,
}: {
  item: ImageRefAbs;
  onPreview: () => void;
}) {
  const { t } = useTranslation();
  const videoRef = useRef<HTMLVideoElement | null>(null);
  const [failed, setFailed] = useState(false);
  const [aspectRatio, setAspectRatio] = useState(
    item.width && item.height ? item.width / item.height : 16 / 9,
  );

  return (
    <div className="plate video-plate" style={{ aspectRatio }}>
      {failed ? (
        <button
          type="button"
          className="video-inline-error"
          onClick={() => {
            setFailed(false);
            videoRef.current?.load();
          }}
        >
          {t("message.videoRetry")}
        </button>
      ) : null}
      <video
        ref={videoRef}
        controls
        playsInline
        preload="metadata"
        src={srcOf(item.abs_path)}
        onLoadedMetadata={(event) => {
          const video = event.currentTarget;
          if (video.videoWidth > 0 && video.videoHeight > 0) {
            setAspectRatio(video.videoWidth / video.videoHeight);
          }
        }}
        onError={() => setFailed(true)}
      />
      <button
        type="button"
        className="video-inline-preview"
        title={t("message.videoFullscreen")}
        aria-label={t("message.videoFullscreen")}
        onClick={onPreview}
      >
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8">
          <path d="M8 3H3v5M16 3h5v5M8 21H3v-5M16 21h5v-5" />
        </svg>
      </button>
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
