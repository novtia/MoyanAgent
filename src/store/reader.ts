import { create } from "zustand";

/** Renderable document kind. `.md`/`.markdown` → markdown, everything else → text. */
export type ReaderFileType = "markdown" | "text";

/** Payload used when opening a document in the reader. */
export interface ReaderDoc {
  path: string;
  text: string;
  fileType: ReaderFileType;
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
  chars?: number;
  lines?: number;
  bytes?: number;
  truncated?: boolean;
  /** One entry per agent Edit call, in chronological order. */
  pendingDiffs: ReaderPendingDiff[];
  dirty?: boolean;
  saveError?: boolean;
}

interface ReaderStore {
  sessionId: string | null;
  tabs: ReaderFileTab[];
  activeTabId: string | null;
  openSeq: number;
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
  getTabByPath: (path: string) => ReaderFileTab | undefined;
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

/** Normalize paths for stable comparison across `/` and `\`. */
export function normalizeReaderPath(path: string): string {
  return path.replace(/\\/g, "/").toLowerCase();
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

/** Count non-whitespace characters (CJK-aware). */
export function countChars(text: string): number {
  let n = 0;
  for (const ch of text) {
    if (!/\s/.test(ch)) n += 1;
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
    path: doc.path,
    text,
    fileType: doc.fileType,
    chars: doc.chars ?? countChars(text),
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
  "path" | "text" | "fileType" | "chars" | "lines" | "bytes" | "truncated"
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
      ({ path, text, fileType, chars, lines, bytes, truncated }): PersistedTab => ({
        path,
        text,
        fileType,
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

export const useReader = create<ReaderStore>((set, get) => ({
  sessionId: null,
  tabs: [],
  activeTabId: null,
  openSeq: 0,

  bindSession: (sessionId) => {
    const loaded = loadPersisted(sessionId);
    set({
      sessionId,
      tabs: loaded.tabs,
      activeTabId: loaded.activeTabId,
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
                chars: doc.chars ?? countChars(cleanText),
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
              chars: countChars(text),
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
    set((s) => {
      const idx = findTabIndex(s.tabs, path);
      if (idx < 0) return s;
      const tabs = s.tabs.map((t, i) =>
        i === idx
          ? {
              ...t,
              pendingDiffs: [...t.pendingDiffs, block],
              text: block.textAfter,
              chars: countChars(block.textAfter),
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
              chars: countChars(revertText),
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

  getTabByPath: (path) => {
    const idx = findTabIndex(get().tabs, path);
    return idx >= 0 ? get().tabs[idx] : undefined;
  },

  clear: () => {
    set({ tabs: [], activeTabId: null });
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
    path,
    text: clean,
    fileType: inferFileType(path),
    chars: typeof o.chars === "number" ? o.chars : countChars(clean),
    lines: typeof o.lines === "number" ? o.lines : clean.split(/\n/).length,
    bytes: typeof o.bytes === "number" ? o.bytes : undefined,
    truncated: o.truncated === true,
  };
}
