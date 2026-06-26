import { startTransition } from "react";
import { create } from "zustand";
import { listen } from "@tauri-apps/api/event";
import type {
  AssistantBlock,
  AttachmentDraft,
  ChainEntry,
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
import { useRoleState, type RoleStateOp } from "./roleState";
import {
  applyParagraphEdit,
  revertParagraphEdit,
  useReader,
  readerDocFromToolOutput,
  stripParagraphLabels,
  resolveToolFilePath,
  parseEditParagraphNumber,
  inferFileType,
} from "./reader";
import { useSettings } from "./settings";
import { isDiffTextEqual, normalizeDiffText } from "../utils/inlineDiff";

interface ComposerState {
  prompt: string;
  /** Absolute file paths referenced as `@` mention chips in the prompt. */
  mentions: string[];
  attachments: AttachmentDraft[];
  pendingAttachments: PendingAttachmentDraft[];
  aspectRatio: string;
  imageSize: string;
  /** Composer thinking toggle (only meaningful for reasoning-capable models). */
  thinkingEnabled: boolean;
  /** Reasoning effort; empty string means provider default (high). */
  thinkingEffort: string;
  chatMode: ComposerChatMode;
}

/**
 * Capabilities of the currently active model (global active provider + model).
 * Used to decide whether per-request thinking params should be forwarded.
 */
function activeModelCapabilities(): string[] {
  const settings = useSettings.getState().settings;
  if (!settings) return [];
  const provider = settings.model_services?.find(
    (p) => p.id === settings.active_provider_id,
  );
  const model = provider?.models?.find((m) => m.id === settings.model);
  return model?.capabilities ?? [];
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
  /** Agent flow chain stage boundary marker. */
  stage?: { agent_type: string; name?: string; index?: number };
  /**
   * Live tool-call argument fragment (OpenAI-compatible streaming). `arguments`
   * is the slice received in this chunk, not the accumulated string. Keyed by
   * `id`, which matches the terminal `gen://tool` ToolUse event id.
   */
  tool_call_delta?: {
    id: string;
    name: string;
    arguments: string;
  } | null;
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
  tool?: string;
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
  setMentions: (paths: string[]) => void;
  setAspectRatio: (s: string) => void;
  setImageSize: (s: string) => void;
  setThinkingEnabled: (on: boolean) => void;
  setThinkingEffort: (effort: string) => void;
  setChatMode: (mode: ComposerChatMode) => Promise<void>;
  setAgentChain: (chain: ChainEntry[]) => Promise<void>;
  addAttachments: (files: File[]) => Promise<void>;
  addAttachmentsFromPaths: (paths: string[]) => Promise<void>;
  addAttachmentFromPath: (path: string) => Promise<void>;
  removeAttachment: (imageId: string) => Promise<void>;
  replaceAttachment: (oldId: string, draft: AttachmentDraft) => void;
  clearComposer: () => void;

  send: () => Promise<void>;
  interrupt: () => void;
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

/** Bumped when the user stops, deletes, or resends — stale in-flight runs must not persist/reload. */
const generationEpochBySession = new Map<string, number>();
/** Tracks backend `generate_image` / `regenerate_image` invokes still settling after cancel. */
const generationFlights = new Map<string, Promise<void>>();

function getGenerationEpoch(sessionId: string) {
  return generationEpochBySession.get(sessionId) ?? 0;
}

function bumpGenerationEpoch(sessionId: string) {
  const next = getGenerationEpoch(sessionId) + 1;
  generationEpochBySession.set(sessionId, next);
  cancelStreamFlushRaf(sessionId);
  streamingBuffers.delete(sessionId);
  const state = useSession.getState();
  if (state.activeId === sessionId && state.active) {
    useSession.setState({
      active: {
        ...state.active,
        messages: state.active.messages.filter((m) => !m.id.startsWith("tmp-assistant-")),
      },
    });
  }
  return next;
}

function trackGenerationFlight(sessionId: string, run: Promise<unknown>): Promise<void> {
  const flight = run.then(
    () => {},
    () => {},
  ).finally(() => {
    if (generationFlights.get(sessionId) === flight) {
      generationFlights.delete(sessionId);
    }
  });
  generationFlights.set(sessionId, flight);
  return flight;
}

async function waitForGenerationIdle(sessionId: string) {
  const pending = generationFlights.get(sessionId);
  if (pending) await pending;
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

/** Persist in-flight stream content so a session reload does not wipe the UI. */
async function persistPartialStreamIfAny(sessionId: string) {
  const partial = extractPartialStreamContent(
    useSession.getState().active?.messages ?? [],
    sessionId,
  );
  if (!partial.text && !partial.thinking && partial.blocks.length === 0) {
    return;
  }
  try {
    await api.saveCancelledMessage(
      sessionId,
      partial.text,
      partial.thinking,
      partial.blocks.length > 0 ? partial.blocks : null,
    );
  } catch (saveErr) {
    console.warn("Failed to save partial message", saveErr);
  }
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
      cancelStreamFlushRaf(sessionId);
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

  /** Cancel in-flight generation and wait for the invoke to settle. */
  const abortInFlightGeneration = async (sessionId: string) => {
    const busy = !!get().busyBySession[sessionId];
    const hasFlight = generationFlights.has(sessionId);
    if (!busy && !hasFlight) return;

    if (busy && !cancellingSessions.has(sessionId)) {
      cancellingSessions.add(sessionId);
      setSessionBusy(sessionId, false);
      freezeStreamingUi(sessionId);
      void api.cancelGeneration(sessionId).catch((e) => {
        console.warn("[atelier] cancel_generation failed", e);
      });
    }
    bumpGenerationEpoch(sessionId);
    await waitForGenerationIdle(sessionId);
  };

  /** Message ids after `idx` that exist in the DB (skip optimistic tmp rows). */
  const persistedIdsAfter = (messages: MessageAbs[], idx: number) =>
    messages
      .slice(idx + 1)
      .filter((msg) => !msg.id.startsWith("tmp-"))
      .map((msg) => msg.id);

  return ({
  sessions: [],
  activeId: null,
  active: null,
  busy: false,
  busyBySession: {},
  composer: {
    prompt: "",
    mentions: [],
    attachments: [],
    pendingAttachments: [],
    aspectRatio: "auto",
    imageSize: "auto",
    thinkingEnabled: false,
    thinkingEffort: "",
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
        mentions: draft?.mentions ?? [],
        attachments: draft?.attachments ?? [],
        pendingAttachments: [],
        aspectRatio: draft?.aspectRatio ?? get().composer.aspectRatio,
        imageSize: draft?.imageSize ?? get().composer.imageSize,
        chatMode: composerModeFromAgentType(data.session.agent_type),
      },
    });
    void useRoleState.getState().loadLatest(id);
    useReader.getState().bindSession(id);
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
  setMentions: (paths) => set({ composer: { ...get().composer, mentions: paths } }),
  setAspectRatio: (s) => set({ composer: { ...get().composer, aspectRatio: s } }),
  setImageSize: (s) => set({ composer: { ...get().composer, imageSize: s } }),
  setThinkingEnabled: (on) =>
    set({ composer: { ...get().composer, thinkingEnabled: on } }),
  setThinkingEffort: (effort) =>
    set({ composer: { ...get().composer, thinkingEffort: effort } }),

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

  setAgentChain: async (chain) => {
    const id = get().activeId;
    if (!id) return;
    const cleaned = chain
      .map((e): ChainEntry => {
        if (typeof e === "string") return e.trim();
        const at = e.agent_type.trim();
        const ov = e.overrides;
        const hasOv =
          !!ov &&
          (ov.system_prompt !== undefined ||
            ov.model !== undefined ||
            ov.tools !== undefined);
        return hasOv ? { agent_type: at, overrides: ov } : at;
      })
      .filter((e) => (typeof e === "string" ? e.length > 0 : e.agent_type.length > 0));
    const active = get().active;
    // Sessions in a project share a single, project-scoped agent flow: editing
    // the flow on any conversation persists to the project so all of its
    // conversations (and any new ones) stay in sync. Plain chats keep a
    // per-session chain.
    const projectId = active?.session.id === id ? active.session.project_id : null;
    if (active && active.session.id === id) {
      set({
        active: {
          ...active,
          session: { ...active.session, agent_chain: cleaned.length ? cleaned : null },
        },
      });
    }
    try {
      if (projectId) {
        await api.setProjectAgentChain(projectId, cleaned);
      } else {
        await api.setSessionAgentChain(id, cleaned);
      }
    } catch (e) {
      console.warn(e);
      await get().reloadActiveSession();
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
        mentions: [],
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
    const a = get().active;
    if (!a) return;
    const sid = a.session.id;
    const idx = a.messages.findIndex((x) => x.id === messageId);
    if (idx < 0) return;
    const target = a.messages[idx];

    await abortInFlightGeneration(sid);

    const toDelete = [messageId];
    // Same branch cut as resend: removing a user turn drops everything after it.
    if (target.role === "user") {
      toDelete.push(...persistedIdsAfter(a.messages, idx));
    }

    for (const id of toDelete) {
      try {
        await api.deleteMessage(id);
      } catch (e) {
        console.warn(e);
      }
    }
    await reloadActiveSessionIfViewing(sid);
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
    if (generationFlights.has(sid)) {
      bumpGenerationEpoch(sid);
      await waitForGenerationIdle(sid);
    }

    const toDelete = persistedIdsAfter(a.messages, idx);

    for (const id of toDelete) {
      try {
        await api.deleteMessage(id);
      } catch (e) {
        console.warn(e);
      }
    }

    bumpGenerationEpoch(sid);
    await reloadActiveSessionIfViewing(sid);
    const epoch = getGenerationEpoch(sid);

    const c = get().composer;
    const canReason = activeModelCapabilities().includes("reasoning");
    const thinkingEnabled = canReason ? c.thinkingEnabled : null;
    const thinkingEffort =
      canReason && c.thinkingEnabled && c.thinkingEffort.trim()
        ? c.thinkingEffort.trim()
        : null;
    setSessionBusy(sid, true);
    ensureGenerationStreamListener();
    const run = (async () => {
      try {
        await api.regenerateImage(
          {
            session_id: sid,
            user_message_id: messageId,
            aspect_ratio: c.aspectRatio,
            image_size: c.imageSize,
            thinking_enabled: thinkingEnabled,
            thinking_effort: thinkingEffort,
          },
          a.session,
        );
        if (epoch !== getGenerationEpoch(sid)) return;
        await reloadActiveSessionIfViewing(sid);
        await get().refreshList();
      } catch (e: unknown) {
        if (epoch !== getGenerationEpoch(sid)) return;
        if (isGenerationCancelled(e)) {
          await persistPartialStreamIfAny(sid);
          await reloadActiveSessionIfViewing(sid);
          await get().refreshList();
          return;
        }
        console.error(e);
        await persistPartialStreamIfAny(sid);
        await reloadActiveSessionIfViewing(sid);
        await get().refreshList();
      } finally {
        cancellingSessions.delete(sid);
        if (epoch === getGenerationEpoch(sid)) {
          setSessionBusy(sid, false);
        }
      }
    })();
    await trackGenerationFlight(sid, run);
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
    await waitForGenerationIdle(sid);
    const epoch = getGenerationEpoch(sid);

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
    const canReason = activeModelCapabilities().includes("reasoning");
    const thinkingEnabled = canReason ? c.thinkingEnabled : null;
    const thinkingEffort =
      canReason && c.thinkingEnabled && c.thinkingEffort.trim()
        ? c.thinkingEffort.trim()
        : null;

    updateActiveSession(sid, (active) => ({
      ...active,
      messages: [...active.messages, optimisticUser],
    }));
    set({
      composer: { ...get().composer, prompt: "", mentions: [], attachments: [], pendingAttachments: [] },
    });
    // Clear the saved draft once the message is sent.
    composerDrafts.delete(sid);
    setSessionBusy(sid, true);
    ensureGenerationStreamListener();

    const run = (async () => {
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
            thinking_enabled: thinkingEnabled,
            thinking_effort: thinkingEffort,
          },
          sessionForLog,
        );
        if (epoch !== getGenerationEpoch(sid)) return;
        await reloadActiveSessionIfViewing(sid);
        await get().refreshList();
      } catch (e: unknown) {
        if (epoch !== getGenerationEpoch(sid)) return;
        if (isGenerationCancelled(e)) {
          await persistPartialStreamIfAny(sid);
          try {
            await reloadActiveSessionIfViewing(sid);
            await get().refreshList();
          } catch (reloadError) {
            console.warn(reloadError);
          }
          return;
        }
        console.error(e);
        await persistPartialStreamIfAny(sid);
        await reloadActiveSessionIfViewing(sid);
        await get().refreshList();
      } finally {
        cancellingSessions.delete(sid);
        if (epoch === getGenerationEpoch(sid)) {
          setSessionBusy(sid, false);
        }
      }
    })();
    await trackGenerationFlight(sid, run);
  },

  interrupt: () => {
    const sid = get().activeId;
    if (!sid || !get().busyBySession[sid]) {
      return;
    }
    if (cancellingSessions.has(sid)) {
      return;
    }
    cancellingSessions.add(sid);
    // Stop accepting stream deltas and release the send button immediately.
    setSessionBusy(sid, false);
    freezeStreamingUi(sid);
    void api.cancelGeneration(sid).catch((e) => {
      console.warn("[atelier] cancel_generation failed", e);
    });
  },
  });
});

// ─── Per-session composer drafts ─────────────────────────────────────────────
// Saves each session's input state so switching sessions preserves whatever
// the user had typed or attached.

interface ComposerDraft {
  prompt: string;
  mentions: string[];
  attachments: AttachmentDraft[];
  aspectRatio: string;
  imageSize: string;
}

const composerDrafts = new Map<string, ComposerDraft>();

function saveComposerDraft(sessionId: string, composer: ComposerState) {
  composerDrafts.set(sessionId, {
    prompt: composer.prompt,
    mentions: composer.mentions,
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

/** Sessions the user interrupted; ignore late stream events until the invoke returns. */
const cancellingSessions = new Set<string>();

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

// ─── Streaming tool-call input ────────────────────────────────────────────────
// Tools whose input arguments we render live as they stream in. Other tools are
// only materialised by the terminal `gen://tool` event (unchanged behaviour).
const STREAMING_INPUT_TOOLS = new Set(["CreateDoc", "Edit"]);

/** Accumulated raw `arguments` JSON string per streaming tool call. */
const toolCallArgBuffers = new Map<string, string>();

function toolCallArgKey(sessionId: string, id: string): string {
  return `${sessionId}:${id}`;
}

function escapeRegExp(s: string): string {
  return s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

/**
 * Extract a string-valued field from a possibly-incomplete JSON object string.
 * Returns the (partial) decoded value when the field is present, even if the
 * closing quote hasn't streamed in yet; `undefined` if the key isn't found.
 */
function extractJsonStringField(raw: string, key: string): string | undefined {
  const opener = new RegExp(`"${escapeRegExp(key)}"\\s*:\\s*"`);
  const m = opener.exec(raw);
  if (!m) return undefined;
  let i = m.index + m[0].length;
  let out = "";
  while (i < raw.length) {
    const ch = raw[i];
    if (ch === "\\") {
      const next = raw[i + 1];
      if (next === undefined) break; // dangling escape at buffer end
      switch (next) {
        case "n":
          out += "\n";
          break;
        case "t":
          out += "\t";
          break;
        case "r":
          out += "\r";
          break;
        case "b":
          out += "\b";
          break;
        case "f":
          out += "\f";
          break;
        case "/":
          out += "/";
          break;
        case '"':
          out += '"';
          break;
        case "\\":
          out += "\\";
          break;
        case "u": {
          const hex = raw.slice(i + 2, i + 6);
          if (hex.length < 4) return out; // incomplete \uXXXX at buffer end
          const code = Number.parseInt(hex, 16);
          if (!Number.isNaN(code)) out += String.fromCharCode(code);
          i += 6;
          continue;
        }
        default:
          out += next;
          break;
      }
      i += 2;
      continue;
    }
    if (ch === '"') return out; // closing quote -> complete value
    out += ch;
    i += 1;
  }
  return out; // buffer ended before closing quote -> partial value
}

/** Build a partial tool input object from the buffered arguments string. */
function buildStreamingToolInput(
  tool: string,
  raw: string,
): Record<string, unknown> {
  if (tool === "CreateDoc") {
    return {
      title: extractJsonStringField(raw, "title"),
      doc_type: extractJsonStringField(raw, "doc_type"),
      content: extractJsonStringField(raw, "content"),
    };
  }
  if (tool === "Edit") {
    return {
      path: extractJsonStringField(raw, "path"),
      original_content: extractJsonStringField(raw, "original_content"),
      modified_content: extractJsonStringField(raw, "modified_content"),
    };
  }
  try {
    return JSON.parse(raw) as Record<string, unknown>;
  } catch {
    return {};
  }
}

/**
 * Apply a live tool-call argument fragment to the block list. Creates a pending
 * `tool_use` block on first sight (keyed by id) and refreshes its partial input
 * on every subsequent fragment. Only document tools are rendered live; others
 * are ignored here and surface via the terminal `gen://tool` event.
 */
function applyStreamingToolCallDelta(
  blocks: AssistantBlock[],
  sessionId: string,
  delta: { id: string; name: string; arguments: string },
) {
  const { id, name, arguments: fragment } = delta;
  if (!id || !name || !STREAMING_INPUT_TOOLS.has(name)) return;

  const key = toolCallArgKey(sessionId, id);
  const raw = (toolCallArgBuffers.get(key) ?? "") + (fragment ?? "");
  toolCallArgBuffers.set(key, raw);

  const input = buildStreamingToolInput(name, raw);

  for (let i = blocks.length - 1; i >= 0; i--) {
    const b = blocks[i];
    if (b.type === "tool_use" && b.id === id) {
      blocks[i] = { ...b, input, streaming: true };
      return;
    }
  }
  blocks.push({
    type: "tool_use",
    id,
    tool: name,
    input,
    status: "pending",
    streaming: true,
  });
}

/**
 * Record a structured tool event into the block list. `tool_use` always
 * pushes a new pending block; `tool_result` mutates the matching `tool_use`
 * block in place so the on-screen card transitions from pending → done
 * without changing the surrounding order.
 */
/**
 * Sync reader panel state when an agent file tool completes.
 */
async function handleReaderToolComplete(
  tool: string,
  input: unknown,
  output: unknown,
  isError: boolean | undefined,
) {
  if (isError) return;
  const o = (output && typeof output === "object" ? output : {}) as Record<string, unknown>;
  const path = resolveToolFilePath(input, output);

  if (tool === "CreateDoc") {
    const doc = readerDocFromToolOutput(output);
    if (doc) useReader.getState().openDoc(doc);
    return;
  }

  if (!path) return;
  const reader = useReader.getState();
  let existing = reader.getTabByPath(path);

  if (tool === "Edit") {
    const inp = (input && typeof input === "object" ? input : {}) as Record<string, unknown>;
    const original =
      typeof inp.original_content === "string" ? inp.original_content : "";
    const modified =
      typeof inp.modified_content === "string" ? inp.modified_content : "";
    const paragraphNumber = parseEditParagraphNumber(input);
    if (paragraphNumber == null) return;

    let textBefore = existing?.text;
    if (textBefore == null) {
      const sessionId = useSession.getState().activeId;
      if (!sessionId) return;
      try {
        const disk = await api.readProjectFile(sessionId, path);
        textBefore =
          revertParagraphEdit(disk.text, paragraphNumber, original, modified) ??
          disk.text;
        reader.openDoc(
          {
            path,
            text: disk.text,
            fileType: inferFileType(path),
            encoding: disk.encoding,
            hadBom: disk.hadBom,
          },
          { activate: false },
        );
        existing = reader.getTabByPath(path);
      } catch (e) {
        console.warn("Edit: failed to load file for reader diff", e);
        return;
      }
    }

    const textAfter = applyParagraphEdit(textBefore, paragraphNumber, original, modified);
    if (textAfter == null) return;
    if (isDiffTextEqual(original, modified)) return;
    reader.appendPendingDiff(path, {
      before: normalizeDiffText(original),
      after: normalizeDiffText(modified),
      paragraphNumber,
      textBefore,
      textAfter,
    });
    return;
  }

  if (tool === "Write") {
    if (!existing) return;
    const text =
      typeof o.text === "string"
        ? stripParagraphLabels(o.text)
        : typeof (input as Record<string, unknown>)?.content === "string"
          ? stripParagraphLabels((input as Record<string, unknown>).content as string)
          : null;
    if (text != null) reader.updateTabText(path, text, { dirty: false });
  }
}

function applyToolEvent(
  blocks: AssistantBlock[],
  event: ToolEventPayload,
  sessionId?: string,
) {
  if (event.type === "tool_use") {
    // Reconcile with a block that was pre-created while its input streamed in:
    // replace the partial input with the authoritative one and stop streaming.
    if (sessionId) toolCallArgBuffers.delete(toolCallArgKey(sessionId, event.id));
    for (let i = blocks.length - 1; i >= 0; i--) {
      const b = blocks[i];
      if (b.type === "tool_use" && b.id === event.id) {
        blocks[i] = {
          ...b,
          tool: event.tool,
          input: event.input,
          status: "pending",
          streaming: false,
        };
        return;
      }
    }
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
      if (!event.is_error) {
        void handleReaderToolComplete(b.tool, b.input, event.output, event.is_error);
      }
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
function syncStreamingMessage(sessionId: string) {
  // Low-priority update so clicks, drags, and session switches stay responsive.
  startTransition(() => {
    const state = useSession.getState();
    if (state.activeId !== sessionId || !state.active) return;
    const buf = streamingBuffers.get(sessionId);
    if (!buf) return;
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
  });
}

/** Coalesce high-frequency stream deltas to one React commit per animation frame. */
const streamFlushRafBySession = new Map<string, number>();

function cancelStreamFlushRaf(sessionId: string) {
  const rafId = streamFlushRafBySession.get(sessionId);
  if (rafId != null) {
    cancelAnimationFrame(rafId);
    streamFlushRafBySession.delete(sessionId);
  }
}

function scheduleStreamingMessageSync(sessionId: string) {
  if (streamFlushRafBySession.has(sessionId)) return;
  const rafId = requestAnimationFrame(() => {
    streamFlushRafBySession.delete(sessionId);
    if (streamingBuffers.has(sessionId)) syncStreamingMessage(sessionId);
  });
  streamFlushRafBySession.set(sessionId, rafId);
}

/** Flush any pending frame batch immediately (stop, cancel, session restore). */
function flushStreamingMessageSync(sessionId: string) {
  cancelStreamFlushRaf(sessionId);
  if (streamingBuffers.has(sessionId)) syncStreamingMessage(sessionId);
}

/** Stop streaming indicators and freeze in-flight tool cards the moment the user hits Stop. */
function freezeStreamingUi(sessionId: string) {
  const buf = streamingBuffers.get(sessionId);
  if (!buf) return;
  const nextBlocks = buf.blocks.map((b) => {
    if (b.type === "tool_use" && b.status === "pending") {
      return {
        ...b,
        status: "error" as const,
        is_error: true,
        output: "Cancelled",
      };
    }
    return { ...b };
  });
  const next: StreamBuffer = { ...buf, blocks: nextBlocks };
  streamingBuffers.set(sessionId, next);
  flushStreamingMessageSync(sessionId);
}

// ─── Stream listeners ─────────────────────────────────────────────────────────

let generationStreamListenerStarted = false;
let toolEventListenerStarted = false;
let roleStateResetListenerStarted = false;
let sessionTitleListenerStarted = false;

function ensureGenerationStreamListener() {
  if (!generationStreamListenerStarted) {
    generationStreamListenerStarted = true;
    listen<GenerationStreamPayload>("gen://stream", (event) => {
      const payload = event.payload;
      const sessionId = payload.session_id;
      if (sessionId && cancellingSessions.has(sessionId)) return;
      const textDelta = payload.text_delta ?? payload.delta ?? "";
      const thinkingDelta = payload.thinking_delta ?? "";
      const stage = payload.stage;
      const toolCallDelta = payload.tool_call_delta ?? null;
      if (
        !sessionId ||
        (!textDelta && !thinkingDelta && !stage && !toolCallDelta)
      )
        return;

      const requestId = payload.request_message_id || sessionId;
      const prev = streamingBuffers.get(sessionId) ?? {
        blocks: [],
        requestId,
      };
      // Always work on a fresh array so React sees a new reference.
      const nextBlocks = prev.blocks.map((b) => ({ ...b }));
      if (stage) {
        nextBlocks.push({
          type: "agent_stage",
          agent_type: stage.agent_type,
          name: stage.name,
          index: stage.index,
        });
      }
      if (thinkingDelta) appendDelta(nextBlocks, "thinking", thinkingDelta);
      if (textDelta) appendDelta(nextBlocks, "text", textDelta);
      if (toolCallDelta)
        applyStreamingToolCallDelta(nextBlocks, sessionId, toolCallDelta);
      const next: StreamBuffer = { blocks: nextBlocks, requestId };
      streamingBuffers.set(sessionId, next);
      scheduleStreamingMessageSync(sessionId);
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
      if (cancellingSessions.has(sessionId)) return;
      const requestId = payload.request_message_id || sessionId;
      const prev = streamingBuffers.get(sessionId) ?? {
        blocks: [],
        requestId,
      };
      const nextBlocks = prev.blocks.map((b) => ({ ...b }));
      applyToolEvent(nextBlocks, payload, sessionId);
      // Incrementally drive the character state board off RoleState results.
      if (
        payload.type === "tool_result" &&
        payload.tool === "RoleState" &&
        !payload.is_error &&
        payload.output &&
        typeof payload.output === "object"
      ) {
        useRoleState
          .getState()
          .applyOp(sessionId, payload.output as RoleStateOp);
      }
      const next: StreamBuffer = { blocks: nextBlocks, requestId };
      streamingBuffers.set(sessionId, next);
      scheduleStreamingMessageSync(sessionId);
    }).catch((e) => {
      toolEventListenerStarted = false;
      console.warn(e);
    });
  }

  if (!roleStateResetListenerStarted) {
    roleStateResetListenerStarted = true;
    listen<{ session_id: string }>("role-state://reset", (event) => {
      const sessionId = event.payload?.session_id;
      if (!sessionId) return;
      void useRoleState.getState().loadLatest(sessionId);
    }).catch((e) => {
      roleStateResetListenerStarted = false;
      console.warn(e);
    });
  }

  if (!sessionTitleListenerStarted) {
    sessionTitleListenerStarted = true;
    listen<{ session_id: string; title: string }>("session://title", (event) => {
      const sessionId = event.payload?.session_id;
      const title = event.payload?.title;
      if (!sessionId || !title) return;
      const state = useSession.getState();
      useSession.setState({
        sessions: state.sessions.map((s) =>
          s.id === sessionId ? { ...s, title } : s,
        ),
      });
      if (state.activeId === sessionId && state.active) {
        const a = state.active;
        useSession.setState({
          active: { ...a, session: { ...a.session, title } },
        });
      }
    }).catch((e) => {
      sessionTitleListenerStarted = false;
      console.warn(e);
    });
  }
}

ensureGenerationStreamListener();
