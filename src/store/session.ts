import { create } from "zustand";
import { listen } from "@tauri-apps/api/event";
import type {
  AttachmentDraft,
  MessageAbs,
  ModelParamSettings,
  SessionSummary,
  SessionWithMessagesAbs,
} from "../types";
import { api } from "../api/tauri";

interface ComposerState {
  prompt: string;
  attachments: AttachmentDraft[];
  pendingAttachments: PendingAttachmentDraft[];
  aspectRatio: string;
  imageSize: string;
}

interface PendingAttachmentDraft {
  id: string;
  label: string;
  bytes: number | null;
}

interface GenerationStreamPayload {
  session_id: string;
  request_message_id?: string;
  delta?: string;
}

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

  setPrompt: (s: string) => void;
  setAspectRatio: (s: string) => void;
  setImageSize: (s: string) => void;
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
      set({ active: await api.loadSession(sessionId) });
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
    const data = await api.loadSession(id);
    set({
      activeId: id,
      active: data,
      busy: !!get().busyBySession[id],
      composer: {
        ...get().composer,
        attachments: [],
        pendingAttachments: [],
        prompt: "",
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

  setPrompt: (s) => set({ composer: { ...get().composer, prompt: s } }),
  setAspectRatio: (s) => set({ composer: { ...get().composer, aspectRatio: s } }),
  setImageSize: (s) => set({ composer: { ...get().composer, imageSize: s } }),

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
      await api.regenerateImage({
        session_id: sid,
        user_message_id: messageId,
        aspect_ratio: c.aspectRatio,
        image_size: c.imageSize,
      });
      await reloadActiveSessionIfViewing(sid);
      await get().refreshList();
    } catch (e: unknown) {
      if (isGenerationCancelled(e)) {
        await reloadActiveSessionIfViewing(sid);
        await get().refreshList();
        return;
      }
      console.error(e);
      await reloadActiveSessionIfViewing(sid);
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
    setSessionBusy(sid, true);
    ensureGenerationStreamListener();

    try {
      await api.generateImage({
        session_id: sid,
        prompt: text,
        attachment_ids: attachmentIds,
        aspect_ratio: aspectRatio,
        image_size: imageSize,
      });
      await reloadActiveSessionIfViewing(sid);
      await get().refreshList();
    } catch (e: unknown) {
      if (isGenerationCancelled(e)) {
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
    } finally {
      setSessionBusy(sid, false);
    }
  },

  interrupt: async () => {
    const sid = get().activeId;
    if (!sid || !get().busyBySession[sid]) return;
    try {
      await api.cancelGeneration(sid);
    } catch (e) {
      console.warn(e);
    }
  },
  });
});

let generationStreamListenerStarted = false;

function ensureGenerationStreamListener() {
  if (generationStreamListenerStarted) return;
  generationStreamListenerStarted = true;
  listen<GenerationStreamPayload>("gen://stream", (event) => {
    const payload = event.payload;
    const sessionId = payload.session_id;
    const delta = payload.delta || "";
    if (!sessionId || !delta) return;

    const state = useSession.getState();
    if (state.activeId !== sessionId || !state.active) return;

    const requestId = payload.request_message_id || sessionId;
    const messageId = `tmp-assistant-${requestId}`;
    const existingIndex = state.active.messages.findIndex((m) => m.id === messageId);
    const messages = [...state.active.messages];

    if (existingIndex >= 0) {
      const existing = messages[existingIndex];
      messages[existingIndex] = {
        ...existing,
        text: `${existing.text || ""}${delta}`,
      };
    } else {
      messages.push({
        id: messageId,
        session_id: sessionId,
        role: "assistant",
        text: delta,
        params: null,
        created_at: Date.now(),
        images: [],
      });
    }

    useSession.setState({
      active: {
        ...state.active,
        messages,
      },
    });
  }).catch((e) => {
    generationStreamListenerStarted = false;
    console.warn(e);
  });
}

ensureGenerationStreamListener();
