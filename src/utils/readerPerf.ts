import { normalizeReaderPath } from "../store/reader";

export interface ReaderOpenTrace {
  key: string;
  path: string;
  mode: string;
  startedAt: number;
  cachedTab: boolean;
  /** Backend IPC timings (ms) */
  backendReadMs?: number;
  backendDiffsMs?: number;
  backendChunkLayoutMs?: number;
  /** Frontend JS & render timings (ms) */
  frontendStoreMs?: number;
  frontendRenderMs?: number;
  frontendMdPreviewMs?: number;
  frontendCmInitMs?: number;
  frontendPaintMs?: number;
  textChars?: number;
  textLines?: number;
  pendingDiffsCount?: number;
  chunksCount?: number;
  finished: boolean;
}

const activeTraces = new Map<string, ReaderOpenTrace>();

function now(): number {
  return typeof performance !== "undefined" ? performance.now() : Date.now();
}

export function startReaderOpenTrace(
  path: string,
  opts: { cachedTab: boolean; mode?: string },
): ReaderOpenTrace {
  const key = normalizeReaderPath(path) || path;
  const trace: ReaderOpenTrace = {
    key,
    path,
    mode: opts.mode ?? "unknown",
    startedAt: now(),
    cachedTab: opts.cachedTab,
    finished: false,
  };
  activeTraces.set(key, trace);
  return trace;
}

export function getReaderOpenTrace(path: string): ReaderOpenTrace | undefined {
  const key = normalizeReaderPath(path) || path;
  return activeTraces.get(key);
}

export function markReaderOpenBackend(
  path: string,
  field: "backendReadMs" | "backendDiffsMs" | "backendChunkLayoutMs",
  durationMs: number,
  extra?: Partial<Pick<ReaderOpenTrace, "textChars" | "textLines" | "pendingDiffsCount" | "chunksCount">>,
) {
  const trace = getReaderOpenTrace(path);
  if (!trace) return;
  trace[field] = durationMs;
  if (extra) Object.assign(trace, extra);
}

export function markReaderOpenFrontend(
  path: string,
  field:
    | "frontendStoreMs"
    | "frontendRenderMs"
    | "frontendMdPreviewMs"
    | "frontendCmInitMs"
    | "frontendPaintMs",
  durationMs: number,
  extra?: Partial<Pick<ReaderOpenTrace, "mode" | "textChars" | "textLines" | "pendingDiffsCount" | "chunksCount">>,
) {
  const trace = getReaderOpenTrace(path);
  if (!trace) return;
  trace[field] = durationMs;
  if (extra) Object.assign(trace, extra);
}

export function finishReaderOpenTrace(path: string, modeOverride?: string) {
  const trace = getReaderOpenTrace(path);
  if (!trace || trace.finished) return;
  trace.finished = true;
  if (modeOverride) trace.mode = modeOverride;

  const totalMs = now() - trace.startedAt;
  const backendTotalMs =
    (trace.backendReadMs ?? 0) +
    (trace.backendDiffsMs ?? 0) +
    (trace.backendChunkLayoutMs ?? 0);
  const frontendTotalMs = Math.max(0, totalMs - backendTotalMs);

  const fmt = (v: number | undefined) => (v == null ? "-" : `${v.toFixed(1)}ms`);

  console.info(
    `[ReaderPerf] open "${trace.path}" | mode=${trace.mode} cached=${trace.cachedTab} ` +
      `chars=${trace.textChars ?? "-"} lines=${trace.textLines ?? "-"} ` +
      `diffs=${trace.pendingDiffsCount ?? 0} chunks=${trace.chunksCount ?? "-"} | ` +
      `total=${totalMs.toFixed(1)}ms | ` +
      `backend=${backendTotalMs.toFixed(1)}ms (read=${fmt(trace.backendReadMs)}, diffs=${fmt(trace.backendDiffsMs)}, chunkLayout=${fmt(trace.backendChunkLayoutMs)}) | ` +
      `frontend=${frontendTotalMs.toFixed(1)}ms (store=${fmt(trace.frontendStoreMs)}, render=${fmt(trace.frontendRenderMs)}, mdPreview=${fmt(trace.frontendMdPreviewMs)}, cmInit=${fmt(trace.frontendCmInitMs)}, paint=${fmt(trace.frontendPaintMs)})`,
  );
}
