import { create } from "zustand";
import { listen } from "@tauri-apps/api/event";
import type {
  AssistantBlock,
  AttachmentDraft,
  MessageAbs,
  ModelParamSettings,
  SessionSummary,
  SessionWithMessagesAbs,
} from "../types";
import { api } from "../api/tauri";
import {
  type ComposerChatMode,
  agentTypeFromComposerMode,
  composerModeFromAgentType,
} from "../config/chatMode";

interface ComposerState {
  prompt: string;
  attachments: AttachmentDraft[];
  pendingAttachments: PendingAttachmentDraft[];
  aspectRatio: string;
  imageSize: string;
  chatMode: ComposerChatMode;
}

interface PendingAttachmentDraft {
  id: string;
  label: string;
  bytes: number | null;
}

interface GenerationStreamPayload {
  session_id: string;
  request_message_id?: string;
  text_delta?: string | null;
  thinking_delta?: string | null;
  /** Backward compatibility for older backend stream events. */
  delta?: string;
}

interface ToolUseEventPayload {
  session_id: string;
  request_message_id?: string;
  type: "tool_use";
  id: string;
  tool: string;
  input: unknown;
}

interface ToolResultEventPayload {
  session_id: string;
  request_message_id?: string;
  type: "tool_result";
  id: string;
  output: unknown;
  is_error: boolean;
}

type ToolEventPayload = ToolUseEventPayload | ToolResultEventPayload;

interface SessionStore {
  sessions: SessionSummary[];
  activeId: string | null;
  active: SessionWithMessagesAbs | null;
  busy: boolean;
  busyBySession: Record<string, boolean>;
  composer: ComposerState;

  refreshList: () => Promise<void>;
  createNew: () => Promise<string>;
  switchTo: (id: string) => Promise<void>;
  rename: (id: string, title: string) => Promise<void>;
  updateConfig: (
    id: string,
    systemPrompt: string,
    historyTurns: number,
    llmParams: ModelParamSettings,
  ) => Promise<void>;
  remove: (id: string) => Promise<void>;
  ensureActive: () => Promise<string>;
  reloadActiveSession: () => Promise<void>;

  setPrompt: (s: string) => void;
  setAspectRatio: (s: string) => void;
  setImageSize: (s: string) => void;
  setChatMode: (mode: ComposerChatMode) => Promise<void>;
  addAttachments: (files: File[]) => Promise<void>;
  addAttachmentsFromPaths: (paths: string[]) => Promise<void>;
  addAttachmentFromPath: (path: string) => Promise<void>;
  removeAttachment: (imageId: string) => Promise<void>;
  replaceAttachment: (oldId: string, draft: AttachmentDraft) => void;
  clearComposer: () => void;

  send: () => Promise<void>;
  interrupt: () => Promise<void>;
  resendMessage: (messageId: string) => Promise<void>;
  deleteMessage: (messageId: string) => Promise<void>;
  editMessage: (messageId: string, text: string, imageIds?: string[]) => Promise<void>;
  appendMessages: (msgs: MessageAbs[]) => void;

  quoteMessage: (m: MessageAbs) => Promise<void>;
}

const ACCEPT_TYPES = ["image/png", "image/jpeg", "image/webp"];
const MAX_FILES = 8;
const MAX_BYTES = 50 * 1024 * 1024;

function makePendingAttachment(label: string, bytes: number | null = null): PendingAttachmentDraft {
  return {
    id: `pending-${Date.now()}-${Math.random().toString(36).slice(2)}`,
    label,
    bytes,
  };
}

function pathLabel(path: string) {
  return path.split(/[\\/]/).pop() || "image";
}

function matchesImageRole(role: string) {
  return role === "input" || role === "output" || role === "edited";
}

async function fileToBytes(f: File): Promise<Uint8Array> {
  const buf = await f.arrayBuffer();
  return new Uint8Array(buf);
}

function isGenerationCancelled(e: unknown) {
  return String(e).includes("generation cancelled");
}

/**
 * Extract partial streaming content for the in-flight assistant message
 * of a given session. Returns:
 *
 * - `text`: best-effort concatenation of all text blocks (for legacy
 *   `save_cancelled_message` params that only know about `text`);
 * - `thinking`: same for thinking blocks;
 * - `blocks`: the structured ordered list — what we actually want to
 *   persist on cancel so the rendered UI matches what the user saw.
 */
function extractPartialStreamContent(
  messages: MessageAbs[],
  sessionId: string,
): { text: string; thinking: string; blocks: AssistantBlock[] } {
  const tmp = messages.find(
    (m) => m.session_id === sessionId && m.id.startsWith("tmp-assistant-"),
  );
  const blocks: AssistantBlock[] = tmp?.params?.blocks ?? [];
  const text =
    tmp?.text?.trim() ??
    blocks
      .filter((b): b is { type: "text"; content: string } => b.type === "text")
      .map((b) => b.content)
      .join("")
      .trim();
  const thinkingFromParams =
    typeof tmp?.params?.thinking_content === "string"
      ? tmp.params.thinking_content.trim()
      : "";
  const thinking =
    thinkingFromParams ||
    blocks
      .filter(
        (b): b is { type: "thinking"; content: string } => b.type === "thinking",
      )
      .map((b) => b.content)
      .join("")
      .trim();
  return { text, thinking, blocks };
}

export const useSession = create<SessionStore>((set, get) => {
  const setSessionBusy = (sessionId: string, busy: boolean) => {
    set((state) => {
      const busyBySession = { ...state.busyBySession };
      if (busy) {
        busyBySession[sessionId] = true;
      } else {
        delete busyBySession[sessionId];
      }
      return {
        busyBySession,
        busy: state.activeId ? !!busyBySession[state.activeId] : false,
      };
    });
  };

  const updateActiveSession = (
    sessionId: string,
    update: (active: SessionWithMessagesAbs) => SessionWithMessagesAbs,
  ) => {
    const active = get().active;
    if (!active || get().activeId !== sessionId) return;
    set({ active: update(active) });
  };

  /** Replace active chat with server truth (avoids duplicate/stale merges after async gen or tab switches). */
  const reloadActiveSessionIfViewing = async (sessionId: string) => {
    if (get().activeId !== sessionId) return;
    try {
      const data = await api.loadSession(sessionId);
      // Generation is complete — discard the streaming buffer so future
      // switchTo calls show the persisted DB content, not stale temp data.
      streamingBuffers.delete(sessionId);
      set({
        active: data,
        composer: {
          ...get().composer,
          chatMode: composerModeFromAgentType(data.session.agent_type),
        },
      });
    } catch (e) {
      console.warn(e);
    }
  };

  return ({
  sessions: [],
  activeId: null,
  active: null,
  busy: false,
  busyBySession: {},
  composer: {
    prompt: "",
    attachments: [],
    pendingAttachments: [],
    aspectRatio: "auto",
    imageSize: "auto",
    chatMode: "agent",
  },

  refreshList: async () => {
    const list = await api.listSessions();
    set({ sessions: list });
  },

  createNew: async () => {
    const s = await api.createSession();
    await get().refreshList();
    await get().switchTo(s.id);
    return s.id;
  },

  switchTo: async (id) => {
    // Save the current session's composer draft before switching away.
    const currentId = get().activeId;
    if (currentId && currentId !== id) {
      saveComposerDraft(currentId, get().composer);
    }

    const data = await api.loadSession(id);
    const isBusy = !!get().busyBySession[id];

    // If this session is still generating, restore any buffered streaming
    // content that accumulated while the user was viewing another session.
    let messagesWithBuffer = data.messages;
    if (isBusy) {
      const buf = streamingBuffers.get(id);
      if (buf && buf.blocks.length > 0) {
        messagesWithBuffer = applyStreamBufferToMessages(data.messages, id, buf);
      }
    }

    // Restore draft for the target session, falling back to empty defaults.
    const draft = composerDrafts.get(id);
    set({
      activeId: id,
      active: { ...data, messages: messagesWithBuffer },
      busy: isBusy,
      composer: {
        ...get().composer,
        prompt: draft?.prompt ?? "",
        attachments: draft?.attachments ?? [],
        pendingAttachments: [],
        aspectRatio: draft?.aspectRatio ?? get().composer.aspectRatio,
        imageSize: draft?.imageSize ?? get().composer.imageSize,
        chatMode: composerModeFromAgentType(data.session.agent_type),
      },
    });
  },

  rename: async (id, title) => {
    await api.renameSession(id, title);
    await get().refreshList();
    if (get().activeId === id && get().active) {
      const a = get().active!;
      set({ active: { ...a, session: { ...a.session, title } } });
    }
  },

  updateConfig: async (id, systemPrompt, historyTurns, llmParams) => {
    await api.updateSessionConfig(id, systemPrompt, historyTurns, llmParams);
    await get().refreshList();
    if (get().activeId === id && get().active) {
      const a = get().active!;
      set({
        active: {
          ...a,
          session: {
            ...a.session,
            system_prompt: systemPrompt,
            history_turns: historyTurns,
            llm_params: llmParams,
          },
        },
      });
    }
  },

  remove: async (id) => {
    await api.deleteSession(id);
    if (get().activeId === id) {
      set({ activeId: null, active: null, busy: false });
    }
    await get().refreshList();
  },

  ensureActive: async () => {
    const cur = get().activeId;
    if (cur) return cur;
    return await get().createNew();
  },

  reloadActiveSession: async () => {
    const id = get().activeId;
    if (!id) return;
    try {
      const data = await api.loadSession(id);
      set({
        active: data,
        composer: {
          ...get().composer,
          chatMode: composerModeFromAgentType(data.session.agent_type),
        },
      });
    } catch (e) {
      console.warn(e);
    }
  },

  setPrompt: (s) => set({ composer: { ...get().composer, prompt: s } }),
  setAspectRatio: (s) => set({ composer: { ...get().composer, aspectRatio: s } }),
  setImageSize: (s) => set({ composer: { ...get().composer, imageSize: s } }),

  setChatMode: async (mode) => {
    const id = get().activeId;
    if (!id) {
      set({ composer: { ...get().composer, chatMode: mode } });
      return;
    }
    try {
      await api.setSessionAgentType(id, agentTypeFromComposerMode(mode));
      await get().refreshList();
      await get().reloadActiveSession();
    } catch (e) {
      console.warn(e);
    }
  },

  addAttachments: async (files) => {
    const sid = await get().ensureActive();
    const cur = get().composer;
    const room = MAX_FILES - cur.attachments.length - cur.pendingAttachments.length;
    const uploads: Array<{ file: File; pending: PendingAttachmentDraft }> = [];
    let rejected = 0;
    for (const f of files) {
      if (uploads.length >= room) {
        rejected++;
        continue;
      }
      if (!ACCEPT_TYPES.includes(f.type)) {
        rejected++;
        continue;
      }
      if (f.size > MAX_BYTES) {
        rejected++;
        continue;
      }
      uploads.push({
        file: f,
        pending: makePendingAttachment(f.name || "image", f.size),
      });
    }
    if (uploads.length) {
      set({
        composer: {
          ...get().composer,
          pendingAttachments: [
            ...get().composer.pendingAttachments,
            ...uploads.map((x) => x.pending),
          ],
        },
      });
    }
    for (const { file, pending } of uploads) {
      try {
        const bytes = await fileToBytes(file);
        const d = await api.addAttachmentFromBytes(sid, file.name || "image", bytes);
        set({
          composer: {
            ...get().composer,
            pendingAttachments: get().composer.pendingAttachments.filter((p) => p.id !== pending.id),
            attachments: [...get().composer.attachments, d],
          },
        });
      } catch (e) {
        console.error(e);
        rejected++;
        set({
          composer: {
            ...get().composer,
            pendingAttachments: get().composer.pendingAttachments.filter((p) => p.id !== pending.id),
          },
        });
      }
    }
    if (rejected > 0) {
      console.warn(`${rejected} file(s) rejected`);
    }
  },

  addAttachmentsFromPaths: async (paths) => {
    const sid = await get().ensureActive();
    const cur = get().composer;
    const room = MAX_FILES - cur.attachments.length - cur.pendingAttachments.length;
    const uploads = paths.slice(0, Math.max(0, room)).map((path) => ({
      path,
      pending: makePendingAttachment(pathLabel(path), null),
    }));
    let rejected = 0;
    rejected += Math.max(0, paths.length - uploads.length);
    if (uploads.length) {
      set({
        composer: {
          ...get().composer,
          pendingAttachments: [
            ...get().composer.pendingAttachments,
            ...uploads.map((x) => x.pending),
          ],
        },
      });
    }
    for (const { path, pending } of uploads) {
      try {
        const d = await api.addAttachmentFromPath(sid, path);
        set({
          composer: {
            ...get().composer,
            pendingAttachments: get().composer.pendingAttachments.filter((p) => p.id !== pending.id),
            attachments: [...get().composer.attachments, d],
          },
        });
      } catch (e) {
        console.error(e);
        rejected++;
        set({
          composer: {
            ...get().composer,
            pendingAttachments: get().composer.pendingAttachments.filter((p) => p.id !== pending.id),
          },
        });
      }
    }
    if (rejected > 0) {
      console.warn(`${rejected} file(s) rejected`);
    }
  },

  addAttachmentFromPath: async (path) => {
    await get().addAttachmentsFromPaths([path]);
  },

  removeAttachment: async (imageId) => {
    try {
      await api.removeAttachmentDraft(imageId);
    } catch (e) {
      console.warn(e);
    }
    set({
      composer: {
        ...get().composer,
        attachments: get().composer.attachments.filter((a) => a.image_id !== imageId),
      },
    });
  },

  replaceAttachment: (oldId, draft) => {
    set({
      composer: {
        ...get().composer,
        attachments: get().composer.attachments.map((a) =>
          a.image_id === oldId ? draft : a,
        ),
      },
    });
  },

  clearComposer: () => {
    set({
      composer: {
        ...get().composer,
        prompt: "",
        attachments: [],
        pendingAttachments: [],
      },
    });
  },

  appendMessages: (msgs) => {
    const a = get().active;
    if (!a) return;
    set({ active: { ...a, messages: [...a.messages, ...msgs] } });
  },

  quoteMessage: async (m) => {
    const a = get().active;
    if (!a || m.session_id !== a.session.id) return;

    const quotableImages = m.images.filter((img) =>
      matchesImageRole(img.role),
    );
    const room = MAX_FILES - get().composer.attachments.length - get().composer.pendingAttachments.length;
    const pending = quotableImages
      .slice(0, Math.max(0, room))
      .map((img) => makePendingAttachment(pathLabel(img.rel_path), img.bytes));
    if (pending.length) {
      set({
        composer: {
          ...get().composer,
          pendingAttachments: [...get().composer.pendingAttachments, ...pending],
        },
      });
    }

    try {
      const drafts = await api.quoteMessageAsAttachments(a.session.id, m.id);
      const cur = get().composer.attachments;
      const room = MAX_FILES - cur.length;
      const toAdd = drafts.slice(0, Math.max(0, room));
      const newAtt = [...cur, ...toAdd];

      let prompt = get().composer.prompt;
      const text = (m.text || "").trim();
      if (text) {
        const quoted = text
          .split("\n")
          .map((line) => `> ${line}`)
          .join("\n");
        const head = `${quoted}\n\n`;
        prompt = prompt.trim() ? `${head}${prompt}` : head;
      }

      set({
        composer: {
          ...get().composer,
          attachments: newAtt,
          pendingAttachments: get().composer.pendingAttachments.filter(
            (p) => !pending.some((x) => x.id === p.id),
          ),
          prompt,
        },
      });
      if (drafts.length > room) {
        console.warn("Some images were skipped (max 8 attachments)");
      }
    } catch (e) {
      console.error(e);
      set({
        composer: {
          ...get().composer,
          pendingAttachments: get().composer.pendingAttachments.filter(
            (p) => !pending.some((x) => x.id === p.id),
          ),
        },
      });
    }
  },

  deleteMessage: async (messageId) => {
    const sid = get().active?.session.id;
    try {
      await api.deleteMessage(messageId);
    } catch (e) {
      console.warn(e);
      return;
    }
    if (sid) await reloadActiveSessionIfViewing(sid);
    await get().refreshList();
  },

  resendMessage: async (messageId) => {
    const a = get().active;
    if (!a) return;
    const sid = a.session.id;
    const idx = a.messages.findIndex((x) => x.id === messageId);
    if (idx < 0) return;
    const m = a.messages[idx];
    if (m.role !== "user") return;
    const text = (m.text || "").trim();
    if (!text) return;
    if (get().busyBySession[sid]) return;

    const toDelete: string[] = [];
    for (let j = idx + 1; j < a.messages.length; j++) {
      const next = a.messages[j];
      if (next.role === "assistant" || next.role === "error") {
        toDelete.push(next.id);
      } else {
        break;
      }
    }

    for (const id of toDelete) {
      try {
        await api.deleteMessage(id);
      } catch (e) {
        console.warn(e);
      }
    }
    updateActiveSession(sid, (active) => ({
      ...active,
      messages: active.messages.filter((x) => !toDelete.includes(x.id)),
    }));

    const c = get().composer;
    setSessionBusy(sid, true);
    ensureGenerationStreamListener();
    try {
      await api.regenerateImage(
        {
          session_id: sid,
          user_message_id: messageId,
          aspect_ratio: c.aspectRatio,
          image_size: c.imageSize,
        },
        a.session,
      );
      await reloadActiveSessionIfViewing(sid);
      await get().refreshList();
    } catch (e: unknown) {
      if (isGenerationCancelled(e)) {
        const partial = extractPartialStreamContent(get().active?.messages ?? [], sid);
        if (partial.text || partial.thinking || partial.blocks.length > 0) {
          try {
            await api.saveCancelledMessage(
              sid,
              partial.text,
              partial.thinking,
              partial.blocks.length > 0 ? partial.blocks : null,
            );
          } catch (saveErr) {
            console.warn("Failed to save partial message", saveErr);
          }
        }
        await reloadActiveSessionIfViewing(sid);
        await get().refreshList();
        return;
      }
      console.error(e);
      await reloadActiveSessionIfViewing(sid);
      await get().refreshList();
    } finally {
      setSessionBusy(sid, false);
    }
  },

  editMessage: async (messageId, text, imageIds) => {
    const trimmed = text.trim();
    if (!trimmed && (!imageIds || imageIds.length === 0)) return;
    let updated: MessageAbs | null = null;
    try {
      await api.updateMessageText(messageId, trimmed);
      if (imageIds) {
        updated = await api.updateMessageImages(messageId, imageIds);
      }
    } catch (e) {
      console.warn(e);
      return;
    }
    const a = get().active;
    if (a) {
      set({
        active: {
          ...a,
          messages: a.messages.map((m) =>
            m.id === messageId
              ? updated
                ? { ...updated, text: trimmed }
                : { ...m, text: trimmed }
              : m,
          ),
        },
      });
    }
  },

  send: async () => {
    const c = get().composer;
    const text = c.prompt.trim();
    if (!text) return;
    if (c.pendingAttachments.length > 0) return;
    const sid = await get().ensureActive();
    if (get().busyBySession[sid]) return;

    const optimisticId = `tmp-user-${Date.now()}`;
    const optimisticUser: MessageAbs = {
      id: optimisticId,
      session_id: sid,
      role: "user",
      text,
      params: { aspect_ratio: c.aspectRatio, image_size: c.imageSize },
      created_at: Date.now(),
      images: c.attachments.map((a, i) => ({
        id: a.image_id,
        role: "input",
        rel_path: a.rel_path,
        thumb_rel_path: a.thumb_rel_path,
        abs_path: a.abs_path,
        thumb_abs_path: a.thumb_abs_path,
        mime: a.mime,
        width: a.width,
        height: a.height,
        bytes: a.bytes,
        ord: i,
      })),
    };

    const attachmentIds = c.attachments.map((a) => a.image_id);
    const aspectRatio = c.aspectRatio;
    const imageSize = c.imageSize;

    updateActiveSession(sid, (active) => ({
      ...active,
      messages: [...active.messages, optimisticUser],
    }));
    set({
      composer: { ...get().composer, prompt: "", attachments: [], pendingAttachments: [] },
    });
    // Clear the saved draft once the message is sent.
    composerDrafts.delete(sid);
    setSessionBusy(sid, true);
    ensureGenerationStreamListener();

    try {
      const active = get().active;
      const sessionForLog =
        active && active.session.id === sid ? active.session : null;
      await api.generateImage(
        {
          session_id: sid,
          prompt: text,
          attachment_ids: attachmentIds,
          aspect_ratio: aspectRatio,
          image_size: imageSize,
        },
        sessionForLog,
      );
      await reloadActiveSessionIfViewing(sid);
      await get().refreshList();
    } catch (e: unknown) {
      if (isGenerationCancelled(e)) {
        // Persist any partial streaming content before reloading from DB.
        const partial = extractPartialStreamContent(get().active?.messages ?? [], sid);
        if (partial.text || partial.thinking || partial.blocks.length > 0) {
          try {
            await api.saveCancelledMessage(
              sid,
              partial.text,
              partial.thinking,
              partial.blocks.length > 0 ? partial.blocks : null,
            );
          } catch (saveErr) {
            console.warn("Failed to save partial message", saveErr);
          }
        }
        try {
          await reloadActiveSessionIfViewing(sid);
          await get().refreshList();
        } catch (reloadError) {
          console.warn(reloadError);
        }
        return;
      }
      console.error(e);
      await reloadActiveSessionIfViewing(sid);
      await get().refreshList();
    } finally {
      setSessionBusy(sid, false);
    }
  },

  interrupt: async () => {
    const sid = get().activeId;
    if (!sid || !get().busyBySession[sid]) {
      console.log("[atelier] 中断对话：未执行（无会话或未在生成中）", {
        sessionId: sid ?? null,
        busyBySession: sid ? get().busyBySession[sid] : undefined,
      });
      return;
    }
    console.log("[atelier] 中断对话：调用 cancel_generation", { sessionId: sid });
    try {
      await api.cancelGeneration(sid);
      console.log("[atelier] 中断对话：cancel_generation 已返回", { sessionId: sid });
    } catch (e) {
      console.warn("[atelier] 中断对话：cancel_generation 失败", e);
    }
  },
  });
});

// ─── Per-session composer drafts ─────────────────────────────────────────────
// Saves each session's input state so switching sessions preserves whatever
// the user had typed or attached.

interface ComposerDraft {
  prompt: string;
  attachments: AttachmentDraft[];
  aspectRatio: string;
  imageSize: string;
}

const composerDrafts = new Map<string, ComposerDraft>();

function saveComposerDraft(sessionId: string, composer: ComposerState) {
  composerDrafts.set(sessionId, {
    prompt: composer.prompt,
    attachments: composer.attachments,
    aspectRatio: composer.aspectRatio,
    imageSize: composer.imageSize,
  });
}

// ─── Per-session streaming buffers ───────────────────────────────────────────
// Keeps accumulating stream events even when the session isn't active so that
// switching back to a generating session immediately shows what was produced
// while the user was away.
//
// Streams flow in strict arrival order — text deltas, thinking deltas, and
// tool events all share one ordered `blocks` array. The merge/split rules
// here are the single source of truth for how the renderer perceives the
// agent's multi-turn output.

interface StreamBuffer {
  blocks: AssistantBlock[];
  requestId: string;
}

const streamingBuffers = new Map<string, StreamBuffer>();

/**
 * Append a text/thinking delta to a block list following the rule
 * "merge with last block when it's the same kind, otherwise push new".
 * Mutates `blocks` in place. Returns nothing — block ordering matters
 * and callers always operate on a fresh array clone.
 */
function appendDelta(
  blocks: AssistantBlock[],
  kind: "text" | "thinking",
  delta: string,
) {
  if (!delta) return;
  const last = blocks[blocks.length - 1];
  if (last && last.type === kind) {
    last.content = `${last.content}${delta}`;
    return;
  }
  blocks.push({ type: kind, content: delta });
}

/**
 * Record a structured tool event into the block list. `tool_use` always
 * pushes a new pending block; `tool_result` mutates the matching `tool_use`
 * block in place so the on-screen card transitions from pending → done
 * without changing the surrounding order.
 */
function applyToolEvent(blocks: AssistantBlock[], event: ToolEventPayload) {
  if (event.type === "tool_use") {
    blocks.push({
      type: "tool_use",
      id: event.id,
      tool: event.tool,
      input: event.input,
      status: "pending",
    });
    return;
  }
  for (let i = blocks.length - 1; i >= 0; i--) {
    const b = blocks[i];
    if (b.type === "tool_use" && b.id === event.id) {
      blocks[i] = {
        ...b,
        status: event.is_error ? "error" : "success",
        output: event.output,
        is_error: event.is_error || undefined,
      };
      return;
    }
  }
}

/** Apply a stream buffer as a temporary assistant message into active messages. */
function applyStreamBufferToMessages(
  messages: MessageAbs[],
  sessionId: string,
  buf: StreamBuffer,
): MessageAbs[] {
  const messageId = `tmp-assistant-${buf.requestId}`;
  const idx = messages.findIndex((m) => m.id === messageId);
  const text = buf.blocks
    .filter((b): b is { type: "text"; content: string } => b.type === "text")
    .map((b) => b.content)
    .join("");
  const thinking = buf.blocks
    .filter(
      (b): b is { type: "thinking"; content: string } => b.type === "thinking",
    )
    .map((b) => b.content)
    .join("");
  const tmpMsg: MessageAbs = {
    id: messageId,
    session_id: sessionId,
    role: "assistant",
    text: text || null,
    params: {
      ...(thinking ? { thinking_content: thinking } : {}),
      blocks: buf.blocks.map((b) => ({ ...b })),
    },
    created_at: Date.now(),
    images: [],
  };
  if (idx >= 0) {
    const next = [...messages];
    next[idx] = tmpMsg;
    return next;
  }
  return [...messages, tmpMsg];
}

/**
 * Mutate the live `active.messages` slot for a session so the streaming
 * tmp-assistant message reflects the latest StreamBuffer. Both listeners
 * funnel through this so they always agree on the message shape.
 */
function syncStreamingMessage(sessionId: string, buf: StreamBuffer) {
  const state = useSession.getState();
  if (state.activeId !== sessionId || !state.active) return;
  const messages = applyStreamBufferToMessages(
    state.active.messages,
    sessionId,
    buf,
  );
  useSession.setState({
    active: {
      ...state.active,
      messages,
    },
  });
}

// ─── Stream listeners ─────────────────────────────────────────────────────────

let generationStreamListenerStarted = false;
let toolEventListenerStarted = false;

function ensureGenerationStreamListener() {
  if (!generationStreamListenerStarted) {
    generationStreamListenerStarted = true;
    listen<GenerationStreamPayload>("gen://stream", (event) => {
      const payload = event.payload;
      const sessionId = payload.session_id;
      const textDelta = payload.text_delta ?? payload.delta ?? "";
      const thinkingDelta = payload.thinking_delta ?? "";
      if (!sessionId || (!textDelta && !thinkingDelta)) return;

      const requestId = payload.request_message_id || sessionId;
      const prev = streamingBuffers.get(sessionId) ?? {
        blocks: [],
        requestId,
      };
      // Always work on a fresh array so React sees a new reference.
      const nextBlocks = prev.blocks.map((b) => ({ ...b }));
      if (thinkingDelta) appendDelta(nextBlocks, "thinking", thinkingDelta);
      if (textDelta) appendDelta(nextBlocks, "text", textDelta);
      const next: StreamBuffer = { blocks: nextBlocks, requestId };
      streamingBuffers.set(sessionId, next);
      syncStreamingMessage(sessionId, next);
    }).catch((e) => {
      generationStreamListenerStarted = false;
      console.warn(e);
    });
  }

  if (!toolEventListenerStarted) {
    toolEventListenerStarted = true;
    listen<ToolEventPayload>("gen://tool", (event) => {
      const payload = event.payload;
      const sessionId = payload.session_id;
      if (!sessionId) return;
      const requestId = payload.request_message_id || sessionId;
      const prev = streamingBuffers.get(sessionId) ?? {
        blocks: [],
        requestId,
      };
      const nextBlocks = prev.blocks.map((b) => ({ ...b }));
      applyToolEvent(nextBlocks, payload);
      const next: StreamBuffer = { blocks: nextBlocks, requestId };
      streamingBuffers.set(sessionId, next);
      syncStreamingMessage(sessionId, next);
    }).catch((e) => {
      toolEventListenerStarted = false;
      console.warn(e);
    });
  }
}

ensureGenerationStreamListener();
