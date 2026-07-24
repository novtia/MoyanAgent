import { DEFAULT_WIDTH, MAX_WIDTH, MIN_WIDTH, WIDTH_KEY } from "../constants";

export function readStoredWidth() {
  if (typeof window === "undefined") return DEFAULT_WIDTH;
  const raw = window.localStorage.getItem(WIDTH_KEY);
  if (!raw) return DEFAULT_WIDTH;
  const v = parseInt(raw, 10);
  if (!Number.isFinite(v)) return DEFAULT_WIDTH;
  return Math.min(MAX_WIDTH, Math.max(MIN_WIDTH, v));
}

export function persistWidth(width: number) {
  try {
    window.localStorage.setItem(WIDTH_KEY, String(width));
  } catch {
    /* ignore */
  }
}
