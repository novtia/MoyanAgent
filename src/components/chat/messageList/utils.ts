import type { AssistantBlock } from "../../../types";
import { copyText as copyTextToClipboard } from "../../../utils/clipboard";
import type { ListFilesEntry, MessageTokenUsageData, RpgOption, TodoItem } from "./types";

export type TodoBlock = Extract<AssistantBlock, { type: "tool_use" }>;

export const tokenUsageFormatter = new Intl.NumberFormat();

export function resolveMessageTokenUsage(usage: MessageTokenUsageData): {
  prompt: number;
  completion: number;
} | null {
  const prompt = usage.prompt_tokens ?? 0;
  const completion = usage.completion_tokens ?? 0;
  if (prompt <= 0 && completion <= 0) return null;
  return { prompt, completion };
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

export function parseRpgChoiceInput(input: unknown): {
  prompt: string;
  options: RpgOption[];
} {
  const obj =
    input && typeof input === "object" ? (input as Record<string, unknown>) : {};
  const prompt = typeof obj.prompt === "string" ? obj.prompt : "";
  const rawOptions = Array.isArray(obj.options) ? obj.options : [];
  const options: RpgOption[] = rawOptions.flatMap((v) => {
    if (!v || typeof v !== "object") return [];
    const o = v as Record<string, unknown>;
    if (typeof o.label !== "string" || !o.label.trim()) return [];
    return [
      {
        id: typeof o.id === "string" ? o.id : undefined,
        label: o.label,
        text: typeof o.text === "string" ? o.text : undefined,
      },
    ];
  });
  return { prompt, options };
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
