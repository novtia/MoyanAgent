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
import type { VideoGenerationMode } from "../config/videoGeneration";
import { useRoleState, resolveRoleStateScope, type RoleStateOp } from "./roleState";
import {
  useReader,
  readerDocFromToolOutput,
  stripParagraphLabels,
  resolveToolFilePath,
  revertStringEdit,
  inferFileType,
} from "./reader";
import { useSettings } from "./settings";
import { useProject } from "./project";
import { normalizeDiffText } from "../utils/inlineDiff";
import { normalizeToolContent } from "../utils/normalizeToolContent";
import {
  type PendingAskUser,
  askUserAnswerText,
  askUserCustomText,
  firstUnansweredAskUserIndex,
  flushAskUserPrompt,
  formatAskUserItems,
  formatAskUserReply,
  parseAskUserInput,
  questionKey,
} from "../components/chat/askUser";

interface ComposerState {
  prompt: string;
  /** Absolute file paths referenced as `@` mention chips in the prompt. */
  mentions: string[];
  attachments: AttachmentDraft[];
  pendingAttachments: PendingAttachmentDraft[];
  aspectRatio: string;
  imageSize: string;
  videoMode: VideoGenerationMode;
  videoDuration: number;
  videoResolution: string;
  generateAudio: boolean;
  watermark: boolean;
  /** Composer thinking toggle (only meaningful for reasoning-capable models). */
  thinkingEnabled: boolean;
  /** Reasoning effort; empty string means provider default (high). */
  thinkingEffort: string;
  chatMode: ComposerChatMode;
}

/**
 * Look up a model's capabilities by provider + model id, falling back to a
 * search across all providers when the provider id is unknown (e.g. a session
 * whose provider hasn't been backfilled yet).
 */
export function modelCapabilities(
  providerId: string | null | undefined,
  modelId: string | null | undefined,
): string[] {
  const settings = useSettings.getState().settings;
  if (!settings || !modelId) return [];
  const byProvider = settings.model_services
    ?.find((p) => p.id === providerId)
    ?.models?.find((m) => m.id === modelId);
  if (byProvider) return byProvider.capabilities ?? [];
  for (const p of settings.model_services ?? []) {
    const m = p.models?.find((mm) => mm.id === modelId);
    if (m) return m.capabilities ?? [];
  }
  return [];
}

/**
 * Capabilities of the currently active session's model (its own provider +
 * model), falling back to the global default when no session is active.
 */
function activeModelCapabilities(): string[] {
  const settings = useSettings.getState().settings;
  if (!settings) return [];
  const active = useSession.getState().active;
  const providerId = active?.session.provider_id ?? settings.active_provider_id;
  const modelId = active?.session.model ?? settings.model;
  return modelCapabilities(providerId, modelId);
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

interface GenerationStatusPayload {
  session_id: string;
  phase: "request" | "polling" | "response" | string;
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
  /**
   * Sessions whose generation finished in the background (while the user was
   * viewing another session). Kept as a "task complete" reminder in the
   * sidebar until the user opens the session or dismisses the card.
   */
  finishedBySession: Record<string, boolean>;
  generationPhaseBySession: Record<string, string>;
  composer: ComposerState;
  /** Active session's pending AskUser questionnaire (null if none / other session). */
  pendingAskUser: PendingAskUser | null;

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
  /** Clear a session's background "task complete" reminder. */
  dismissFinished: (id: string) => void;

  setPrompt: (s: string) => void;
  setMentions: (paths: string[]) => void;
  setAspectRatio: (s: string) => void;
  setImageSize: (s: string) => void;
  setVideoMode: (mode: VideoGenerationMode) => void;
  setVideoDuration: (duration: number) => void;
  setVideoResolution: (resolution: string) => void;
  setGenerateAudio: (enabled: boolean) => void;
  setWatermark: (enabled: boolean) => void;
  setThinkingEnabled: (on: boolean) => void;
  setThinkingEffort: (effort: string) => void;
  /**
   * Persist the composer's current thinking toggle into the given session's
   * own `llm_params` so thinking is self-owned per session. No-op when the
   * session is not the active one or the value is unchanged.
   */
  persistComposerThinking: (sessionId: string) => Promise<void>;
  setChatMode: (mode: ComposerChatMode) => Promise<void>;
  setAgentChain: (chain: ChainEntry[]) => Promise<void>;
  addAttachments: (files: File[]) => Promise<void>;
  addAttachmentsFromPaths: (paths: string[]) => Promise<void>;
  addAttachmentFromPath: (path: string) => Promise<void>;
  addReferenceVideoUrl: (url: string) => Promise<void>;
  removeAttachment: (imageId: string) => Promise<void>;
  replaceAttachment: (oldId: string, draft: AttachmentDraft) => void;
  clearComposer: () => void;

  setAskUserIndex: (index: number) => void;
  /** Select an option for the active question (does not fill the composer). */
  setAskUserAnswer: (optionKey: string, optionText: string) => void;
  /** Clear the selected option for the active question (custom text kept). */
  clearAskUserAnswer: () => void;
  clearPendingAskUser: () => void;
  /** Submit AskUser answers and resume the blocked agent loop (does not start a new send). */
  answerPendingAskUser: () => Promise<void>;

  send: () => Promise<void>;
  interrupt: () => void;
  resendMessage: (messageId: string) => Promise<void>;
  deleteMessage: (messageId: string) => Promise<void>;
  editMessage: (messageId: string, text: string, imageIds?: string[]) => Promise<void>;
  appendMessages: (msgs: MessageAbs[]) => void;

  quoteMessage: (m: MessageAbs) => Promise<void>;
}

const ACCEPT_TYPES = [
  "image/png",
  "image/jpeg",
  "image/webp",
  "image/bmp",
  "image/gif",
  "image/tiff",
  "audio/wav",
  "audio/mpeg",
];
const MAX_FILES = 15;
const MAX_IMAGE_BYTES = 30 * 1024 * 1024;
const MAX_AUDIO_BYTES = 15 * 1024 * 1024;

/** Per-session AskUser questionnaires (survives session switches). */
const askUserPendingBySession = new Map<string, PendingAskUser>();

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

function stripMediaMentionTokens(text: string) {
  return text
    .replace(/@(?:"(?:image|音频|视频)\d+"|(?:image|音频|视频)\d+)/g, "")
    .replace(/[ \t]+\n/g, "\n")
    .replace(/[ \t]{2,}/g, " ")
    .trim();
}

function matchesImageRole(role: string) {
  return role === "input" || role === "output" || role === "edited";
}

function maxBytesForMime(mime: string) {
  return mime.startsWith("audio/") ? MAX_AUDIO_BYTES : MAX_IMAGE_BYTES;
}

function uploadMime(file: File) {
  const declared = file.type.toLowerCase();
  if (ACCEPT_TYPES.includes(declared)) return declared;
  const extension = file.name.split(".").pop()?.toLowerCase() ?? "";
  if (extension === "jpg" || extension === "jpeg") return "image/jpeg";
  if (extension === "png") return "image/png";
  if (extension === "webp") return "image/webp";
  if (extension === "bmp") return "image/bmp";
  if (extension === "gif") return "image/gif";
  if (extension === "tif" || extension === "tiff") return "image/tiff";
  if (extension === "wav") return "audio/wav";
  if (extension === "mp3") return "audio/mpeg";
  return declared;
}

function isVideoModel() {
  return activeModelCapabilities().includes("video");
}

/**
 * The parameters actually applied to a session's generation. For project
 * sessions the project shares its sampling params / prompt / history with all
 * its sessions (so the session's own values are overridden), while `thinking`
 * stays session-owned. Mirrors the backend `effective_session_params` so the
 * request log reflects what is really sent.
 */
function effectiveSessionForLog(session: SessionWithMessagesAbs["session"]) {
  if (!session.project_id) return session;
  const proj = useProject
    .getState()
    .projects.find((p) => p.id === session.project_id);
  if (!proj) return session;
  return {
    ...session,
    system_prompt: proj.system_prompt,
    history_turns: proj.history_turns,
    context_window: proj.context_window ?? session.context_window,
    llm_params: {
      ...proj.llm_params,
      thinking_enabled: session.llm_params?.thinking_enabled ?? null,
      thinking_effort: session.llm_params?.thinking_effort ?? null,
    },
  };
}

function resolvedMediaRole(
  mode: VideoGenerationMode,
  mime: string,
  index: number,
): string | null {
  if (mime.startsWith("audio/")) return "reference_audio";
  if (mime.startsWith("video/")) return "reference_video";
  if (!mime.startsWith("image/")) return null;
  if (mode === "first_frame") return "first_frame";
  if (mode === "first_last") return index === 0 ? "first_frame" : "last_frame";
  if (mode === "reference") return "reference_image";
  return null;
}

function validateVideoModeAttachments(
  mode: VideoGenerationMode,
  media: Array<{ mime: string }>,
  prompt: string,
): boolean {
  const images = media.filter((a) => a.mime.startsWith("image/")).length;
  const audio = media.filter((a) => a.mime.startsWith("audio/")).length;
  const videos = media.filter((a) => a.mime.startsWith("video/")).length;
  switch (mode) {
    case "text":
      return !!prompt && images + audio + videos === 0;
    case "first_frame":
      return images === 1 && audio + videos === 0;
    case "first_last":
      return images === 2 && audio + videos === 0;
    case "reference":
      return images <= 9 && audio <= 3 && videos <= 3 && images + videos >= 1;
  }
}

/**
 * If attachments no longer match the recorded mode, pick a compatible
 * mode when the mapping is unambiguous (e.g. all images removed → text).
 */
function coerceVideoModeForMedia(
  mode: VideoGenerationMode | string | undefined,
  media: Array<{ mime: string }>,
  prompt: string,
): VideoGenerationMode | null {
  const resolved: VideoGenerationMode =
    mode === "first_frame" ||
    mode === "first_last" ||
    mode === "reference" ||
    mode === "text"
      ? mode
      : "text";
  if (validateVideoModeAttachments(resolved, media, prompt)) return resolved;

  const images = media.filter((a) => a.mime.startsWith("image/")).length;
  const audio = media.filter((a) => a.mime.startsWith("audio/")).length;
  const videos = media.filter((a) => a.mime.startsWith("video/")).length;
  const total = images + audio + videos;

  if (total === 0 && prompt) return "text";
  if (images === 1 && audio + videos === 0) return "first_frame";
  if (images === 2 && audio + videos === 0) return "first_last";
  if (
    images <= 9 &&
    audio <= 3 &&
    videos <= 3 &&
    images + videos >= 1
  ) {
    return "reference";
  }
  return null;
}

function validateVideoComposer(composer: ComposerState, prompt: string): boolean {
  return validateVideoModeAttachments(
    composer.videoMode,
    composer.attachments,
    prompt,
  );
}

/** True when message media matches its recorded video_mode (or non-video messages). */
export function messageMatchesVideoMode(message: {
  text?: string | null;
  params?: { video_mode?: string } | null;
  images: Array<{ role: string; mime: string }>;
}): boolean {
  const mode = message.params?.video_mode;
  if (
    mode !== "text" &&
    mode !== "first_frame" &&
    mode !== "first_last" &&
    mode !== "reference"
  ) {
    return true;
  }
  const inputs = message.images.filter((i) => i.role === "input");
  return (
    coerceVideoModeForMedia(mode, inputs, (message.text || "").trim()) != null
  );
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
      const finishedBySession = { ...state.finishedBySession };
      if (busy) {
        busyBySession[sessionId] = true;
        // A new run supersedes any lingering "task complete" reminder.
        delete finishedBySession[sessionId];
      } else {
        delete busyBySession[sessionId];
      }
      const generationPhaseBySession = {
        ...state.generationPhaseBySession,
      };
      if (!busy) delete generationPhaseBySession[sessionId];
      return {
        busyBySession,
        finishedBySession,
        generationPhaseBySession,
        busy: state.activeId ? !!busyBySession[state.activeId] : false,
      };
    });
  };

  /** Flag a session's generation as finished in the background (reminder). */
  const markSessionFinished = (sessionId: string) => {
    set((state) => ({
      finishedBySession: { ...state.finishedBySession, [sessionId]: true },
    }));
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
    // Generation is complete — always discard the streaming buffer, even when
    // the user is viewing another session. A leftover buffer would otherwise
    // be picked up by the next generation in this session and render the
    // previous assistant reply as part of the new streaming message.
    cancelStreamFlushRaf(sessionId);
    streamingBuffers.delete(sessionId);
    if (get().activeId !== sessionId) return;
    try {
      const data = await api.loadSession(sessionId);
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
  finishedBySession: {},
  generationPhaseBySession: {},
  composer: {
    prompt: "",
    mentions: [],
    attachments: [],
    pendingAttachments: [],
    aspectRatio: "auto",
    imageSize: "auto",
    videoMode: "text",
    videoDuration: 5,
    videoResolution: "720p",
    generateAudio: true,
    watermark: false,
    thinkingEnabled: false,
    thinkingEffort: "",
    chatMode: "agent",
  },
  pendingAskUser: null,

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
      const pending = get().pendingAskUser;
      if (pending && pending.sessionId === currentId) {
        const flushed = flushAskUserPrompt(pending, get().composer.prompt);
        askUserPendingBySession.set(currentId, flushed);
      }
    }

    const data = await api.loadSession(id);
    const isBusy = !!get().busyBySession[id];

    // Opening a session acknowledges any background "task complete" reminder.
    if (get().finishedBySession[id]) {
      set((state) => {
        const finishedBySession = { ...state.finishedBySession };
        delete finishedBySession[id];
        return { finishedBySession };
      });
    }

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
    const pendingAsk = askUserPendingBySession.get(id) ?? null;
    const askCustom =
      pendingAsk != null
        ? askUserCustomText(pendingAsk, pendingAsk.activeIndex)
        : null;
    set({
      activeId: id,
      active: { ...data, messages: messagesWithBuffer },
      busy: isBusy,
      pendingAskUser: pendingAsk,
      composer: {
        ...get().composer,
        prompt: askCustom ?? draft?.prompt ?? "",
        mentions: draft?.mentions ?? [],
        attachments: draft?.attachments ?? [],
        pendingAttachments: [],
        aspectRatio: draft?.aspectRatio ?? get().composer.aspectRatio,
        imageSize: draft?.imageSize ?? get().composer.imageSize,
        videoMode: draft?.videoMode ?? get().composer.videoMode,
        videoDuration: draft?.videoDuration ?? get().composer.videoDuration,
        videoResolution: draft?.videoResolution ?? get().composer.videoResolution,
        generateAudio: draft?.generateAudio ?? get().composer.generateAudio,
        watermark: draft?.watermark ?? get().composer.watermark,
        thinkingEnabled: data.session.llm_params?.thinking_enabled ?? false,
        thinkingEffort: data.session.llm_params?.thinking_effort ?? "",
        chatMode: composerModeFromAgentType(data.session.agent_type),
      },
    });
    void useRoleState.getState().loadLatest(id, roleStateScopeForSession(id));
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
    askUserPendingBySession.delete(id);
    composerDrafts.delete(id);
    if (get().activeId === id) {
      set({ activeId: null, active: null, busy: false, pendingAskUser: null });
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
      const state = get();
      // A settings-only reload (for example, after switching models) can race
      // with an in-flight stream. Keep the optimistic/streaming rows until the
      // generation completion path replaces them with the persisted messages.
      // Also ignore a stale response if the user switched sessions meanwhile.
      if (state.activeId !== id) return;
      const preserveLiveMessages =
        !!state.busyBySession[id] && state.active?.session.id === id;
      set({
        active: preserveLiveMessages
          ? { ...data, messages: state.active!.messages }
          : data,
        composer: {
          ...get().composer,
          chatMode: composerModeFromAgentType(data.session.agent_type),
        },
      });
    } catch (e) {
      console.warn(e);
    }
  },

  dismissFinished: (id) => {
    set((state) => {
      if (!state.finishedBySession[id]) return state;
      const finishedBySession = { ...state.finishedBySession };
      delete finishedBySession[id];
      return { finishedBySession };
    });
  },

  setPrompt: (s) => set({ composer: { ...get().composer, prompt: s } }),
  setMentions: (paths) => set({ composer: { ...get().composer, mentions: paths } }),
  setAspectRatio: (s) => set({ composer: { ...get().composer, aspectRatio: s } }),
  setImageSize: (s) => set({ composer: { ...get().composer, imageSize: s } }),
  setVideoMode: (mode) => set({ composer: { ...get().composer, videoMode: mode } }),
  setVideoDuration: (duration) =>
    set({ composer: { ...get().composer, videoDuration: duration } }),
  setVideoResolution: (resolution) =>
    set({ composer: { ...get().composer, videoResolution: resolution } }),
  setGenerateAudio: (enabled) =>
    set({ composer: { ...get().composer, generateAudio: enabled } }),
  setWatermark: (enabled) =>
    set({ composer: { ...get().composer, watermark: enabled } }),
  setThinkingEnabled: (on) =>
    set({ composer: { ...get().composer, thinkingEnabled: on } }),
  setThinkingEffort: (effort) =>
    set({ composer: { ...get().composer, thinkingEffort: effort } }),

  persistComposerThinking: async (sessionId) => {
    const a = get().active;
    if (!a || a.session.id !== sessionId) return;
    const c = get().composer;
    const canReason = activeModelCapabilities().includes("reasoning");
    const enabled = canReason ? c.thinkingEnabled : false;
    const effort =
      canReason && c.thinkingEnabled && c.thinkingEffort.trim()
        ? c.thinkingEffort.trim()
        : null;
    const cur = a.session.llm_params;
    if ((cur.thinking_enabled ?? false) === enabled && (cur.thinking_effort ?? null) === effort) {
      return;
    }
    const llm: ModelParamSettings = {
      ...cur,
      thinking_enabled: enabled,
      thinking_effort: effort,
    };
    try {
      await api.updateSessionConfig(
        sessionId,
        a.session.system_prompt,
        a.session.history_turns,
        llm,
      );
      const now = get().active;
      if (now && now.session.id === sessionId) {
        set({ active: { ...now, session: { ...now.session, llm_params: llm } } });
      }
    } catch (e) {
      console.warn(e);
    }
  },

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
    let imageCount = cur.attachments.filter((a) =>
      a.mime.startsWith("image/"),
    ).length;
    let audioCount = cur.attachments.filter((a) =>
      a.mime.startsWith("audio/"),
    ).length;
    const videoModel = isVideoModel();
    for (const f of files) {
      if (uploads.length >= room) {
        rejected++;
        continue;
      }
      const mime = uploadMime(f);
      if (!ACCEPT_TYPES.includes(mime)) {
        rejected++;
        continue;
      }
      const isImage = mime.startsWith("image/");
      const isAudio = mime.startsWith("audio/");
      const invalidForMode = videoModel
        ? cur.videoMode === "text" ||
          ((cur.videoMode === "first_frame" ||
            cur.videoMode === "first_last") &&
            (!isImage ||
              imageCount >= (cur.videoMode === "first_frame" ? 1 : 2))) ||
          (cur.videoMode === "reference" &&
            ((!isImage && !isAudio) ||
              (isImage && imageCount >= 9) ||
              (isAudio && audioCount >= 3)))
        : !isImage;
      if (invalidForMode) {
        rejected++;
        continue;
      }
      if (f.size > maxBytesForMime(mime)) {
        rejected++;
        continue;
      }
      uploads.push({
        file: f,
        pending: makePendingAttachment(f.name || "image", f.size),
      });
      if (isImage) imageCount += 1;
      if (isAudio) audioCount += 1;
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
        const latest = get().composer;
        const imageCount = latest.attachments.filter((a) =>
          a.mime.startsWith("image/"),
        ).length;
        const audioCount = latest.attachments.filter((a) =>
          a.mime.startsWith("audio/"),
        ).length;
        const isImage = d.mime.startsWith("image/");
        const isAudio = d.mime.startsWith("audio/");
        const videoModel = isVideoModel();
        const invalidForMode =
          d.mime.startsWith("video/") ||
          (videoModel
            ? latest.videoMode === "text" ||
              ((latest.videoMode === "first_frame" ||
                latest.videoMode === "first_last") &&
                (!isImage ||
                  imageCount >=
                    (latest.videoMode === "first_frame" ? 1 : 2))) ||
              (latest.videoMode === "reference" &&
                ((!isImage && !isAudio) ||
                  (isImage && imageCount >= 9) ||
                  (isAudio && audioCount >= 3)))
            : !isImage);
        if (invalidForMode) {
          await api.removeAttachmentDraft(d.image_id).catch(() => {});
          rejected++;
          set({
            composer: {
              ...get().composer,
              pendingAttachments: get().composer.pendingAttachments.filter(
                (p) => p.id !== pending.id,
              ),
            },
          });
          continue;
        }
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

  addReferenceVideoUrl: async (url) => {
    const normalized = url.trim();
    if (!normalized || !/^(https?:\/\/|asset:\/\/)/i.test(normalized)) return;
    const current = get().composer;
    const videoCount = current.attachments.filter((a) => a.mime.startsWith("video/")).length;
    if (!isVideoModel() || current.videoMode !== "reference" || videoCount >= 3) return;
    const sid = await get().ensureActive();
    try {
      const draft = await api.addUrlAttachment(sid, normalized);
      set({
        composer: {
          ...get().composer,
          attachments: [...get().composer.attachments, draft],
        },
      });
    } catch (error) {
      console.error(error);
    }
  },

  removeAttachment: async (imageId) => {
    set({
      composer: {
        ...get().composer,
        attachments: get().composer.attachments.filter(
          (a) => a.image_id !== imageId,
        ),
      },
    });
    try {
      await api.removeAttachmentDraft(imageId);
    } catch (e) {
      console.warn(e);
    }
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

  setAskUserIndex: (index) => {
    const pending = get().pendingAskUser;
    if (!pending) return;
    if (index < 0 || index >= pending.questions.length) return;
    if (index === pending.activeIndex) return;
    const flushed = flushAskUserPrompt(pending, get().composer.prompt);
    const next: PendingAskUser = { ...flushed, activeIndex: index };
    askUserPendingBySession.set(next.sessionId, next);
    set({
      pendingAskUser: next,
      composer: {
        ...get().composer,
        // Input is custom-only — never restore option text into the editor.
        prompt: askUserCustomText(next, index),
      },
    });
  },

  setAskUserAnswer: (optionKey, optionText) => {
    const pending = get().pendingAskUser;
    if (!pending) return;
    const q = pending.questions[pending.activeIndex];
    if (!q) return;
    const key = questionKey(q, pending.activeIndex);
    const next: PendingAskUser = {
      ...pending,
      answers: {
        ...pending.answers,
        [key]: {
          optionKey,
          optionText,
          // Selecting an option clears custom input for this question.
          custom: "",
        },
      },
    };
    askUserPendingBySession.set(next.sessionId, next);
    set({
      pendingAskUser: next,
      // Do not fill the composer — keep it empty for optional custom reply.
      composer: { ...get().composer, prompt: "" },
    });
  },

  clearAskUserAnswer: () => {
    const pending = get().pendingAskUser;
    if (!pending) return;
    const q = pending.questions[pending.activeIndex];
    if (!q) return;
    const key = questionKey(q, pending.activeIndex);
    const prev = pending.answers[key];
    const custom = prev?.custom ?? get().composer.prompt;
    const nextAnswers = { ...pending.answers };
    if (custom.trim()) {
      nextAnswers[key] = { custom };
    } else {
      delete nextAnswers[key];
    }
    const next: PendingAskUser = { ...pending, answers: nextAnswers };
    askUserPendingBySession.set(next.sessionId, next);
    set({ pendingAskUser: next });
  },

  clearPendingAskUser: () => {
    const pending = get().pendingAskUser;
    if (pending) askUserPendingBySession.delete(pending.sessionId);
    set({ pendingAskUser: null });
  },

  answerPendingAskUser: async () => {
    const c = get().composer;
    let pending = get().pendingAskUser;
    if (!pending) return;
    const activeId = get().activeId;
    if (activeId && pending.sessionId !== activeId) return;

    pending = flushAskUserPrompt(pending, c.prompt);
    askUserPendingBySession.set(pending.sessionId, pending);
    set({ pendingAskUser: pending });

    const unfinished = firstUnansweredAskUserIndex(pending);
    if (unfinished >= 0) {
      if (unfinished !== pending.activeIndex) {
        get().setAskUserIndex(unfinished);
      }
      return;
    }

    const answer = formatAskUserReply(pending).trim();
    if (!answer) return;
    const items = formatAskUserItems(pending);
    const promptId = pending.blockId;

    askUserPendingBySession.delete(pending.sessionId);
    set({
      pendingAskUser: null,
      composer: { ...get().composer, prompt: "", mentions: [] },
    });
    composerDrafts.delete(pending.sessionId);

    try {
      await api.answerAskUser(promptId, answer, items);
    } catch (e) {
      console.warn("[atelier] answer_ask_user failed", e);
    }
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
      matchesImageRole(img.role) && img.mime.startsWith("image/"),
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
      const text = stripMediaMentionTokens(m.text || "");
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
    const videoModel = isVideoModel();
    if (!text && (!videoModel || m.images.length === 0)) return;
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
    await get().persistComposerThinking(sid);
    const inputMedia = m.images.filter((i) => i.role === "input");
    // Always use the LATEST composer/session params on resend — never the
    // parameters recorded on the original message. If the user changed anything
    // in the composer, the resend must reflect it.
    const effectiveVideoMode = videoModel
      ? coerceVideoModeForMedia(c.videoMode, inputMedia, text)
      : null;
    if (videoModel && !effectiveVideoMode) return;
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
            ...(videoModel && effectiveVideoMode
              ? {
                  video_mode: effectiveVideoMode,
                  video_duration: c.videoDuration,
                  video_resolution: c.videoResolution,
                  generate_audio: c.generateAudio,
                  watermark: c.watermark,
                }
              : {}),
          },
          effectiveSessionForLog(a.session),
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
        const wasCancelled = cancellingSessions.has(sid);
        cancellingSessions.delete(sid);
        if (epoch === getGenerationEpoch(sid)) {
          // Only remind when the run finished in the background: not user-
          // cancelled, and the user had already left this session.
          const remindInBackground = !wasCancelled && get().activeId !== sid;
          setSessionBusy(sid, false);
          if (remindInBackground) markSessionFinished(sid);
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
    // AskUser blocks the in-flight generation — answer it instead of starting
    // a new user turn.
    if (get().pendingAskUser) {
      await get().answerPendingAskUser();
      return;
    }

    const c = get().composer;
    const text = c.prompt.trim();
    const videoModel = isVideoModel();
    if (videoModel ? !validateVideoComposer(c, text) : !text) return;
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
      params: {
        aspect_ratio: c.aspectRatio,
        image_size: c.imageSize,
        ...(videoModel
          ? {
              video_mode: c.videoMode,
              video_duration: c.videoDuration,
              video_resolution: c.videoResolution,
              generate_audio: c.generateAudio,
              watermark: c.watermark,
            }
          : {}),
      },
      created_at: Date.now(),
      images: c.attachments.map((a, i) => ({
        id: a.image_id,
        role: "input",
        rel_path: a.rel_path,
        thumb_rel_path: a.thumb_rel_path,
        abs_path: a.abs_path,
        thumb_abs_path: a.thumb_abs_path,
        mime: a.mime,
        media_role: videoModel
          ? resolvedMediaRole(c.videoMode, a.mime, i)
          : a.media_role,
        source_url: a.source_url,
        width: a.width,
        height: a.height,
        bytes: a.bytes,
        ord: i,
      })),
    };

    const attachmentIds = c.attachments.map((a) => a.image_id);
    const aspectRatio = c.aspectRatio;
    const imageSize = c.imageSize;
    await get().persistComposerThinking(sid);

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
          active && active.session.id === sid
            ? effectiveSessionForLog(active.session)
            : null;
        await api.generateImage(
          {
            session_id: sid,
            prompt: text,
            attachment_ids: attachmentIds,
            aspect_ratio: aspectRatio,
            image_size: imageSize,
            ...(videoModel
              ? {
                  video_mode: c.videoMode,
                  video_duration: c.videoDuration,
                  video_resolution: c.videoResolution,
                  generate_audio: c.generateAudio,
                  watermark: c.watermark,
                }
              : {}),
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
        const wasCancelled = cancellingSessions.has(sid);
        cancellingSessions.delete(sid);
        if (epoch === getGenerationEpoch(sid)) {
          // Only remind when the run finished in the background: not user-
          // cancelled, and the user had already left this session.
          const remindInBackground = !wasCancelled && get().activeId !== sid;
          setSessionBusy(sid, false);
          if (remindInBackground) markSessionFinished(sid);
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
    // Drop any in-flight AskUser questionnaire for this session.
    askUserPendingBySession.delete(sid);
    if (get().pendingAskUser?.sessionId === sid) {
      set({ pendingAskUser: null });
    }
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
  videoMode: VideoGenerationMode;
  videoDuration: number;
  videoResolution: string;
  generateAudio: boolean;
  watermark: boolean;
}

const composerDrafts = new Map<string, ComposerDraft>();

function saveComposerDraft(sessionId: string, composer: ComposerState) {
  composerDrafts.set(sessionId, {
    prompt: composer.prompt,
    mentions: composer.mentions,
    attachments: composer.attachments,
    aspectRatio: composer.aspectRatio,
    imageSize: composer.imageSize,
    videoMode: composer.videoMode,
    videoDuration: composer.videoDuration,
    videoResolution: composer.videoResolution,
    generateAudio: composer.generateAudio,
    watermark: composer.watermark,
  });
}

function roleStateScopeForSession(sessionId: string): string {
  const state = useSession.getState();
  const session =
    state.active?.session.id === sessionId
      ? state.active.session
      : state.sessions.find((s) => s.id === sessionId);
  if (!session) return sessionId;
  return resolveRoleStateScope(session);
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
      content: extractJsonStringField(raw, "content"),
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
    const out = (output && typeof output === "object" ? output : {}) as Record<string, unknown>;

    // Backend returns the exact strings it matched/replaced (already
    // normalized), plus the match offset and full pre/post-edit text.
    const oldString = normalizeToolContent(
      typeof out.old_string === "string"
        ? out.old_string
        : typeof inp.old_string === "string"
          ? inp.old_string
          : "",
    );
    const newString = normalizeToolContent(
      typeof out.new_string === "string"
        ? out.new_string
        : typeof inp.new_string === "string"
          ? inp.new_string
          : "",
    );
    const replaceAll =
      out.replace_all === true || inp.replace_all === true;
    const matchStart =
      typeof out.match_start === "number" ? out.match_start : 0;

    const sessionId = useSession.getState().activeId;
    if (!sessionId) return;

    // The backend already wrote the edit; read the file back so the reader's
    // after-text is authoritative (never a stale in-memory replay that could
    // be auto-saved back over a correct on-disk edit).
    let diskText: string;
    let diskEncoding: string | undefined;
    let diskHadBom: boolean | undefined;
    try {
      const disk = await api.readProjectFile(sessionId, path);
      diskText = disk.text;
      diskEncoding = disk.encoding;
      diskHadBom = disk.hadBom;
    } catch (e) {
      console.warn("Edit: failed to load file for reader diff", e);
      return;
    }

    const textAfter = diskText;
    // Prefer the backend's authoritative pre-edit snapshot; fall back to
    // reconstructing it from the disk text and the applied replacement.
    const textBefore =
      typeof out.text_before === "string"
        ? out.text_before
        : revertStringEdit(diskText, oldString, newString, matchStart, replaceAll);

    if (!existing) {
      reader.openDoc(
        {
          path,
          text: diskText,
          fileType: inferFileType(path),
          encoding: diskEncoding,
          hadBom: diskHadBom,
        },
        { activate: false },
      );
      existing = reader.getTabByPath(path);
    }

    reader.appendPendingDiff(path, {
      before: normalizeDiffText(oldString),
      after: normalizeDiffText(newString),
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

function activatePendingAskUser(
  sessionId: string,
  blockId: string,
  input: unknown,
) {
  const questions = parseAskUserInput(input);
  if (questions.length === 0) return;
  const pending: PendingAskUser = {
    sessionId,
    blockId,
    questions,
    activeIndex: 0,
    answers: {},
  };
  askUserPendingBySession.set(sessionId, pending);
  const state = useSession.getState();
  if (state.activeId !== sessionId) return;
  setTimeout(() => {
    useSession.setState({
      pendingAskUser: pending,
      composer: { ...useSession.getState().composer, prompt: "" },
    });
    window.dispatchEvent(new CustomEvent("atelier:focus-composer"));
  }, 0);
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
        const prevInput =
          b.input && typeof b.input === "object"
            ? (b.input as Record<string, unknown>)
            : {};
        const nextInput =
          event.input && typeof event.input === "object"
            ? (event.input as Record<string, unknown>)
            : {};
        const input = { ...prevInput, ...nextInput };
        // Keep streamed doc fields when the final payload fails validation
        // (e.g. content sent as non-string) so the UI can still show them.
        if (
          (event.tool === "CreateDoc" || event.tool === "Edit") &&
          typeof input.content !== "string" &&
          typeof prevInput.content === "string"
        ) {
          input.content = prevInput.content;
        }
        if (
          event.tool === "CreateDoc" &&
          typeof input.title !== "string" &&
          typeof prevInput.title === "string"
        ) {
          input.title = prevInput.title;
        }
        blocks[i] = {
          ...b,
          tool: event.tool,
          input,
          status: "pending",
          streaming: false,
        };
        if (sessionId && event.tool === "AskUser") {
          activatePendingAskUser(sessionId, event.id, input);
        }
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
    if (sessionId && event.tool === "AskUser") {
      activatePendingAskUser(sessionId, event.id, event.input);
    }
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
      if (b.tool === "AskUser" && sessionId) {
        askUserPendingBySession.delete(sessionId);
        const state = useSession.getState();
        if (
          state.pendingAskUser?.sessionId === sessionId &&
          state.pendingAskUser.blockId === event.id
        ) {
          useSession.setState({ pendingAskUser: null });
        }
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
let generationStatusListenerStarted = false;
let toolEventListenerStarted = false;
let roleStateResetListenerStarted = false;
let sessionTitleListenerStarted = false;

function ensureGenerationStreamListener() {
  if (!generationStatusListenerStarted) {
    generationStatusListenerStarted = true;
    listen<GenerationStatusPayload>("gen://status", (event) => {
      const sessionId = event.payload?.session_id;
      const phase = event.payload?.phase;
      if (!sessionId || !phase) return;
      const state = useSession.getState();
      const next = { ...state.generationPhaseBySession };
      if (phase === "response") delete next[sessionId];
      else next[sessionId] = phase;
      useSession.setState({ generationPhaseBySession: next });
    }).catch((error) => {
      generationStatusListenerStarted = false;
      console.warn(error);
    });
  }

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
      // A buffer left over from a previous request (e.g. a generation that
      // finished while another session was active) must not leak into this
      // one — start from a clean slate when the requestId doesn't match.
      const existing = streamingBuffers.get(sessionId);
      const prev =
        existing && existing.requestId === requestId
          ? existing
          : { blocks: [], requestId };
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
      // Same stale-buffer guard as the gen://stream listener above.
      const existing = streamingBuffers.get(sessionId);
      const prev =
        existing && existing.requestId === requestId
          ? existing
          : { blocks: [], requestId };
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
          .applyOp(roleStateScopeForSession(sessionId), payload.output as RoleStateOp);
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
    listen<{ scope_id?: string; session_id?: string }>("role-state://reset", (event) => {
      const scopeId = event.payload?.scope_id;
      const sessionId = event.payload?.session_id;
      const state = useSession.getState();
      const activeScope = state.active
        ? roleStateScopeForSession(state.active.session.id)
        : null;
      if (scopeId && activeScope === scopeId && state.active) {
        void useRoleState
          .getState()
          .loadLatest(state.active.session.id, scopeId);
        return;
      }
      if (sessionId && state.activeId === sessionId) {
        void useRoleState
          .getState()
          .loadLatest(sessionId, roleStateScopeForSession(sessionId));
      }
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
