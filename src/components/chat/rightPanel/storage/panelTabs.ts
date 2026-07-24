import { TABS_KEY_PREFIX } from "../constants";
import type { PanelTab } from "../types";

export function newTabId() {
  return `tab-${Date.now()}-${Math.random().toString(36).slice(2, 7)}`;
}

function tabsStorageKey(sessionId: string | null): string | null {
  return sessionId ? `${TABS_KEY_PREFIX}${sessionId}` : null;
}

export function readStoredTabs(
  sessionId: string | null,
): { tabs: PanelTab[]; activeId: string | null } {
  const key = tabsStorageKey(sessionId);
  if (!key || typeof window === "undefined") return { tabs: [], activeId: null };
  try {
    const raw = window.localStorage.getItem(key);
    if (!raw) return { tabs: [], activeId: null };
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
    return { tabs, activeId };
  } catch {
    return { tabs: [], activeId: null };
  }
}

export function persistTabs(
  sessionId: string | null,
  tabs: PanelTab[],
  activeId: string | null,
) {
  const key = tabsStorageKey(sessionId);
  if (!key || typeof window === "undefined") return;
  try {
    window.localStorage.setItem(key, JSON.stringify({ tabs, activeId }));
  } catch {
    /* ignore */
  }
}

export function pickActiveId(tabs: PanelTab[], preferred: string | null): string | null {
  if (preferred && tabs.some((tb) => tb.id === preferred)) return preferred;
  return tabs[0]?.id ?? null;
}
