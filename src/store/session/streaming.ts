import { startTransition } from "react";
import { api } from "../../api/tauri";
import type { AssistantBlock, MessageAbs } from "../../types";
import {
  type PendingAskUser,
  parseAskUserInput,
} from "../../components/chat/askUser";
import { normalizeDiffText } from "../../utils/inlineDiff";
import { normalizeToolContent } from "../../utils/normalizeToolContent";
import {
  useReader,
  readerDocFromToolOutput,
  stripParagraphLabels,
  resolveToolFilePath,
  revertStringEdit,
  inferFileType,
} from "../reader";
import { useFileExplorer } from "../fileExplorer";
import { FS_TREE_TOOLS, STREAMING_INPUT_TOOLS } from "./constants";
import { askUserPendingBySession } from "./drafts";
import type { StreamBuffer, ToolEventPayload } from "./types";
import { useSession } from "./store";

export const streamingBuffers = new Map<string, StreamBuffer>();

/** Sessions the user interrupted; ignore late stream events until the invoke returns. */
export const cancellingSessions = new Set<string>();

/**
 * Append a text/thinking delta to a block list.
 * Text: merge with trailing text block, else push.
 * Thinking: merge into the current segment's thinking block (after the last
 * tool_use / agent_stage), or insert at the segment head so late-arriving
 * reasoning (common on Volcengine Responses) still appears above the answer.
 * Mutates `blocks` in place.
 */
export function appendDelta(
  blocks: AssistantBlock[],
  kind: "text" | "thinking",
  delta: string,
) {
  if (!delta) return;

  if (kind === "thinking") {
    let segmentStart = 0;
    for (let i = blocks.length - 1; i >= 0; i--) {
      const t = blocks[i].type;
      if (t === "tool_use" || t === "agent_stage") {
        segmentStart = i + 1;
        break;
      }
    }
    for (let i = segmentStart; i < blocks.length; i++) {
      const b = blocks[i];
      if (b.type === "thinking") {
        b.content = `${b.content}${delta}`;
        return;
      }
    }
    blocks.splice(segmentStart, 0, { type: "thinking", content: delta });
    return;
  }

  const last = blocks[blocks.length - 1];
  if (last && last.type === "text") {
    last.content = `${last.content}${delta}`;
    return;
  }
  blocks.push({ type: "text", content: delta });
}

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
      old_string: extractJsonStringField(raw, "old_string"),
      new_string: extractJsonStringField(raw, "new_string"),
    };
  }
  if (tool === "Write") {
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
export function applyStreamingToolCallDelta(
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

async function handleReaderToolComplete(
  tool: string,
  input: unknown,
  output: unknown,
  isError: boolean | undefined,
) {
  if (isError) return;
  if (FS_TREE_TOOLS.has(tool)) {
    useFileExplorer.getState().bumpTree();
  }
  const o = (output && typeof output === "object" ? output : {}) as Record<string, unknown>;
  const path = resolveToolFilePath(input, output);

  if (tool === "CreateDoc") {
    const doc = readerDocFromToolOutput(output);
    if (doc) useReader.getState().openDoc(doc);
    return;
  }

  if (tool === "Delete" && path) {
    useReader.getState().closeByPaths([path]);
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
    // A `pending_diff_id` means the backend recorded a reviewable hunk.
    const pendingDiffId =
      typeof out.pending_diff_id === "string" ? out.pending_diff_id : null;
    if (!pendingDiffId) {
      // Edit applied but too large (or failed) to record — refresh tab text only.
      const sessionId = useSession.getState().activeId;
      if (sessionId) {
        try {
          const disk = await api.readProjectFile(sessionId, path);
          if (!existing) {
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
          } else {
            reader.updateTabText(path, disk.text, { dirty: false });
          }
        } catch (e) {
          console.warn("Edit: failed to refresh reader after non-reviewable edit", e);
        }
      }
      return;
    }

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
      id: pendingDiffId,
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

export function applyToolEvent(
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
          event.tool === "CreateDoc" &&
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
        if (event.tool === "Edit") {
          if (
            typeof input.old_string !== "string" &&
            typeof prevInput.old_string === "string"
          ) {
            input.old_string = prevInput.old_string;
          }
          if (
            typeof input.new_string !== "string" &&
            typeof prevInput.new_string === "string"
          ) {
            input.new_string = prevInput.new_string;
          }
          if (
            typeof input.path !== "string" &&
            typeof prevInput.path === "string"
          ) {
            input.path = prevInput.path;
          }
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
      const outObj =
        event.output && typeof event.output === "object"
          ? (event.output as Record<string, unknown>)
          : null;
      const keepPending =
        !!event.keep_pending ||
        (event.tool === "Agent" &&
          !event.is_error &&
          outObj?.status === "running");
      if (keepPending) {
        blocks[i] = {
          ...b,
          status: "pending",
          output: event.output,
        };
        const childId =
          typeof outObj?.child_session_id === "string"
            ? outObj.child_session_id
            : null;
        if (childId) markChildSessionBusy(childId, true);
        return;
      }
      blocks[i] = {
        ...b,
        status: event.is_error ? "error" : "success",
        output: event.output,
        is_error: event.is_error || undefined,
      };
      if (!event.is_error) {
        void handleReaderToolComplete(b.tool, b.input, event.output, event.is_error);
      }
      if (b.tool === "Agent") {
        const prevOut =
          b.output && typeof b.output === "object"
            ? (b.output as Record<string, unknown>)
            : null;
        const childId =
          (typeof outObj?.child_session_id === "string"
            ? outObj.child_session_id
            : null) ||
          (typeof prevOut?.child_session_id === "string"
            ? prevOut.child_session_id
            : null);
        if (childId) {
          // Preserve child_session_id on error payloads so the card stays openable.
          const cur = blocks[i];
          if (
            event.is_error &&
            outObj &&
            typeof outObj.child_session_id !== "string" &&
            cur.type === "tool_use"
          ) {
            blocks[i] = {
              ...cur,
              output: { ...outObj, child_session_id: childId },
            };
          }
          markChildSessionBusy(childId, false);
          void reloadSessionIfViewing(childId);
        }
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

/** Mark a temp child session busy so switchTo can restore its stream buffer. */
function markChildSessionBusy(sessionId: string, busy: boolean) {
  useSession.setState((state) => {
    const busyBySession = { ...state.busyBySession };
    if (busy) busyBySession[sessionId] = true;
    else delete busyBySession[sessionId];
    const generationPhaseBySession = { ...state.generationPhaseBySession };
    if (!busy) delete generationPhaseBySession[sessionId];
    return {
      busyBySession,
      generationPhaseBySession,
      busy: state.activeId ? !!busyBySession[state.activeId] : false,
    };
  });
}

export async function reloadSessionIfViewing(sessionId: string) {
  cancelStreamFlushRaf(sessionId);
  streamingBuffers.delete(sessionId);
  const state = useSession.getState();
  if (state.activeId !== sessionId) return;
  try {
    const data = await api.loadSession(sessionId);
    useSession.setState({
      active: data,
      busy: !!useSession.getState().busyBySession[sessionId],
    });
  } catch (e) {
    console.warn(e);
  }
}

/** Apply a stream buffer as a temporary assistant message into active messages. */
export function applyStreamBufferToMessages(
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

export function cancelStreamFlushRaf(sessionId: string) {
  const rafId = streamFlushRafBySession.get(sessionId);
  if (rafId != null) {
    cancelAnimationFrame(rafId);
    streamFlushRafBySession.delete(sessionId);
  }
}

export function scheduleStreamingMessageSync(sessionId: string) {
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
export function freezeStreamingUi(sessionId: string) {
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
