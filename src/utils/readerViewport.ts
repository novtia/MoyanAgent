/** Last scrollTop per open reader path + view, so agent Create/Edit cannot
 *  yank the viewport: switching between the plain editor and the diff view
 *  (or remounting either) restores where the user was reading. Preview and
 *  source keep separate positions — they do not share a scroller. */
const scrollTopByKey = new Map<string, number>();

function normalizePath(path: string): string {
  let p = path.trim();
  if (p.startsWith("\\\\?\\")) p = p.slice(4);
  return p.replace(/\\/g, "/").toLowerCase();
}

export type ReaderViewportSlot = "source" | "preview";

function storageKey(path: string, slot: ReaderViewportSlot): string {
  return `${normalizePath(path)}::${slot}`;
}

export function rememberReaderViewport(
  path: string | undefined,
  scrollTop: number,
  slot: ReaderViewportSlot = "source",
) {
  const trimmed = path?.trim();
  if (!trimmed) return;
  scrollTopByKey.set(storageKey(trimmed, slot), Math.max(0, scrollTop));
}

export function readerViewportTop(
  path: string | undefined,
  slot: ReaderViewportSlot = "source",
): number {
  const trimmed = path?.trim();
  if (!trimmed) return 0;
  return scrollTopByKey.get(storageKey(trimmed, slot)) ?? 0;
}

// ───────────── persisted reading progress (survives restart) ─────────────
// Pixel offsets above are exact but only valid for this run and this layout.
// The line (paragraph) the viewport top sits on stays meaningful after a
// restart, a font-size change or a window resize, so it is what we persist.

export interface ReaderProgress {
  /** 0-based document line at the top of the viewport. */
  line: number;
  /** Line count when recorded; lets restore clamp after the file changed. */
  total: number;
  /** scrollTop / scrollable height, for views without a line notion (preview). */
  ratio: number;
  at: number;
}

const PROGRESS_KEY = "lumen.reader.progress.v1";
const PROGRESS_CAP = 300;
const FLUSH_DELAY_MS = 400;

let progressCache: Map<string, ReaderProgress> | null = null;
let flushTimer: ReturnType<typeof setTimeout> | null = null;

function loadProgress(): Map<string, ReaderProgress> {
  if (progressCache) return progressCache;
  const map = new Map<string, ReaderProgress>();
  try {
    const raw =
      typeof window !== "undefined" ? window.localStorage.getItem(PROGRESS_KEY) : null;
    if (raw) {
      const obj = JSON.parse(raw) as Record<string, Partial<ReaderProgress>>;
      for (const [k, v] of Object.entries(obj)) {
        if (!v || typeof v !== "object") continue;
        map.set(k, {
          line: Number.isFinite(v.line) ? Math.max(0, Math.floor(v.line as number)) : 0,
          total: Number.isFinite(v.total) ? Math.max(0, Math.floor(v.total as number)) : 0,
          ratio: Number.isFinite(v.ratio) ? Math.min(1, Math.max(0, v.ratio as number)) : 0,
          at: Number.isFinite(v.at) ? (v.at as number) : 0,
        });
      }
    }
  } catch {
    /* corrupt or unavailable storage: start empty */
  }
  progressCache = map;
  return map;
}

/** Write the cache to localStorage now (also called on unload). */
export function flushReaderProgress() {
  if (flushTimer) {
    clearTimeout(flushTimer);
    flushTimer = null;
  }
  const map = progressCache;
  if (!map || typeof window === "undefined") return;
  if (map.size > PROGRESS_CAP) {
    const keep = [...map.entries()].sort((a, b) => b[1].at - a[1].at).slice(0, PROGRESS_CAP);
    map.clear();
    for (const [k, v] of keep) map.set(k, v);
  }
  try {
    window.localStorage.setItem(PROGRESS_KEY, JSON.stringify(Object.fromEntries(map)));
  } catch {
    /* quota / disabled storage: progress is best-effort */
  }
}

function scheduleFlush() {
  if (flushTimer) return;
  flushTimer = setTimeout(flushReaderProgress, FLUSH_DELAY_MS);
}

if (typeof window !== "undefined") {
  window.addEventListener("beforeunload", flushReaderProgress);
}

function patchProgress(path: string | undefined, patch: Partial<ReaderProgress>) {
  const trimmed = path?.trim();
  if (!trimmed) return;
  const map = loadProgress();
  const key = normalizePath(trimmed);
  const prev = map.get(key) ?? { line: 0, total: 0, ratio: 0, at: 0 };
  map.set(key, { ...prev, ...patch, at: Date.now() });
  scheduleFlush();
}

export function rememberReaderLine(path: string | undefined, line: number, total: number) {
  const t = Math.max(1, Math.floor(total));
  const l = Math.min(Math.max(0, Math.floor(line)), t - 1);
  patchProgress(path, { line: l, total: t, ratio: t > 1 ? l / (t - 1) : 0 });
}

export function rememberReaderRatio(path: string | undefined, ratio: number) {
  const r = Math.min(1, Math.max(0, ratio));
  const prev = readerSavedProgress(path);
  // Keep the line in step so the source view opens where the preview left off.
  const line = prev && prev.total > 1 ? Math.round(r * (prev.total - 1)) : prev?.line;
  patchProgress(path, line != null ? { ratio: r, line } : { ratio: r });
}

export function readerSavedProgress(path: string | undefined): ReaderProgress | null {
  const trimmed = path?.trim();
  if (!trimmed) return null;
  return loadProgress().get(normalizePath(trimmed)) ?? null;
}

/** Saved line clamped to a document of `total` lines, or null when never read. */
export function readerSavedLine(path: string | undefined, total: number): number | null {
  const saved = readerSavedProgress(path);
  if (!saved || total <= 0) return null;
  if (saved.line <= 0 && saved.ratio <= 0) return null;
  // Same length: trust the line. File grew/shrank: scale by reading fraction.
  const line = saved.total === total ? saved.line : Math.round(saved.ratio * (total - 1));
  return Math.min(Math.max(0, line), total - 1);
}

/** True when this run already holds an exact pixel position for the view. */
export function hasReaderViewport(
  path: string | undefined,
  slot: ReaderViewportSlot = "source",
): boolean {
  const trimmed = path?.trim();
  return !!trimmed && scrollTopByKey.has(storageKey(trimmed, slot));
}
