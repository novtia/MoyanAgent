export const MODEL_PRESETS = [
  "openai/gpt-5.4-image-2",
  "google/gemini-3.1-flash-image-preview",
  "google/gemini-2.5-flash-image",
  "black-forest-labs/flux.2-pro",
] as const;

export const ASPECT_RATIOS = [
  "auto",
  "1:1",
  "3:2",
  "2:3",
  "4:3",
  "3:4",
  "5:4",
  "4:5",
  "16:9",
  "9:16",
  "21:9",
] as const;

export const IMAGE_SIZES = ["auto", "1K", "2K", "4K"] as const;

export const RATIO_PIXEL_HINT: Record<string, string> = {
  "1:1": "1024 × 1024",
  "2:3": "832 × 1248",
  "3:2": "1248 × 832",
  "3:4": "864 × 1184",
  "4:3": "1184 × 864",
  "4:5": "896 × 1152",
  "5:4": "1152 × 896",
  "9:16": "768 × 1344",
  "16:9": "1344 × 768",
  "21:9": "1536 × 672",
};

export function shortModelName(model?: string | null) {
  if (!model) return "model";
  const slash = model.lastIndexOf("/");
  return slash >= 0 ? model.slice(slash + 1) : model;
}
