import { MAX_RATIO, MIN_RATIO, RATIO_KEY, SHOW_TREE_KEY } from "../constants";

export function readStoredRatio(): number {
  try {
    const raw = window.localStorage.getItem(RATIO_KEY);
    const n = raw ? Number.parseFloat(raw) : NaN;
    if (!Number.isFinite(n)) return 0.58;
    return Math.min(MAX_RATIO, Math.max(MIN_RATIO, n));
  } catch {
    return 0.58;
  }
}

export function readStoredShowTree(): boolean {
  if (typeof window === "undefined") return true;
  return window.localStorage.getItem(SHOW_TREE_KEY) !== "0";
}

export function persistRatio(ratio: number) {
  try {
    window.localStorage.setItem(RATIO_KEY, String(ratio));
  } catch {
    /* ignore */
  }
}

export function persistShowTreeValue(next: boolean) {
  try {
    window.localStorage.setItem(SHOW_TREE_KEY, next ? "1" : "0");
  } catch {
    /* ignore */
  }
}
