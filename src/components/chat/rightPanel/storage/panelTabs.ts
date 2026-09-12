import { TABS_KEY, TABS_KEY_PREFIX } from "../constants";
import type { PanelTab } from "../types";

export function newTabId() {
  return `tab-${Date.now()}-${Math.random().toString(36).slice(2, 7)}`;
}

export interface StoredTabs {
  tabs: PanelTab[];
  activeId: string | null;
  /** True when a storage key was present (even if `tabs` is empty). */
  found: boolean;
}

const EMPTY: StoredTabs = { tabs: [], activeId: null, found: false };

function parseTabs(raw: string | null): StoredTabs {
  if (!raw) return EMPTY;
  try {
    const parsed = JSON.parse(raw) as { tabs?: PanelTab[]; activeId?: string | null };
    const tabs = Array.isArray(parsed.tabs)
      ? parsed.tabs
          .filter(
            (tb): tb is PanelTab =>
              !!tb &&
              typeof tb.id === "string" &&
              (tb.kind === "empty" ||
                tb.kind === "gallery" ||
                tb.kind === "agent-flow" ||
                tb.kind === "role-state" ||
                tb.kind === "reader"),
          )
          .map((tb) => ({
            id: tb.id,
            kind: tb.kind,
            path: typeof tb.path === "string" ? tb.path : null,
          }))
      : [];
    const activeId = tabs.some((tb) => tb.id === parsed.activeId)
      ? (parsed.activeId as string)
      : (tabs[0]?.id ?? null);
    return { tabs, activeId, found: true };
  } catch {
    return { tabs: [], activeId: null, found: true };
  }
}

function readKey(key: string): StoredTabs {
  if (typeof window === "undefined") return EMPTY;
  try {
    return parseTabs(window.localStorage.getItem(key));
  } catch {
    return EMPTY;
  }
}

function sessionKey(sessionId: string) {
  return `${TABS_KEY_PREFIX}${sessionId}`;
}

function readGlobalTabs(): StoredTabs {
  return readKey(TABS_KEY);
}

function clearGlobalTabs() {
  if (typeof window === "undefined") return;
  try {
    window.localStorage.removeItem(TABS_KEY);
  } catch {
    /* ignore */
  }
}

/** Per-session chrome tabs. Empty when the session has never stored any. */
export function readStoredTabs(sessionId: string | null): StoredTabs {
  if (!sessionId) return EMPTY;

  // One-time lift from the brief global-key era: the first session that binds
  // absorbs the blob (that session is the one the user last had open), then
  // the global key is deleted so later sessions cannot inherit A/B/C.
  const global = readGlobalTabs();
  if (global.found) {
    persistTabs(sessionId, global.tabs, global.activeId);
    clearGlobalTabs();
    return { ...global, found: true };
  }

  return readKey(sessionKey(sessionId));
}

export function persistTabs(
  sessionId: string | null,
  tabs: PanelTab[],
  activeId: string | null,
) {
  if (!sessionId || typeof window === "undefined") return;
  try {
    window.localStorage.setItem(sessionKey(sessionId), JSON.stringify({ tabs, activeId }));
  } catch {
    /* ignore */
  }
}

export function clearStoredTabs(sessionId: string | null) {
  if (!sessionId || typeof window === "undefined") return;
  try {
    window.localStorage.removeItem(sessionKey(sessionId));
  } catch {
    /* ignore */
  }
}

export function pickActiveId(tabs: PanelTab[], preferred: string | null): string | null {
  if (preferred && tabs.some((tb) => tb.id === preferred)) return preferred;
  return tabs[0]?.id ?? null;
}
