import { create } from "zustand";
import { DEFAULT_TEXT_ENCODING } from "../types";

/** Renderable document kind. `.md`/`.markdown` → markdown, everything else → text. */
export type ReaderFileType = "markdown" | "text";

/** Payload used when opening a document in the reader. */
export interface ReaderDoc {
  path: string;
  text: string;
  fileType: ReaderFileType;
  encoding?: string;
  hadBom?: boolean;
  chars?: number;
  lines?: number;
  bytes?: number;
  truncated?: boolean;
}

export interface ReaderPendingDiff {
  id: string;
  /** Snippet removed (for inline diff display). */
  before: string;
  /** Snippet added (for inline diff display). */
  after: string;
  paragraphNumber?: number;
  /** Full file content before this Edit (for reject / restore). */
  textBefore: string;
  /** Full file content after this Edit. */
  textAfter: string;
}

/** One open file tab in the reader workspace. */
export interface ReaderFileTab {
  id: string;
  path: string;
  text: string;
  fileType: ReaderFileType;
  encoding: string;
  hadBom: boolean;
  chars?: number;
  lines?: number;
  bytes?: number;
  truncated?: boolean;
  /** One entry per agent Edit call, in chronological order. */
  pendingDiffs: ReaderPendingDiff[];
  dirty?: boolean;
  saveError?: boolean;
}

/** Path rewrite emitted when project files are renamed/moved (for UI chrome sync). */
export type ReaderPathOp =
  | { type: "remap"; from: string; to: string }
  | { type: "close"; paths: string[] };

interface ReaderStore {
  sessionId: string | null;
  tabs: ReaderFileTab[];
  activeTabId: string | null;
  openSeq: number;
  /** Bumps whenever open file paths are remapped or closed due to FS ops. */
  pathSeq: number;
  lastPathOps: ReaderPathOp[];
  bindSession: (sessionId: string | null) => void;
  openDoc: (doc: ReaderDoc, opts?: { activate?: boolean }) => void;
  closeTab: (id: string) => void;
  setActiveTab: (id: string) => void;
  updateTabText: (path: string, text: string, opts?: { dirty?: boolean }) => void;
  setTabDirty: (path: string, dirty: boolean, saveError?: boolean) => void;
  appendPendingDiff: (
    path: string,
    diff: Omit<ReaderPendingDiff, "id"> & { id?: string },
  ) => void;
  confirmDiffBlock: (
    path: string,
    blockId: string,
    accept: boolean,
  ) => { block: ReaderPendingDiff; revertText: string | null } | null;
  confirmAllDiffs: (path: string) => void;
  rejectAllDiffs: (path: string) => { revertText: string } | null;
  getTabByPath: (path: string) => ReaderFileTab | undefined;
  /** Rename/move a file or folder: update open tabs whose path equals or is under `from`. */
  remapPath: (from: string, to: string) => void;
  /** Batch rename/move (single pathSeq bump so UI consumers see every op). */
  remapPaths: (pairs: { from: string; to: string }[]) => void;
  /** Close open tabs for deleted files/folders (exact path or nested under). */
  closeByPaths: (paths: string[]) => void;
  clear: () => void;
}

const STORAGE_PREFIX = "atelier:reader-file-tabs:";

function newTabId() {
  return `ftab-${Date.now()}-${Math.random().toString(36).slice(2, 7)}`;
}

function newDiffId() {
  return `diff-${Date.now()}-${Math.random().toString(36).slice(2, 7)}`;
}

/** Split prose into paragraphs (one line each, matches backend). */
export function splitParagraphs(text: string): string[] {
  if (text === "") return [""];
  return text.split("\n");
}

/**
 * Reconstruct the pre-edit file text from the authoritative post-edit disk
 * text and the applied string replacement, so the reader diff's reject/restore
 * stays exact without replaying against a possibly-stale in-memory tab.
 *
 * `matchStart` is the code-point offset of the first match in the file (as
 * reported by the backend). For a single replacement we splice `oldString`
 * back in at that offset; for `replaceAll` we swap every `newString`
 * occurrence back to `oldString`.
 */
export function revertStringEdit(
  diskTextAfter: string,
  oldString: string,
  newString: string,
  matchStart: number,
  replaceAll: boolean,
): string {
  if (replaceAll) {
    if (newString === "") return diskTextAfter;
    return diskTextAfter.split(newString).join(oldString);
  }
  const after = Array.from(diskTextAfter);
  const newLen = Array.from(newString).length;
  if (matchStart < 0 || matchStart > after.length) return diskTextAfter;
  const head = after.slice(0, matchStart).join("");
  const tail = after.slice(matchStart + newLen).join("");
  return head + oldString + tail;
}

/** 1-based paragraph index, matches Edit `paragraph_number`. */
export function paragraphAt(text: string, oneBased: number): string {
  const paras = splitParagraphs(text);
  const idx = oneBased - 1;
  if (idx < 0 || idx >= paras.length) return "";
  return paras[idx] ?? "";
}

/** Format 1-based paragraph index like Read tool output (`[P001]`). */
export function formatParagraphLabel(oneBased: number): string {
  return `[P${String(oneBased).padStart(3, "0")}]`;
}

/** Compact paragraph index for reader gutter (1, 2, 3…). */
export function formatParagraphNumber(oneBased: number): string {
  return String(oneBased);
}

/**
 * For each line, its 1-based paragraph index (one line = one paragraph).
 */
export function buildLineParagraphLabels(text: string): (number | null)[] {
  return text.split("\n").map((_, i) => i + 1);
}

/** Strip Windows `\\?\` extended path prefix and normalize for comparison. */
export function normalizeReaderPath(path: string): string {
  let p = path.trim();
  if (p.startsWith("\\\\?\\")) {
    p = p.slice(4);
  }
  return p.replace(/\\/g, "/").toLowerCase();
}

/** Store/display path without the extended prefix. */
export function sanitizeReaderPath(path: string): string {
  let p = path.trim();
  if (p.startsWith("\\\\?\\")) {
    p = p.slice(4);
  }
  return p;
}

export function resolveToolFilePath(input: unknown, output: unknown): string {
  const inp = input && typeof input === "object" ? (input as Record<string, unknown>) : {};
  const o = output && typeof output === "object" ? (output as Record<string, unknown>) : {};
  // Prefer the backend-resolved absolute path so reader tabs / diffs stay keyed
  // consistently even when the model passes a project-relative breadcrumb in input.
  const raw =
    (typeof o.path === "string" && o.path.trim()) ||
    (typeof inp.path === "string" && inp.path.trim()) ||
    "";
  return raw ? sanitizeReaderPath(raw) : "";
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

/** Parent path segments (folder trail), excluding the file name. */
export function readerFolderLabel(path: string): string {
  const parts = sanitizeReaderPath(path).split(/[/\\]/).filter(Boolean);
  if (parts.length <= 1) return "";
  return parts.slice(0, -1).join("/");
}

function readToolPath(input: unknown, output: unknown): string {
  const o = input && typeof input === "object" ? (input as Record<string, unknown>) : {};
  const out = output && typeof output === "object" ? (output as Record<string, unknown>) : {};
  const raw =
    (typeof out.path === "string" && out.path.trim()) ||
    (typeof o.path === "string" && o.path.trim()) ||
    "";
  return raw ? sanitizeReaderPath(raw) : "";
}

function readToolParagraph(v: unknown): number | undefined {
  if (typeof v === "number" && Number.isFinite(v) && v >= 1) return Math.trunc(v);
  if (typeof v === "string" && /^\d+$/.test(v)) {
    const n = parseInt(v, 10);
    return n >= 1 ? n : undefined;
  }
  return undefined;
}

function formatReadParagraphRange(from: number, to: number): string {
  const pad = (n: number) => String(n).padStart(3, "0");
  if (from === to) return `P${pad(from)}`;
  return `P${pad(from)}–P${pad(to)}`;
}

/** Chat card title for a `Read` tool call — file name or folder + paragraph range. */
export function formatReadToolTitle(input: unknown, output: unknown): string {
  const path = readToolPath(input, output);
  if (!path) return "";

  const inp = input && typeof input === "object" ? (input as Record<string, unknown>) : {};
  const out = output && typeof output === "object" ? (output as Record<string, unknown>) : {};
  const ranged =
    out.ranged === true ||
    inp.paragraph_from != null ||
    inp.paragraph_to != null;
  const from = readToolParagraph(out.paragraph_from) ?? readToolParagraph(inp.paragraph_from);
  const to =
    readToolParagraph(out.paragraph_to) ??
    readToolParagraph(inp.paragraph_to) ??
    from;

  if (ranged && from != null) {
    const folder = readerFolderLabel(path);
    const range = formatReadParagraphRange(from, to ?? from);
    return folder ? `${folder} · ${range}` : range;
  }
  return readerFileName(path);
}

/**
 * Unified document word count used throughout the frontend.
 *
 * For Chinese-writing workflows, each non-whitespace Unicode character counts
 * as one word. Display stats are always derived from the normalized text in
 * the frontend, not from tool-result numeric fields (models may mis-sum them).
 */
const UNICODE_WHITESPACE = /^\p{White_Space}$/u;

export function countWords(text: string): number {
  let n = 0;
  for (const ch of text) {
    if (!UNICODE_WHITESPACE.test(ch)) n += 1;
  }
  return n;
}

/** Strip `[P001]` paragraph labels from Read tool output. */
export function stripParagraphLabels(text: string): string {
  const lines = text.split("\n");
  const out: string[] = [];
  for (const line of lines) {
    const m = line.match(/^\[P\d{3,}\]\s?(.*)$/);
    out.push(m ? m[1] : line);
  }
  return out.join("\n");
}

function docToTab(doc: ReaderDoc): ReaderFileTab {
  const text = stripParagraphLabels(doc.text);
  return {
    id: newTabId(),
    path: sanitizeReaderPath(doc.path),
    text,
    fileType: doc.fileType,
    encoding: doc.encoding ?? DEFAULT_TEXT_ENCODING,
    hadBom: doc.hadBom ?? false,
    chars: countWords(text),
    lines: doc.lines ?? text.split(/\n/).length,
    bytes: doc.bytes,
    truncated: doc.truncated,
    pendingDiffs: [],
    dirty: false,
    saveError: false,
  };
}

type PersistedTab = Pick<
  ReaderFileTab,
  | "path"
  | "text"
  | "fileType"
  | "encoding"
  | "hadBom"
  | "chars"
  | "lines"
  | "bytes"
  | "truncated"
>;

function loadPersisted(sessionId: string | null): {
  tabs: ReaderFileTab[];
  activeTabId: string | null;
} {
  if (!sessionId || typeof window === "undefined") {
    return { tabs: [], activeTabId: null };
  }
  try {
    const raw = window.localStorage.getItem(`${STORAGE_PREFIX}${sessionId}`);
    if (!raw) return { tabs: [], activeTabId: null };
    const parsed = JSON.parse(raw) as {
      tabs?: PersistedTab[];
      activePath?: string | null;
    };
    const tabs: ReaderFileTab[] = (parsed.tabs ?? []).map((t) => ({
      ...t,
      path: sanitizeReaderPath(t.path),
      encoding: t.encoding ?? DEFAULT_TEXT_ENCODING,
      hadBom: t.hadBom ?? false,
      id: newTabId(),
      pendingDiffs: [],
      dirty: false,
      saveError: false,
    }));
    const activePath = parsed.activePath ?? null;
    const activeTabId =
      activePath != null
        ? tabs.find((t) => normalizeReaderPath(t.path) === normalizeReaderPath(activePath))?.id ??
          tabs[0]?.id ??
          null
        : tabs[0]?.id ?? null;
    return { tabs, activeTabId };
  } catch {
    return { tabs: [], activeTabId: null };
  }
}

function persistTabs(sessionId: string | null, tabs: ReaderFileTab[], activeTabId: string | null) {
  if (!sessionId || typeof window === "undefined") return;
  const active = tabs.find((t) => t.id === activeTabId);
  const payload = {
    tabs: tabs.map(
      ({
        path,
        text,
        fileType,
        encoding,
        hadBom,
        chars,
        lines,
        bytes,
        truncated,
      }): PersistedTab => ({
        path,
        text,
        fileType,
        encoding,
        hadBom,
        chars,
        lines,
        bytes,
        truncated,
      }),
    ),
    activePath: active?.path ?? null,
  };
  try {
    window.localStorage.setItem(`${STORAGE_PREFIX}${sessionId}`, JSON.stringify(payload));
  } catch {
    /* ignore quota */
  }
}

function findTabIndex(tabs: ReaderFileTab[], path: string): number {
  const key = normalizeReaderPath(path);
  return tabs.findIndex((t) => normalizeReaderPath(t.path) === key);
}

function pathSep(path: string): string {
  return path.includes("\\") ? "\\" : "/";
}

/**
 * If `path` equals `from` or is nested under it, return the path with that
 * prefix replaced by `to`. Otherwise null.
 */
export function rewritePathPrefix(path: string, from: string, to: string): string | null {
  if (!path || !from || !to) return null;
  const normPath = normalizeReaderPath(path);
  const normFrom = normalizeReaderPath(from);
  if (!normPath || !normFrom) return null;
  if (normPath !== normFrom && !normPath.startsWith(`${normFrom}/`)) return null;

  const pathSan = sanitizeReaderPath(path);
  const fromSan = sanitizeReaderPath(from);
  const toSan = sanitizeReaderPath(to);
  let suffix = "";
  if (
    pathSan.length >= fromSan.length &&
    normalizeReaderPath(pathSan.slice(0, fromSan.length)) === normFrom
  ) {
    suffix = pathSan.slice(fromSan.length);
  } else {
    suffix = normPath.slice(normFrom.length).replace(/\//g, pathSep(toSan));
  }
  if (!suffix) return toSan;
  const sep = pathSep(toSan);
  const base = toSan.replace(/[/\\]+$/, "");
  const rest = suffix.replace(/^[/\\]+/, "");
  return rest ? `${base}${sep}${rest}` : base;
}

function pathMatchesOrUnder(path: string, root: string): boolean {
  const p = normalizeReaderPath(path);
  const r = normalizeReaderPath(root);
  if (!p || !r) return false;
  return p === r || p.startsWith(`${r}/`);
}

/** Apply FS path ops to a list of panel/file paths (null paths pass through). */
export function applyReaderPathOpsToPath(
  path: string | null | undefined,
  ops: ReaderPathOp[],
): string | null | undefined {
  if (path == null) return path;
  let next: string | null = path;
  for (const op of ops) {
    if (next == null) return null;
    if (op.type === "close") {
      if (op.paths.some((root) => pathMatchesOrUnder(next!, root))) return null;
      continue;
    }
    const rewritten = rewritePathPrefix(next, op.from, op.to);
    if (rewritten != null) next = rewritten;
  }
  return next;
}

export const useReader = create<ReaderStore>((set, get) => ({
  sessionId: null,
  tabs: [],
  activeTabId: null,
  openSeq: 0,
  pathSeq: 0,
  lastPathOps: [],

  bindSession: (sessionId) => {
    const loaded = loadPersisted(sessionId);
    set({
      sessionId,
      tabs: loaded.tabs,
      activeTabId: loaded.activeTabId,
      lastPathOps: [],
    });
  },

  openDoc: (doc, opts) => {
    const activate = opts?.activate !== false;
    set((s) => {
      const idx = findTabIndex(s.tabs, doc.path);
      let tabs = s.tabs;
      let activeTabId = s.activeTabId;

      if (idx >= 0) {
        const existing = s.tabs[idx];
        // Keep in-flight edits / pending diff hunks when re-opening from Read.
        if (existing.pendingDiffs.length > 0 || existing.dirty) {
          if (activate) activeTabId = existing.id;
          persistTabs(s.sessionId, tabs, activeTabId);
          return {
            tabs,
            activeTabId,
            openSeq: s.openSeq + 1,
          };
        }

        const cleanText = stripParagraphLabels(doc.text);
        tabs = s.tabs.map((t, i) =>
          i === idx
            ? {
                ...t,
                text: cleanText,
                fileType: doc.fileType,
                encoding: doc.encoding ?? t.encoding,
                hadBom: doc.hadBom ?? t.hadBom,
                chars: countWords(cleanText),
                lines: doc.lines ?? cleanText.split(/\n/).length,
                bytes: doc.bytes,
                truncated: doc.truncated,
                pendingDiffs: [],
                dirty: false,
                saveError: false,
              }
            : t,
        );
        if (activate) activeTabId = tabs[idx].id;
      } else {
        const tab = docToTab(doc);
        tabs = [...s.tabs, tab];
        if (activate) activeTabId = tab.id;
      }

      persistTabs(s.sessionId, tabs, activeTabId);
      return {
        tabs,
        activeTabId,
        openSeq: s.openSeq + 1,
      };
    });
  },

  closeTab: (id) => {
    set((s) => {
      const idx = s.tabs.findIndex((t) => t.id === id);
      if (idx < 0) return s;
      const tabs = s.tabs.filter((t) => t.id !== id);
      let activeTabId = s.activeTabId;
      if (activeTabId === id) {
        activeTabId = tabs[Math.min(idx, tabs.length - 1)]?.id ?? null;
      }
      persistTabs(s.sessionId, tabs, activeTabId);
      return { tabs, activeTabId };
    });
  },

  setActiveTab: (id) => {
    set((s) => {
      if (!s.tabs.some((t) => t.id === id)) return s;
      persistTabs(s.sessionId, s.tabs, id);
      return { activeTabId: id };
    });
  },

  updateTabText: (path, text, opts) => {
    set((s) => {
      const idx = findTabIndex(s.tabs, path);
      if (idx < 0) return s;
      const tabs = s.tabs.map((t, i) =>
        i === idx
          ? {
              ...t,
              text,
              chars: countWords(text),
              lines: text.split(/\n/).length,
              dirty: opts?.dirty ?? t.dirty,
              saveError: opts?.dirty ? false : t.saveError,
            }
          : t,
      );
      persistTabs(s.sessionId, tabs, s.activeTabId);
      return { tabs };
    });
  },

  setTabDirty: (path, dirty, saveError = false) => {
    set((s) => {
      const idx = findTabIndex(s.tabs, path);
      if (idx < 0) return s;
      const tabs = s.tabs.map((t, i) =>
        i === idx ? { ...t, dirty, saveError } : t,
      );
      return { tabs };
    });
  },

  appendPendingDiff: (path, diff) => {
    const block: ReaderPendingDiff = { ...diff, id: diff.id ?? newDiffId() };
    const key = normalizeReaderPath(path);
    set((s) => {
      const idx = s.tabs.findIndex((t) => normalizeReaderPath(t.path) === key);
      if (idx < 0) return s;
      const tabs = s.tabs.map((t, i) =>
        i === idx
          ? {
              ...t,
              pendingDiffs: [...t.pendingDiffs, block],
              text: block.textAfter,
              chars: countWords(block.textAfter),
              lines: block.textAfter.split(/\n/).length,
              dirty: false,
              saveError: false,
            }
          : t,
      );
      persistTabs(s.sessionId, tabs, s.activeTabId);
      return { tabs, openSeq: s.openSeq + 1 };
    });
  },

  confirmDiffBlock: (path, blockId, accept) => {
    const s = get();
    const idx = findTabIndex(s.tabs, path);
    if (idx < 0) return null;
    const tab = s.tabs[idx];
    const blockIdx = tab.pendingDiffs.findIndex((d) => d.id === blockId);
    if (blockIdx < 0) return null;
    const block = tab.pendingDiffs[blockIdx];

    if (accept) {
      const nextDiffs = tab.pendingDiffs.filter((d) => d.id !== blockId);
      set({
        tabs: s.tabs.map((t, i) =>
          i === idx
            ? {
                ...t,
                pendingDiffs: nextDiffs,
                dirty: false,
                saveError: false,
              }
            : t,
        ),
      });
      persistTabs(s.sessionId, get().tabs, s.activeTabId);
      return { block, revertText: null };
    }

    const revertText = block.textBefore;
    const nextDiffs = tab.pendingDiffs.slice(0, blockIdx);
    set({
      tabs: s.tabs.map((t, i) =>
        i === idx
          ? {
              ...t,
              pendingDiffs: nextDiffs,
              text: revertText,
              chars: countWords(revertText),
              lines: revertText.split(/\n/).length,
              dirty: false,
              saveError: false,
            }
          : t,
      ),
    });
    persistTabs(s.sessionId, get().tabs, s.activeTabId);
    return { block, revertText };
  },

  confirmAllDiffs: (path) => {
    const s = get();
    const idx = findTabIndex(s.tabs, path);
    if (idx < 0) return;
    const tab = s.tabs[idx];
    if (tab.pendingDiffs.length === 0) return;
    set({
      tabs: s.tabs.map((t, i) =>
        i === idx
          ? {
              ...t,
              pendingDiffs: [],
              dirty: false,
              saveError: false,
            }
          : t,
      ),
    });
    persistTabs(s.sessionId, get().tabs, s.activeTabId);
  },

  rejectAllDiffs: (path) => {
    const s = get();
    const idx = findTabIndex(s.tabs, path);
    if (idx < 0) return null;
    const tab = s.tabs[idx];
    if (tab.pendingDiffs.length === 0) return null;
    const revertText = tab.pendingDiffs[0].textBefore;
    set({
      tabs: s.tabs.map((t, i) =>
        i === idx
          ? {
              ...t,
              pendingDiffs: [],
              text: revertText,
              chars: countWords(revertText),
              lines: revertText.split(/\n/).length,
              dirty: false,
              saveError: false,
            }
          : t,
      ),
    });
    persistTabs(s.sessionId, get().tabs, s.activeTabId);
    return { revertText };
  },

  getTabByPath: (path) => {
    const idx = findTabIndex(get().tabs, path);
    return idx >= 0 ? get().tabs[idx] : undefined;
  },

  remapPath: (from, to) => {
    get().remapPaths([{ from, to }]);
  },

  remapPaths: (pairs) => {
    const ops = pairs
      .filter(
        (p) =>
          !!p.from &&
          !!p.to &&
          normalizeReaderPath(p.from) !== normalizeReaderPath(p.to),
      )
      .map((p) => ({ type: "remap" as const, from: p.from, to: p.to }));
    if (ops.length === 0) return;
    set((s) => {
      let changed = false;
      let tabs = s.tabs;
      /** Tab ids whose path was rewritten (exact file or under a renamed folder). */
      const touchedIds = new Set<string>();
      for (const op of ops) {
        const seen = new Set<string>();
        const next: ReaderFileTab[] = [];
        for (const tab of tabs) {
          const rewritten = rewritePathPrefix(tab.path, op.from, op.to);
          if (rewritten == null) {
            next.push(tab);
            seen.add(normalizeReaderPath(tab.path));
            continue;
          }
          changed = true;
          const key = normalizeReaderPath(rewritten);
          // Destination already open: drop the old-path tab (close old, keep new).
          if (seen.has(key)) {
            const kept = next.find((t) => normalizeReaderPath(t.path) === key);
            if (kept) touchedIds.add(kept.id);
            continue;
          }
          seen.add(key);
          touchedIds.add(tab.id);
          next.push({
            ...tab,
            path: rewritten,
            fileType: inferFileType(rewritten),
          });
        }
        tabs = next;
      }
      if (!changed) return s;
      // Prefer activating a rewritten tab so the panel opens the new path.
      let activeTabId = s.activeTabId;
      if (!activeTabId || !tabs.some((t) => t.id === activeTabId)) {
        activeTabId =
          tabs.find((t) => touchedIds.has(t.id))?.id ?? tabs[0]?.id ?? null;
      } else if (!touchedIds.has(activeTabId)) {
        const touched = tabs.find((t) => touchedIds.has(t.id));
        if (touched) activeTabId = touched.id;
      }
      persistTabs(s.sessionId, tabs, activeTabId);
      return {
        tabs,
        activeTabId,
        // openSeq forces the right-panel chrome to open/focus the new path.
        openSeq: s.openSeq + 1,
        pathSeq: s.pathSeq + 1,
        lastPathOps: ops,
      };
    });
  },

  closeByPaths: (paths) => {
    const roots = paths.map((p) => p.trim()).filter(Boolean);
    if (roots.length === 0) return;
    set((s) => {
      const tabs = s.tabs.filter(
        (t) => !roots.some((root) => pathMatchesOrUnder(t.path, root)),
      );
      if (tabs.length === s.tabs.length) return s;
      let activeTabId = s.activeTabId;
      if (activeTabId && !tabs.some((t) => t.id === activeTabId)) {
        activeTabId = tabs[0]?.id ?? null;
      }
      persistTabs(s.sessionId, tabs, activeTabId);
      return {
        tabs,
        activeTabId,
        pathSeq: s.pathSeq + 1,
        lastPathOps: [{ type: "close", paths: roots }],
      };
    });
  },

  clear: () => {
    set({ tabs: [], activeTabId: null, lastPathOps: [] });
  },
}));

/** Build a {@link ReaderDoc} from a tool `output` payload. */
export function readerDocFromToolOutput(output: unknown): ReaderDoc | null {
  if (!output || typeof output !== "object") return null;
  const o = output as Record<string, unknown>;
  const text = typeof o.text === "string" ? o.text : null;
  if (text == null) return null;
  const path = typeof o.path === "string" ? o.path : "";
  const clean = stripParagraphLabels(text);
  return {
    path: sanitizeReaderPath(path),
    text: clean,
    fileType: inferFileType(path),
    encoding:
      typeof o.encoding === "string" && o.encoding.trim()
        ? o.encoding.trim()
        : DEFAULT_TEXT_ENCODING,
    hadBom: o.had_bom === true || o.hadBom === true,
    // Always calculate display stats from the normalized frontend text. Tool
    // outputs may come from older versions or use a different counting source.
    chars: countWords(clean),
    lines: typeof o.lines === "number" ? o.lines : clean.split(/\n/).length,
    bytes: typeof o.bytes === "number" ? o.bytes : undefined,
    truncated: o.truncated === true,
  };
}
