export type VideoGenerationMode =
  | "text"
  | "first_frame"
  | "first_last"
  | "reference";

export const VIDEO_MODES: readonly VideoGenerationMode[] = [
  "text",
  "first_frame",
  "first_last",
  "reference",
];

export const VIDEO_RATIOS = [
  "adaptive",
  "16:9",
  "4:3",
  "1:1",
  "3:4",
  "9:16",
  "21:9",
] as const;

export const VIDEO_RESOLUTIONS = ["480p", "720p", "1080p", "4k"] as const;

export const VIDEO_DURATIONS = [4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, -1] as const;
