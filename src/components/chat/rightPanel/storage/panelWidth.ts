import {
  CHAT_MAIN_MIN,
  DEFAULT_WIDTH,
  MIN_WIDTH,
  SIDEBAR_FALLBACK,
  WIDTH_KEY,
} from "../constants";

/** Largest panel width that still leaves `CHAT_MAIN_MIN` for the conversation. */
export function panelMaxWidth(rowWidth: number) {
  if (!Number.isFinite(rowWidth) || rowWidth <= 0) return MIN_WIDTH;
  return Math.max(MIN_WIDTH, Math.floor(rowWidth - CHAT_MAIN_MIN));
}

export function clampPanelWidth(width: number, maxWidth: number) {
  return Math.min(maxWidth, Math.max(MIN_WIDTH, width));
}

function readSidebarWidth() {
  if (typeof document === "undefined") return SIDEBAR_FALLBACK;
  const shell = document.querySelector(".app-shell");
  if (shell?.classList.contains("sidebar-collapsed")) return 0;
  const raw = getComputedStyle(document.documentElement)
    .getPropertyValue("--sidebar-width")
    .trim();
  const width = parseFloat(raw);
  return Number.isFinite(width) ? width : SIDEBAR_FALLBACK;
}

/** Estimate before the chat row has been measured. */
export function estimatePanelMaxWidth() {
  if (typeof window === "undefined") return MIN_WIDTH;
  return panelMaxWidth(window.innerWidth - readSidebarWidth());
}

export function readStoredWidth() {
  if (typeof window === "undefined") return DEFAULT_WIDTH;
  const raw = window.localStorage.getItem(WIDTH_KEY);
  if (!raw) return DEFAULT_WIDTH;
  const width = parseInt(raw, 10);
  if (!Number.isFinite(width)) return DEFAULT_WIDTH;
  return Math.max(MIN_WIDTH, width);
}

export function persistWidth(width: number) {
  try {
    window.localStorage.setItem(WIDTH_KEY, String(width));
  } catch {
    /* ignore */
  }
}
