import type { NovelAiSettings } from "../../../types";

export const DEFAULT_NOVELAI_SETTINGS: NovelAiSettings = {
  api_key: "",
  model: "nai-diffusion-5-full",
  width: 832,
  height: 1216,
  sampler: "k_euler_ancestral",
  noise_schedule: "karras",
  steps: 28,
  scale: 5,
  cfg_rescale: 0,
  quality: "gallery",
  v5_mode: "anime",
  uc_preset: "heavy",
  uc: "",
  artists: "",
  straight_alpha: true,
};

export const NAI_MODELS = [
  { value: "nai-diffusion-5-full", labelKey: "full" as const },
  { value: "nai-diffusion-5-curated", labelKey: "curated" as const },
];

export const NAI_SAMPLERS = [
  "k_euler_ancestral",
  "k_euler",
  "k_dpmpp_2s_ancestral",
  "k_dpmpp_2m",
  "k_dpmpp_2m_sde",
  "k_dpmpp_sde",
];

export const NAI_SCHEDULES = ["karras", "exponential", "polyexponential"] as const;

export const NAI_SIZE_PRESETS: { width: number; height: number; key: string }[] = [
  { width: 768, height: 512, key: "smallLandscape" },
  { width: 512, height: 768, key: "smallPortrait" },
  { width: 640, height: 640, key: "smallSquare" },
  { width: 1216, height: 832, key: "normalLandscape" },
  { width: 832, height: 1216, key: "normalPortrait" },
  { width: 1024, height: 1024, key: "normalSquare" },
  { width: 1536, height: 1024, key: "largeLandscape" },
  { width: 1024, height: 1536, key: "largePortrait" },
  { width: 1472, height: 1472, key: "largeSquare" },
];

export function sizePresetValue(width: number, height: number): string {
  const hit = NAI_SIZE_PRESETS.find((p) => p.width === width && p.height === height);
  return hit ? `${hit.width}x${hit.height}` : "custom";
}

export function parseSizePreset(value: string): { width: number; height: number } | null {
  if (value === "custom") return null;
  const m = /^(\d+)x(\d+)$/.exec(value);
  if (!m) return null;
  return { width: Number(m[1]), height: Number(m[2]) };
}

export function mergeNovelAi(
  current: NovelAiSettings | undefined,
  patch: Partial<NovelAiSettings>,
): NovelAiSettings {
  return { ...DEFAULT_NOVELAI_SETTINGS, ...current, ...patch };
}
