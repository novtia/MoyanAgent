import { create } from "zustand";

/** Renderable document kind. `.md`/`.markdown` → markdown, everything else → text. */
export type ReaderFileType = "markdown" | "text";

/** A single document loaded into the right-panel reader. */
export interface ReaderDoc {
  /** Absolute path (used to derive the title and file type). */
  path: string;
  /** Full text content to render. */
  text: string;
  fileType: ReaderFileType;
  /** Non-whitespace character count (from the Read tool, when available). */
  chars?: number;
  lines?: number;
  bytes?: number;
  /** Whether the source text was truncated by the Read tool's byte cap. */
  truncated?: boolean;
}

interface ReaderStore {
  doc: ReaderDoc | null;
  /** Bumped on every `openDoc`; subscribers use it to react to open requests. */
  openSeq: number;
  openDoc: (doc: ReaderDoc) => void;
  clear: () => void;
}

/** Infer the renderable file type from a path's extension. */
export function inferFileType(path: string): ReaderFileType {
  const lower = path.toLowerCase();
  if (lower.endsWith(".md") || lower.endsWith(".markdown") || lower.endsWith(".mdx")) {
    return "markdown";
  }
  return "text";
}

/** Last path segment, tolerant of both `/` and `\` separators. */
export function readerFileName(path: string): string {
  const parts = path.split(/[/\\]/).filter(Boolean);
  return parts[parts.length - 1] || path;
}

/** Count non-whitespace characters (CJK-aware) as a fallback when the tool
 *  result omits `chars` (e.g. opened from a historical block). */
export function countChars(text: string): number {
  let n = 0;
  for (const ch of text) {
    if (!/\s/.test(ch)) n += 1;
  }
  return n;
}

export const useReader = create<ReaderStore>((set) => ({
  doc: null,
  openSeq: 0,
  openDoc: (doc) => set((s) => ({ doc, openSeq: s.openSeq + 1 })),
  clear: () => set({ doc: null }),
}));

/** Build a {@link ReaderDoc} from a Read tool's `output` payload. Returns null
 *  if the payload has no usable text. */
export function readerDocFromToolOutput(output: unknown): ReaderDoc | null {
  if (!output || typeof output !== "object") return null;
  const o = output as Record<string, unknown>;
  const text = typeof o.text === "string" ? o.text : null;
  if (text == null) return null;
  const path = typeof o.path === "string" ? o.path : "";
  return {
    path,
    text,
    fileType: inferFileType(path),
    chars: typeof o.chars === "number" ? o.chars : countChars(text),
    lines: typeof o.lines === "number" ? o.lines : text.split(/\n/).length,
    bytes: typeof o.bytes === "number" ? o.bytes : undefined,
    truncated: o.truncated === true,
  };
}
