import { create } from "zustand";

export const CHAT_FONT_STORAGE_KEY = "atelier.chatFont";

export interface ChatFontSettings {
  /** CSS font-family stack, or "inherit" to follow the app default. */
  fontFamily: string;
  /** Base font size in px. */
  fontSize: number;
  /** Unitless line-height multiplier. */
  lineHeight: number;
  /** Hex color string, or "default" to follow the theme ink color. */
  color: string;
}

export interface ChatFontFamilyPreset {
  /** Stable id used as the <option> value and for i18n lookup. */
  id: string;
  /** i18n key suffix under the `chat` namespace. */
  labelKey: string;
  /** CSS font-family value. */
  value: string;
}

/**
 * Font family presets offered in the panel. `inherit` keeps the current app
 * default so an untouched install renders exactly as before.
 */
export const CHAT_FONT_FAMILY_PRESETS: ChatFontFamilyPreset[] = [
  { id: "system", labelKey: "fontFamilySystem", value: "inherit" },
  {
    id: "sans",
    labelKey: "fontFamilySans",
    value:
      "'Instrument Sans', system-ui, -apple-system, 'Segoe UI', sans-serif",
  },
  {
    id: "serif",
    labelKey: "fontFamilySerif",
    value: "Georgia, 'Songti SC', 'SimSun', 'Times New Roman', serif",
  },
  {
    id: "mono",
    labelKey: "fontFamilyMono",
    value: "'JetBrains Mono', 'Cascadia Code', Consolas, monospace",
  },
  {
    id: "round",
    labelKey: "fontFamilyRound",
    value:
      "'Quicksand', 'Yuanti SC', 'Microsoft YaHei', 'PingFang SC', sans-serif",
  },
];

export const DEFAULT_CHAT_FONT: ChatFontSettings = {
  fontFamily: "inherit",
  fontSize: 14,
  lineHeight: 1.6,
  color: "default",
};

export const CHAT_FONT_SIZE_RANGE = { min: 12, max: 22 } as const;
export const CHAT_LINE_HEIGHT_RANGE = { min: 1.2, max: 2.2 } as const;

function clamp(value: number, min: number, max: number): number {
  if (Number.isNaN(value)) return min;
  return Math.min(max, Math.max(min, value));
}

function normalize(settings: Partial<ChatFontSettings>): ChatFontSettings {
  return {
    fontFamily:
      typeof settings.fontFamily === "string" && settings.fontFamily
        ? settings.fontFamily
        : DEFAULT_CHAT_FONT.fontFamily,
    fontSize: clamp(
      typeof settings.fontSize === "number"
        ? settings.fontSize
        : DEFAULT_CHAT_FONT.fontSize,
      CHAT_FONT_SIZE_RANGE.min,
      CHAT_FONT_SIZE_RANGE.max,
    ),
    lineHeight: clamp(
      typeof settings.lineHeight === "number"
        ? settings.lineHeight
        : DEFAULT_CHAT_FONT.lineHeight,
      CHAT_LINE_HEIGHT_RANGE.min,
      CHAT_LINE_HEIGHT_RANGE.max,
    ),
    color:
      typeof settings.color === "string" && settings.color
        ? settings.color
        : DEFAULT_CHAT_FONT.color,
  };
}

function readStored(): ChatFontSettings {
  try {
    const raw = window.localStorage.getItem(CHAT_FONT_STORAGE_KEY);
    if (!raw) return { ...DEFAULT_CHAT_FONT };
    const parsed = JSON.parse(raw) as Partial<ChatFontSettings>;
    return normalize(parsed);
  } catch {
    return { ...DEFAULT_CHAT_FONT };
  }
}

/** Push the settings into CSS variables consumed by chat message styles. */
export function applyChatFont(settings: ChatFontSettings) {
  const root = document.documentElement;
  root.style.setProperty("--chat-font-family", settings.fontFamily);
  root.style.setProperty("--chat-font-size", `${settings.fontSize}px`);
  root.style.setProperty("--chat-line-height", String(settings.lineHeight));
  root.style.setProperty(
    "--chat-font-color",
    settings.color === "default" ? "var(--ink)" : settings.color,
  );
}

interface ChatFontStore extends ChatFontSettings {
  set: (patch: Partial<ChatFontSettings>) => void;
  reset: () => void;
}

const initial = readStored();
applyChatFont(initial);

export const useChatFont = create<ChatFontStore>((set, get) => ({
  ...initial,
  set: (patch) => {
    const next = normalize({ ...get(), ...patch });
    set(next);
    applyChatFont(next);
    try {
      window.localStorage.setItem(CHAT_FONT_STORAGE_KEY, JSON.stringify(next));
    } catch {
      // ignore persistence failures (e.g. private mode)
    }
  },
  reset: () => {
    set({ ...DEFAULT_CHAT_FONT });
    applyChatFont(DEFAULT_CHAT_FONT);
    try {
      window.localStorage.removeItem(CHAT_FONT_STORAGE_KEY);
    } catch {
      // ignore
    }
  },
}));
