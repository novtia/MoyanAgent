/** Clipboard MIME for multi-line selections copied from the document reader. */
export const READER_SELECTION_MIME = "application/x-moyan-reader-selection";

export interface ReaderSelectionPayload {
  path: string;
  /** 1-based inclusive paragraph (line) start. */
  paragraphFrom: number;
  /** 1-based inclusive paragraph (line) end; must be > paragraphFrom for ranged paste. */
  paragraphTo: number;
  /** Selected plain text (what external paste / Win+V should see). */
  text: string;
  /** Project root the file belongs to; paste into another project is rejected. */
  projectRoot?: string | null;
  /** Session the copy came from (informational). */
  sessionId?: string | null;
}

function normalizeRootForCompare(p: string): string {
  let t = p.trim();
  if (/^\\\\\?\\UNC\\/i.test(t)) t = `\\\\${t.slice(8)}`;
  else if (/^\\\\\?\\/.test(t)) t = t.slice(4);
  return t.replace(/\\/g, "/").replace(/\/+$/, "").toLowerCase();
}

/**
 * True when the payload's file belongs to `currentRoot`. Without a current
 * project root nothing matches, so the paste falls back to plain text.
 */
export function isReaderSelectionForProject(
  payload: ReaderSelectionPayload,
  currentRoot: string | null | undefined,
): boolean {
  if (!currentRoot?.trim()) return false;
  const current = normalizeRootForCompare(currentRoot);
  if (payload.projectRoot?.trim()) {
    return normalizeRootForCompare(payload.projectRoot) === current;
  }
  const path = normalizeRootForCompare(payload.path);
  return path === current || path.startsWith(`${current}/`);
}

/** True if `path` lies inside `root` (case-insensitive, slash-agnostic). */
export function isPathInProject(
  path: string,
  root: string | null | undefined,
): boolean {
  if (!root?.trim()) return false;
  const current = normalizeRootForCompare(root);
  const p = normalizeRootForCompare(path);
  return p === current || p.startsWith(`${current}/`);
}

/**
 * In-app side channel: host clipboard rewrites often strip custom MIME, so the
 * composer matches paste plain-text against the last multi-line reader copy.
 */
let lastReaderSelection: ReaderSelectionPayload | null = null;
/** Set by reader copy; consumed by clipboard history fix so it does not clear us. */
let readerSelectionCopyPending = false;

export function rememberReaderSelection(payload: ReaderSelectionPayload): void {
  lastReaderSelection = payload;
  readerSelectionCopyPending = true;
}

/** Last multi-line reader copy, if any (used to rewrite the clipboard with HTML). */
export function peekRememberedReaderSelection(): ReaderSelectionPayload | null {
  return lastReaderSelection;
}

export function clearRememberedReaderSelection(): void {
  lastReaderSelection = null;
  readerSelectionCopyPending = false;
}

/** True if the current copy event was tagged as a multi-line reader selection. */
export function consumeReaderSelectionCopyFlag(): boolean {
  const pending = readerSelectionCopyPending;
  readerSelectionCopyPending = false;
  return pending;
}

/** If `plainText` matches the last reader multi-line copy, return that payload. */
export function matchRememberedReaderSelection(
  plainText: string,
): ReaderSelectionPayload | null {
  if (!lastReaderSelection || !isMultiLineReaderSelection(lastReaderSelection)) {
    return null;
  }
  const normalize = (s: string) => s.replace(/\r\n/g, "\n");
  if (normalize(plainText) !== normalize(lastReaderSelection.text)) return null;
  return lastReaderSelection;
}

export function encodeReaderSelectionHtml(payload: ReaderSelectionPayload): string {
  const meta = encodeURIComponent(JSON.stringify(payload));
  const escaped = payload.text
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;");
  return `<!--moyan-reader-selection:${meta}--><pre>${escaped}</pre>`;
}

export function parseReaderSelectionHtml(html: string): ReaderSelectionPayload | null {
  const m = html.match(/<!--moyan-reader-selection:([^>]+)-->/);
  if (!m) return null;
  try {
    const payload = JSON.parse(decodeURIComponent(m[1])) as ReaderSelectionPayload;
    if (
      typeof payload.path === "string" &&
      payload.path &&
      Number.isFinite(payload.paragraphFrom) &&
      Number.isFinite(payload.paragraphTo)
    ) {
      return payload;
    }
  } catch {
    // ignore
  }
  return null;
}

export function isMultiLineReaderSelection(
  payload: ReaderSelectionPayload,
): boolean {
  return (
    payload.paragraphFrom >= 1 &&
    payload.paragraphTo > payload.paragraphFrom
  );
}
