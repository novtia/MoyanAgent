import { create } from "zustand";

export const APPEARANCE_STORAGE_KEY = "atelier.appearance";

export type ChatWidthOption = "narrow" | "default" | "wide" | "full";
export type DensityOption = "compact" | "comfortable" | "spacious";
export type RadiusOption = "sharp" | "default" | "rounded";
export type UiFontOption = "default" | "sans" | "serif" | "mono";

export interface AppearanceSettings {
  /** Preset id ("default" | "blue" | …) or a custom "#rrggbb". */
  accent: string;
  uiFont: UiFontOption;
  chatWidth: ChatWidthOption;
  density: DensityOption;
  radius: RadiusOption;
}

export interface AccentPreset {
  id: string;
  /** Hex used for swatch preview; omitted for "default". */
  color?: string;
}

export const ACCENT_PRESETS: AccentPreset[] = [
  { id: "default" },
  { id: "blue", color: "#2563eb" },
  { id: "teal", color: "#0d9488" },
  { id: "amber", color: "#d97706" },
  { id: "rose", color: "#e11d48" },
  { id: "violet", color: "#7c3aed" },
];

export const UI_FONT_PRESETS: {
  id: UiFontOption;
  value: string;
}[] = [
  {
    id: "default",
    value:
      "'Instrument Sans', 'Noto Sans SC', 'PingFang SC', 'Microsoft YaHei', -apple-system, BlinkMacSystemFont, sans-serif",
  },
  {
    id: "sans",
    value:
      "'Instrument Sans', system-ui, -apple-system, 'Segoe UI', sans-serif",
  },
  {
    id: "serif",
    value:
      "'Instrument Serif', Georgia, 'Songti SC', 'SimSun', 'Times New Roman', serif",
  },
  {
    id: "mono",
    value: "'JetBrains Mono', 'Cascadia Code', Consolas, monospace",
  },
];

export const CHAT_WIDTH_VALUES: Record<ChatWidthOption, string> = {
  narrow: "640px",
  default: "760px",
  wide: "960px",
  full: "min(100%, 1200px)",
};

export const RADIUS_VALUES: Record<
  RadiusOption,
  { radius: string; sm: string; lg: string; xs: string; md: string }
> = {
  sharp: { radius: "4px", sm: "2px", lg: "8px", xs: "2px", md: "3px" },
  default: { radius: "14px", sm: "8px", lg: "22px", xs: "6px", md: "10px" },
  rounded: { radius: "20px", sm: "12px", lg: "28px", xs: "8px", md: "14px" },
};

export const SIDEBAR_WIDTH_BY_DENSITY: Record<DensityOption, string> = {
  compact: "200px",
  comfortable: "220px",
  spacious: "240px",
};

export const DEFAULT_APPEARANCE: AppearanceSettings = {
  accent: "default",
  uiFont: "default",
  chatWidth: "default",
  density: "comfortable",
  radius: "default",
};

const ACCENT_VARS = [
  "--accent",
  "--accent-soft",
  "--blue-50",
  "--blue-100",
  "--blue-500",
  "--blue-600",
  "--blue-700",
] as const;

const HEX_RE = /^#([0-9a-fA-F]{6})$/;

function clampByte(n: number): number {
  return Math.min(255, Math.max(0, Math.round(n)));
}

function hexToRgb(hex: string): { r: number; g: number; b: number } | null {
  const m = HEX_RE.exec(hex.trim());
  if (!m) return null;
  const n = parseInt(m[1], 16);
  return { r: (n >> 16) & 255, g: (n >> 8) & 255, b: n & 255 };
}

function rgbToHex(r: number, g: number, b: number): string {
  return `#${[r, g, b]
    .map((c) => clampByte(c).toString(16).padStart(2, "0"))
    .join("")}`;
}

function mix(
  a: { r: number; g: number; b: number },
  b: { r: number; g: number; b: number },
  t: number,
): { r: number; g: number; b: number } {
  return {
    r: a.r + (b.r - a.r) * t,
    g: a.g + (b.g - a.g) * t,
    b: a.b + (b.b - a.b) * t,
  };
}

function resolveAccentHex(accent: string): string | null {
  if (accent === "default") return null;
  const preset = ACCENT_PRESETS.find((p) => p.id === accent);
  if (preset?.color) return preset.color;
  if (HEX_RE.test(accent.trim())) return accent.trim().toLowerCase();
  return null;
}

function deriveAccentPalette(hex: string): Record<(typeof ACCENT_VARS)[number], string> {
  const rgb = hexToRgb(hex)!;
  const white = { r: 255, g: 255, b: 255 };
  const black = { r: 0, g: 0, b: 0 };
  const darkBg = { r: 16, g: 17, b: 18 };
  const isDark = document.documentElement.dataset.theme === "dark";

  if (isDark) {
    const soft = mix(rgb, white, 0.28);
    const blue50 = mix(rgb, darkBg, 0.78);
    const blue100 = mix(rgb, darkBg, 0.62);
    const blue600 = mix(rgb, white, 0.18);
    const blue700 = mix(rgb, white, 0.32);
    return {
      "--accent": hex,
      "--accent-soft": rgbToHex(soft.r, soft.g, soft.b),
      "--blue-50": rgbToHex(blue50.r, blue50.g, blue50.b),
      "--blue-100": rgbToHex(blue100.r, blue100.g, blue100.b),
      "--blue-500": hex,
      "--blue-600": rgbToHex(blue600.r, blue600.g, blue600.b),
      "--blue-700": rgbToHex(blue700.r, blue700.g, blue700.b),
    };
  }

  const soft = mix(rgb, white, 0.22);
  const blue50 = mix(rgb, white, 0.88);
  const blue100 = mix(rgb, white, 0.78);
  const blue600 = mix(rgb, black, 0.12);
  const blue700 = mix(rgb, black, 0.28);
  return {
    "--accent": hex,
    "--accent-soft": rgbToHex(soft.r, soft.g, soft.b),
    "--blue-50": rgbToHex(blue50.r, blue50.g, blue50.b),
    "--blue-100": rgbToHex(blue100.r, blue100.g, blue100.b),
    "--blue-500": hex,
    "--blue-600": rgbToHex(blue600.r, blue600.g, blue600.b),
    "--blue-700": rgbToHex(blue700.r, blue700.g, blue700.b),
  };
}

function isChatWidth(v: unknown): v is ChatWidthOption {
  return v === "narrow" || v === "default" || v === "wide" || v === "full";
}

function isDensity(v: unknown): v is DensityOption {
  return v === "compact" || v === "comfortable" || v === "spacious";
}

function isRadius(v: unknown): v is RadiusOption {
  return v === "sharp" || v === "default" || v === "rounded";
}

function isUiFont(v: unknown): v is UiFontOption {
  return v === "default" || v === "sans" || v === "serif" || v === "mono";
}

function normalize(settings: Partial<AppearanceSettings>): AppearanceSettings {
  const accent =
    typeof settings.accent === "string" && settings.accent
      ? settings.accent
      : DEFAULT_APPEARANCE.accent;
  return {
    accent:
      accent === "default" ||
      ACCENT_PRESETS.some((p) => p.id === accent) ||
      HEX_RE.test(accent.trim())
        ? accent === "default"
          ? "default"
          : ACCENT_PRESETS.some((p) => p.id === accent)
            ? accent
            : accent.trim().toLowerCase()
        : DEFAULT_APPEARANCE.accent,
    uiFont: isUiFont(settings.uiFont)
      ? settings.uiFont
      : DEFAULT_APPEARANCE.uiFont,
    chatWidth: isChatWidth(settings.chatWidth)
      ? settings.chatWidth
      : DEFAULT_APPEARANCE.chatWidth,
    density: isDensity(settings.density)
      ? settings.density
      : DEFAULT_APPEARANCE.density,
    radius: isRadius(settings.radius)
      ? settings.radius
      : DEFAULT_APPEARANCE.radius,
  };
}

function readStored(): AppearanceSettings {
  try {
    const raw = window.localStorage.getItem(APPEARANCE_STORAGE_KEY);
    if (!raw) return { ...DEFAULT_APPEARANCE };
    const parsed = JSON.parse(raw) as Partial<AppearanceSettings>;
    return normalize(parsed);
  } catch {
    return { ...DEFAULT_APPEARANCE };
  }
}

/** Resolve the effective accent hex for UI swatches (null = theme default). */
export function getAccentSwatchColor(accent: string): string | null {
  return resolveAccentHex(accent);
}

export function applyAppearance(settings: AppearanceSettings) {
  const root = document.documentElement;
  const font =
    UI_FONT_PRESETS.find((p) => p.id === settings.uiFont)?.value ??
    UI_FONT_PRESETS[0].value;

  root.style.setProperty("--app-font-family", font);
  root.style.setProperty(
    "--chat-content-width",
    CHAT_WIDTH_VALUES[settings.chatWidth],
  );
  root.style.setProperty(
    "--sidebar-width",
    SIDEBAR_WIDTH_BY_DENSITY[settings.density],
  );

  const radii = RADIUS_VALUES[settings.radius];
  root.style.setProperty("--radius", radii.radius);
  root.style.setProperty("--radius-sm", radii.sm);
  root.style.setProperty("--radius-lg", radii.lg);
  root.style.setProperty("--radius-xs", radii.xs);
  root.style.setProperty("--radius-md", radii.md);

  root.dataset.density = settings.density;
  root.dataset.radius = settings.radius;

  const accentHex = resolveAccentHex(settings.accent);
  if (!accentHex) {
    for (const key of ACCENT_VARS) {
      root.style.removeProperty(key);
    }
  } else {
    const palette = deriveAccentPalette(accentHex);
    for (const key of ACCENT_VARS) {
      root.style.setProperty(key, palette[key]);
    }
  }
}

interface AppearanceStore extends AppearanceSettings {
  set: (patch: Partial<AppearanceSettings>) => void;
  reset: () => void;
}

const initial = readStored();
applyAppearance(initial);

export const useAppearance = create<AppearanceStore>((set, get) => ({
  ...initial,
  set: (patch) => {
    const next = normalize({ ...get(), ...patch });
    set(next);
    applyAppearance(next);
    try {
      window.localStorage.setItem(APPEARANCE_STORAGE_KEY, JSON.stringify(next));
    } catch {
      // ignore persistence failures
    }
  },
  reset: () => {
    set({ ...DEFAULT_APPEARANCE });
    applyAppearance(DEFAULT_APPEARANCE);
    try {
      window.localStorage.removeItem(APPEARANCE_STORAGE_KEY);
    } catch {
      // ignore
    }
  },
}));
