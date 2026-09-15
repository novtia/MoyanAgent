import { pruneReaderCaches } from "../../../../store/reader";
import { LEGACY_TABS_KEY, TABS_KEY_PREFIX, TABS_SCHEMA_VERSION } from "../constants";
import type { PanelTab, TabKind, TabViewState } from "../types";

export function newTabId() {
  return `tab-${Date.now()}-${Math.random().toString(36).slice(2, 7)}`;
}

export interface StoredTabs {
  tabs: PanelTab[];
  activeId: string | null;
}

const EMPTY: StoredTabs = { tabs: [], activeId: null };

const TAB_KINDS: TabKind[] = ["empty", "gallery", "agent-flow", "role-state", "reader"];

function isTabKind(value: unknown): value is TabKind {
  return typeof value === "string" && TAB_KINDS.includes(value as TabKind);
}

function num(value: unknown, min: number, max: number): number | undefined {
  if (typeof value !== "number" || !Number.isFinite(value)) return undefined;
  return Math.min(max, Math.max(min, value));
}

function parseView(raw: unknown): TabViewState | undefined {
  if (!raw || typeof raw !== "object") return undefined;
  const v = raw as Record<string, unknown>;
  const view: TabViewState = {};
  const ratio = num(v.ratio, 0.1, 0.95);
  if (ratio != null) view.ratio = ratio;
  if (typeof v.showTree === "boolean") view.showTree = v.showTree;
  if (v.rightView === "tree" || v.rightView === "search") view.rightView = v.rightView;
  if (typeof v.preview === "boolean") view.preview = v.preview;

  const find = v.find && typeof v.find === "object" ? (v.find as Record<string, unknown>) : null;
  if (find) {
    view.find = {
      open: find.open === true,
      query: typeof find.query === "string" ? find.query : "",
      replaceWith: typeof find.replaceWith === "string" ? find.replaceWith : "",
      matchCase: find.matchCase === true,
      scope: find.scope === "all" ? "all" : "file",
    };
  }

  const hist =
    v.history && typeof v.history === "object" ? (v.history as Record<string, unknown>) : null;
  if (hist && Array.isArray(hist.stack)) {
    const stack = hist.stack.filter((p): p is string => typeof p === "string" && !!p);
    const index = num(hist.index, -1, stack.length - 1);
    view.history = {
      stack,
      index: index != null ? Math.trunc(index) : stack.length - 1,
    };
  }

  return Object.keys(view).length > 0 ? view : undefined;
}

/**
 * Parse a stored payload for `sessionId`.
 *
 * A payload that names a *different* session is dropped outright: tabs may
 * never bleed across sessions, even if a storage key was copied or a stale
 * write landed under the wrong key. Version-1 payloads carry no session id;
 * the key itself is per-session, so those are accepted and rewritten as v2.
 */
function parseTabs(raw: string | null, sessionId: string): StoredTabs {
  if (!raw) return EMPTY;
  try {
    const parsed = JSON.parse(raw) as {
      v?: number;
      sessionId?: string;
      tabs?: unknown;
      activeId?: unknown;
    };
    if (typeof parsed.sessionId === "string" && parsed.sessionId !== sessionId) {
      return EMPTY;
    }
    const tabs: PanelTab[] = Array.isArray(parsed.tabs)
      ? (parsed.tabs as unknown[])
          .filter(
            (tb): tb is Record<string, unknown> =>
              !!tb &&
              typeof tb === "object" &&
              typeof (tb as Record<string, unknown>).id === "string" &&
              isTabKind((tb as Record<string, unknown>).kind),
          )
          .map((tb) => {
            const view = parseView(tb.view);
            const tab: PanelTab = {
              id: tb.id as string,
              kind: tb.kind as TabKind,
              path: typeof tb.path === "string" ? tb.path : null,
            };
            if (view) tab.view = view;
            return tab;
          })
      : [];
    const activeId =
      typeof parsed.activeId === "string" && tabs.some((tb) => tb.id === parsed.activeId)
        ? parsed.activeId
        : (tabs[0]?.id ?? null);
    return { tabs, activeId };
  } catch {
    return EMPTY;
  }
}

function sessionKey(sessionId: string) {
  return `${TABS_KEY_PREFIX}${sessionId}`;
}

/** Per-session chrome tabs. Empty when the session has never stored any. */
export function readStoredTabs(sessionId: string | null): StoredTabs {
  if (!sessionId || typeof window === "undefined") return EMPTY;
  try {
    return parseTabs(window.localStorage.getItem(sessionKey(sessionId)), sessionId);
  } catch {
    return EMPTY;
  }
}

function write(key: string, value: string): boolean {
  try {
    window.localStorage.setItem(key, value);
    return true;
  } catch {
    return false;
  }
}

/**
 * Back the session's tabs up to storage. Returns false when storage refused
 * the write; callers keep their in-memory copy authoritative either way.
 *
 * A full origin (quota is shared, ~5 MB) makes every write throw. Rather than
 * losing the tabs, free the rebuildable document caches and try once more.
 */
export function persistTabs(
  sessionId: string | null,
  tabs: PanelTab[],
  activeId: string | null,
): boolean {
  if (!sessionId || typeof window === "undefined") return false;
  const key = sessionKey(sessionId);
  const payload = JSON.stringify({
    v: TABS_SCHEMA_VERSION,
    sessionId,
    tabs,
    activeId,
  });
  if (write(key, payload)) return true;
  if (pruneReaderCaches() === 0) return false;
  return write(key, payload);
}

export function clearStoredTabs(sessionId: string | null) {
  if (!sessionId || typeof window === "undefined") return;
  try {
    window.localStorage.removeItem(sessionKey(sessionId));
  } catch {
    /* ignore */
  }
}

/** Drop the pre-per-session global tab blob so no session can inherit it. */
export function purgeLegacyTabStorage() {
  if (typeof window === "undefined") return;
  try {
    window.localStorage.removeItem(LEGACY_TABS_KEY);
  } catch {
    /* ignore */
  }
}

export function pickActiveId(tabs: PanelTab[], preferred: string | null): string | null {
  if (preferred && tabs.some((tb) => tb.id === preferred)) return preferred;
  return tabs[0]?.id ?? null;
}
