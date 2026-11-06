import { LogicalSize } from "@tauri-apps/api/dpi";

export const PHONE_PREFS_KEY = "lumen.phoneReader.prefs.v1";

/** Portrait window: phone bezel plus the right-side hover dock. */
export const PHONE_WINDOW_PORTRAIT = new LogicalSize(476, 900);
/** Landscape window: wide phone plus the same dock. */
export const PHONE_WINDOW_LANDSCAPE = new LogicalSize(900, 520);

export type PhoneTheme = "parchment" | "white" | "dark" | "warm" | "green" | "pink";
export type PhoneFont = "system" | "serif" | "sans" | "kai";

export type PhonePrefs = {
  theme: PhoneTheme;
  /** Theme to restore when night mode turns off. */
  dayTheme: PhoneTheme;
  nightMode: boolean;
  fontSize: number;
  brightness: number;
  followSystem: boolean;
  eyeCare: boolean;
  autoRead: boolean;
  font: PhoneFont;
  landscape: boolean;
  /** null follows the document payload. */
  indentOverride: boolean | null;
  spacingOverride: boolean | null;
};

export const DEFAULT_PHONE_PREFS: PhonePrefs = {
  theme: "parchment",
  dayTheme: "parchment",
  nightMode: false,
  fontSize: 18,
  brightness: 100,
  followSystem: false,
  eyeCare: false,
  autoRead: false,
  font: "system",
  landscape: false,
  indentOverride: null,
  spacingOverride: null,
};

const THEMES = new Set<PhoneTheme>(["parchment", "white", "dark", "warm", "green", "pink"]);
const FONTS = new Set<PhoneFont>(["system", "serif", "sans", "kai"]);
const DAY_THEMES = new Set<PhoneTheme>(["parchment", "white", "warm", "green", "pink"]);

function clamp(n: number, min: number, max: number, fallback: number): number {
  if (!Number.isFinite(n)) return fallback;
  return Math.min(max, Math.max(min, n));
}

export function loadPhonePrefs(): PhonePrefs {
  try {
    const raw = localStorage.getItem(PHONE_PREFS_KEY);
    if (!raw) return { ...DEFAULT_PHONE_PREFS };
    const parsed = JSON.parse(raw) as Partial<PhonePrefs>;
    const theme = THEMES.has(parsed.theme as PhoneTheme)
      ? (parsed.theme as PhoneTheme)
      : DEFAULT_PHONE_PREFS.theme;
    const dayTheme = DAY_THEMES.has(parsed.dayTheme as PhoneTheme)
      ? (parsed.dayTheme as PhoneTheme)
      : theme === "dark"
        ? "parchment"
        : theme;
    const nightMode = Boolean(parsed.nightMode) || theme === "dark";
    return {
      theme: nightMode ? "dark" : theme,
      dayTheme,
      nightMode,
      fontSize: clamp(Number(parsed.fontSize), 12, 32, 18),
      brightness: clamp(Number(parsed.brightness), 25, 100, 100),
      followSystem: Boolean(parsed.followSystem),
      eyeCare: Boolean(parsed.eyeCare),
      autoRead: Boolean(parsed.autoRead),
      font: FONTS.has(parsed.font as PhoneFont) ? (parsed.font as PhoneFont) : "system",
      landscape: Boolean(parsed.landscape),
      indentOverride: typeof parsed.indentOverride === "boolean" ? parsed.indentOverride : null,
      spacingOverride: typeof parsed.spacingOverride === "boolean" ? parsed.spacingOverride : null,
    };
  } catch {
    return { ...DEFAULT_PHONE_PREFS };
  }
}

export function savePhonePrefs(prefs: PhonePrefs): void {
  try {
    localStorage.setItem(PHONE_PREFS_KEY, JSON.stringify(prefs));
  } catch {
    /* private mode / quota */
  }
}

export function phoneWindowSize(): LogicalSize {
  return loadPhonePrefs().landscape ? PHONE_WINDOW_LANDSCAPE : PHONE_WINDOW_PORTRAIT;
}
