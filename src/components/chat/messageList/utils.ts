import type { AssistantBlock } from "../../../types";
import { copyText as copyTextToClipboard } from "../../../utils/clipboard";
import type { ListFilesEntry, MessageTokenUsageData, TodoItem } from "./types";

export type TodoBlock = Extract<AssistantBlock, { type: "tool_use" }>;

export const tokenUsageFormatter = new Intl.NumberFormat();

export function resolveMessageTokenUsage(usage: MessageTokenUsageData): {
  prompt: number;
  completion: number;
  cacheRead: number;
  cacheWrite: number;
} | null {
  const prompt = usage.prompt_tokens ?? 0;
  const completion = usage.completion_tokens ?? 0;
  const cacheRead = usage.cache_read_tokens ?? 0;
  const cacheWrite = usage.cache_write_tokens ?? 0;
  if (prompt <= 0 && completion <= 0 && cacheRead <= 0 && cacheWrite <= 0) {
    return null;
  }
  return { prompt, completion, cacheRead, cacheWrite };
}

export function nativeFilePath(file: File) {
  return (file as File & { path?: string }).path || "";
}

const IMAGE_EXT_RE = /\.(png|jpe?g|webp|gif|bmp|svg)$/i;

export function isImageFile(file: File): boolean {
  return file.type.startsWith("image/") || IMAGE_EXT_RE.test(file.name);
}

export async function fileToBytes(file: File): Promise<Uint8Array> {
  const buf = await file.arrayBuffer();
  return new Uint8Array(buf);
}

export function nowStamp(ts: number) {
  const d = new Date(ts);
  return d.toTimeString().slice(0, 8);
}

/**
 * Spans shorter than this are treated as unmeasured rather than instant.
 *
 * Some providers hand over the whole reasoning in one chunk (Volcengine
 * Responses surfaces it on `response.completed`), which the typewriter split
 * replays with no delay at all. The elapsed span is then a few milliseconds of
 * local replay, not how long the model thought, so there is nothing honest to
 * show.
 */
export const MIN_MEASURABLE_THINKING_MS = 400;

/**
 * Split an elapsed span into the parts the duration labels interpolate.
 *
 * Sub-minute spans keep one decimal so the live counter visibly moves; past a
 * minute the tenths are noise and the minute becomes the useful unit.
 */
export function splitElapsed(ms: number): { minutes: number; seconds: string } {
  const total = Math.max(0, ms) / 1000;
  if (total < 60) {
    return { minutes: 0, seconds: total.toFixed(1) };
  }
  return {
    minutes: Math.floor(total / 60),
    seconds: String(Math.floor(total % 60)).padStart(2, "0"),
  };
}

export async function copyText(text: string) {
  try {
    await copyTextToClipboard(text);
  } catch (e) {
    console.warn(e);
  }
}

export function safeJsonStringify(v: unknown): string {
  if (v === undefined) return "";
  try {
    if (typeof v === "string") return v;
    return JSON.stringify(v, null, 2);
  } catch {
    return String(v);
  }
}

/**
 * Extract a human-readable error message from a failed tool call's output.
 * Backend errors arrive as `{ error: "..." }`; cancelled calls as a plain
 * string. Falls back to a JSON dump so the reason is never silently lost.
 */
export function extractToolErrorMessage(output: unknown): string {
  if (output == null) return "";
  if (typeof output === "string") return output;
  if (typeof output === "object") {
    const o = output as Record<string, unknown>;
    for (const key of ["error", "message", "detail"]) {
      const v = o[key];
      if (typeof v === "string" && v.trim()) return v;
    }
    return safeJsonStringify(output);
  }
  return String(output);
}

export function summarizeToolInput(input: unknown): string {
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
      o.title ?? o.path ?? o.file_path ?? o.command ?? o.query ?? o.prompt ?? o.name;
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

export function parseListFilesOutput(output: unknown): ListFilesEntry[] | null {
  if (!output || typeof output !== "object") return null;
  const entries = (output as { entries?: unknown }).entries;
  if (!Array.isArray(entries)) return null;
  return parseListFilesEntries(entries);
}

export function parseListFilesEntries(entries: unknown[]): ListFilesEntry[] {
  const parsed: ListFilesEntry[] = [];
  for (const row of entries) {
    if (!row || typeof row !== "object") continue;
    const o = row as Record<string, unknown>;
    if (typeof o.name !== "string") continue;
    const kind = o.kind === "directory" || o.kind === "file" ? o.kind : "file";
    const children =
      kind === "directory"
        ? Array.isArray(o.children)
          ? parseListFilesEntries(o.children)
          : []
        : undefined;
    const paragraphs =
      typeof o.paragraphs === "number" && Number.isFinite(o.paragraphs) && o.paragraphs >= 0
        ? o.paragraphs
        : undefined;
    parsed.push({ name: o.name, kind, children, paragraphs });
  }
  return parsed;
}

function parseRawItems(arr: unknown[]): TodoItem[] {
  return arr.flatMap((v) => {
    if (!v || typeof v !== "object") return [];
    const item = v as Record<string, unknown>;
    if (typeof item.id !== "number") return [];
    // Backend field is `title`; older sessions used `content`.
    const content =
      typeof item.title === "string"
        ? item.title
        : typeof item.content === "string"
          ? item.content
          : null;
    if (content == null) return [];
    return [
      {
        id: item.id as number,
        content,
        detail: typeof item.detail === "string" ? item.detail : undefined,
        status: (item.status as TodoItem["status"]) ?? "pending",
      },
    ];
  });
}

/**
 * Replay todo list state from all TodoList tool_use blocks in order.
 * The TodoList tool maintains its own state: `create` and `update` each
 * return the full item list, so we replace local state wholesale from the
 * latest successful block's `items`.
 */
export function replayTodoState(blocks: TodoBlock[]): {
  items: TodoItem[];
  busy: boolean;
} {
  let items: TodoItem[] = [];
  let busy = false;

  for (const block of blocks) {
    if (block.tool !== "TodoList") continue;

    if (block.status === "pending") {
      busy = true;
      continue;
    }
    if (block.status === "error") continue;

    const out =
      block.output && typeof block.output === "object"
        ? (block.output as Record<string, unknown>)
        : {};

    if (Array.isArray(out.items)) {
      items = parseRawItems(out.items as unknown[]);
    }
  }

  return { items, busy };
}

/** @deprecated Use {@link replayTodoState} — kept for call-site compatibility. */
export function replayTodoBlocks(blocks: TodoBlock[]): {
  items: TodoItem[];
  busy: boolean;
} {
  return replayTodoState(blocks);
}

function isPlainObject(v: unknown): v is Record<string, unknown> {
  return typeof v === "object" && v !== null && !Array.isArray(v);
}

/**
 * Mirrors backend `fix_tool_call::deep_merge`: objects merge recursively,
 * arrays and scalars (including null) replace.
 */
function deepMergeToolInput(base: unknown, patch: unknown): unknown {
  if (isPlainObject(base) && isPlainObject(patch)) {
    const out: Record<string, unknown> = { ...base };
    for (const [k, v] of Object.entries(patch)) {
      out[k] = k in out ? deepMergeToolInput(out[k], v) : v;
    }
    return out;
  }
  return patch;
}

/**
 * Fold `FixToolCall` blocks directly into the prior failed `tool_use` block
 * they amend so the UI updates the original tool call in place rather than
 * rendering a separate FixToolCall row.
 *
 * - Failed calls stay `status: "error"` (rendered red).
 * - Successfully amended calls take the merged input, new output,
 *   `status: "success"`, and `amended: true` (rendered blue).
 * - If a `FixToolCall` has no matching target in `blocks` (e.g. amending an
 *   earlier message or an unresolvable FixToolCall error), it is kept as-is.
 */
export function foldFixToolCalls(blocks: AssistantBlock[]): AssistantBlock[] {
  const hasFix = blocks.some(
    (b) => b.type === "tool_use" && b.tool === "FixToolCall",
  );
  if (!hasFix) return blocks;

  const result: AssistantBlock[] = [];

  const findTargetIndex = (
    targetId: string | null,
    amendedTool: string | null,
  ): number => {
    if (targetId) {
      // Prefer a matching error/amended-candidate block searching backwards
      // (handles providers like Gemini that reuse call ids across rounds).
      for (let i = result.length - 1; i >= 0; i--) {
        const b = result[i];
        if (b.type !== "tool_use" || b.tool === "FixToolCall") continue;
        if (b.id !== targetId) continue;
        if (amendedTool && b.tool !== amendedTool) continue;
        if (b.status === "error" || b.status === "pending") return i;
      }
      for (let i = result.length - 1; i >= 0; i--) {
        const b = result[i];
        if (b.type !== "tool_use" || b.tool === "FixToolCall") continue;
        if (b.id !== targetId) continue;
        if (amendedTool && b.tool !== amendedTool) continue;
        return i;
      }
      return -1;
    }
    // No explicit target id (typically while FixToolCall is still pending):
    // match the latest un-amended failed tool call, mirroring resolve_amendment.
    for (let i = result.length - 1; i >= 0; i--) {
      const b = result[i];
      if (b.type !== "tool_use" || b.tool === "FixToolCall") continue;
      if (b.status === "error") return i;
    }
    return -1;
  };

  for (const block of blocks) {
    if (block.type !== "tool_use" || block.tool !== "FixToolCall") {
      result.push(block);
      continue;
    }

    const inputObj = isPlainObject(block.input) ? block.input : null;
    const outputObj = isPlainObject(block.output) ? block.output : null;
    const rawTargetId =
      (typeof outputObj?.target_call_id === "string" &&
        outputObj.target_call_id.trim()) ||
      (typeof inputObj?.call_id === "string" && inputObj.call_id.trim()) ||
      null;
    const amendedTool =
      typeof outputObj?.amended_tool === "string" &&
      outputObj.amended_tool.trim()
        ? outputObj.amended_tool.trim()
        : null;

    // Only infer the latest error target when pending or when backend stamped
    // target_call_id. If FixToolCall errored before resolving a target (no
    // target_call_id and no explicit call_id), keep it visible as an error row.
    const canInferLatest = block.status === "pending";
    const targetIdx =
      rawTargetId || canInferLatest
        ? findTargetIndex(rawTargetId, amendedTool)
        : -1;

    if (targetIdx < 0) {
      result.push(
        block.status === "success" ? { ...block, amended: true } : block,
      );
      continue;
    }

    const target = result[targetIdx] as TodoBlock;
    const patchSet = isPlainObject(inputObj?.set) ? inputObj.set : null;
    const mergedInput = patchSet
      ? deepMergeToolInput(target.input, patchSet)
      : target.input;

    result[targetIdx] = {
      ...target,
      input: mergedInput,
      status: block.status,
      output: block.output !== undefined ? block.output : undefined,
      is_error: block.status === "error" ? true : undefined,
      streaming: block.streaming,
      amended: block.status === "success",
    };
  }

  return result;
}
