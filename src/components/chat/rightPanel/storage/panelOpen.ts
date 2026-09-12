import { OPEN_KEY } from "../constants";

export function readStoredOpen(): boolean {
  if (typeof window === "undefined") return false;
  try {
    return window.localStorage.getItem(OPEN_KEY) === "1";
  } catch {
    return false;
  }
}

export function persistOpen(open: boolean) {
  if (typeof window === "undefined") return;
  try {
    window.localStorage.setItem(OPEN_KEY, open ? "1" : "0");
  } catch {
    /* ignore */
  }
}
