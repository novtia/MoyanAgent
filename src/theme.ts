export type ThemeMode = "light" | "dark" | "system" | "paper" | "mist" | "ink";
export type ResolvedTheme = "light" | "dark" | "paper" | "mist" | "ink";

export const THEME_STORAGE_KEY = "atelier.theme";
const THEME_MEDIA_QUERY = "(prefers-color-scheme: dark)";

export function isThemeMode(value: string | null): value is ThemeMode {
  return (
    value === "light" ||
    value === "dark" ||
    value === "system" ||
    value === "paper" ||
    value === "mist" ||
    value === "ink"
  );
}

export function readStoredThemeMode(): ThemeMode {
  const stored = window.localStorage.getItem(THEME_STORAGE_KEY);
  return isThemeMode(stored) ? stored : "system";
}

export function resolveThemeMode(mode: ThemeMode): ResolvedTheme {
  if (mode !== "system") return mode;
  return window.matchMedia(THEME_MEDIA_QUERY).matches ? "dark" : "light";
}

export function isDarkLike(resolved: ResolvedTheme): boolean {
  return resolved === "dark" || resolved === "ink";
}

export function applyThemeMode(mode: ThemeMode) {
  const resolved = resolveThemeMode(mode);
  const root = document.documentElement;
  root.dataset.themeMode = mode;
  root.dataset.theme = resolved;
  root.style.colorScheme = isDarkLike(resolved) ? "dark" : "light";
}

export function watchSystemTheme(onChange: () => void) {
  const media = window.matchMedia(THEME_MEDIA_QUERY);
  media.addEventListener("change", onChange);
  return () => media.removeEventListener("change", onChange);
}
