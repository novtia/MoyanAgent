import type {
  AttachmentDraft,
  ChainEntry,
  MessageAbs,
  ModelParamSettings,
  SessionSummary,
  SessionWithMessagesAbs,
  AssistantBlock,
} from "../../types";
import type { ComposerChatMode } from "../../config/chatMode";
import type { VideoGenerationMode } from "../../config/videoGeneration";
import type { PendingAskUser } from "../../components/chat/askUser";

export interface ComposerState {
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

export interface PendingAttachmentDraft {
  id: string;
  label: string;
  bytes: number | null;
}

export interface GenerationStreamPayload {
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

export interface GenerationStatusPayload {
  session_id: string;
  phase: "request" | "polling" | "response" | string;
}

export interface ToolUseEventPayload {
  session_id: string;
  request_message_id?: string;
  type: "tool_use";
  id: string;
  tool: string;
  input: unknown;
}

export interface ToolResultEventPayload {
  session_id: string;
  request_message_id?: string;
  type: "tool_result";
  id: string;
  tool?: string;
  output: unknown;
  is_error: boolean;
  /** When true, update output but keep the tool block pending (Agent mid-run). */
  keep_pending?: boolean;
}

export type ToolEventPayload = ToolUseEventPayload | ToolResultEventPayload;

export interface SessionStore {
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

export interface ComposerDraft {
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

export interface StreamBuffer {
  blocks: AssistantBlock[];
  requestId: string;
}
