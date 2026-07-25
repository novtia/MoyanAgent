export const ACCEPT_TYPES = [
  "image/png",
  "image/jpeg",
  "image/webp",
  "image/bmp",
  "image/gif",
  "image/tiff",
  "audio/wav",
  "audio/mpeg",
];
export const MAX_FILES = 15;
export const MAX_IMAGE_BYTES = 30 * 1024 * 1024;
export const MAX_AUDIO_BYTES = 15 * 1024 * 1024;

/** Tools whose input arguments we render live as they stream in. */
export const STREAMING_INPUT_TOOLS = new Set(["CreateDoc", "Edit", "Write"]);

/** File-tree tools that should bump the explorer on completion. */
export const FS_TREE_TOOLS = new Set(["CreateDoc", "Write", "Edit", "Delete", "Bash"]);
