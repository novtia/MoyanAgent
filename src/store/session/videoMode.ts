import type { VideoGenerationMode } from "../../config/videoGeneration";
import type { ComposerState } from "./types";

export function resolvedMediaRole(
  mode: VideoGenerationMode,
  mime: string,
  index: number,
): string | null {
  if (mime.startsWith("audio/")) return "reference_audio";
  if (mime.startsWith("video/")) return "reference_video";
  if (!mime.startsWith("image/")) return null;
  if (mode === "first_frame") return "first_frame";
  if (mode === "first_last") return index === 0 ? "first_frame" : "last_frame";
  if (mode === "reference") return "reference_image";
  return null;
}

export function validateVideoModeAttachments(
  mode: VideoGenerationMode,
  media: Array<{ mime: string }>,
  prompt: string,
): boolean {
  const images = media.filter((a) => a.mime.startsWith("image/")).length;
  const audio = media.filter((a) => a.mime.startsWith("audio/")).length;
  const videos = media.filter((a) => a.mime.startsWith("video/")).length;
  switch (mode) {
    case "text":
      return !!prompt && images + audio + videos === 0;
    case "first_frame":
      return images === 1 && audio + videos === 0;
    case "first_last":
      return images === 2 && audio + videos === 0;
    case "reference":
      return images <= 9 && audio <= 3 && videos <= 3 && images + videos >= 1;
  }
}

/**
 * If attachments no longer match the recorded mode, pick a compatible
 * mode when the mapping is unambiguous (e.g. all images removed → text).
 */
export function coerceVideoModeForMedia(
  mode: VideoGenerationMode | string | undefined,
  media: Array<{ mime: string }>,
  prompt: string,
): VideoGenerationMode | null {
  const resolved: VideoGenerationMode =
    mode === "first_frame" ||
    mode === "first_last" ||
    mode === "reference" ||
    mode === "text"
      ? mode
      : "text";
  if (validateVideoModeAttachments(resolved, media, prompt)) return resolved;

  const images = media.filter((a) => a.mime.startsWith("image/")).length;
  const audio = media.filter((a) => a.mime.startsWith("audio/")).length;
  const videos = media.filter((a) => a.mime.startsWith("video/")).length;
  const total = images + audio + videos;

  if (total === 0 && prompt) return "text";
  if (images === 1 && audio + videos === 0) return "first_frame";
  if (images === 2 && audio + videos === 0) return "first_last";
  if (
    images <= 9 &&
    audio <= 3 &&
    videos <= 3 &&
    images + videos >= 1
  ) {
    return "reference";
  }
  return null;
}

export function validateVideoComposer(composer: ComposerState, prompt: string): boolean {
  return validateVideoModeAttachments(
    composer.videoMode,
    composer.attachments,
    prompt,
  );
}

/** True when message media matches its recorded video_mode (or non-video messages). */
export function messageMatchesVideoMode(message: {
  text?: string | null;
  params?: { video_mode?: string } | null;
  images: Array<{ role: string; mime: string }>;
}): boolean {
  const mode = message.params?.video_mode;
  if (
    mode !== "text" &&
    mode !== "first_frame" &&
    mode !== "first_last" &&
    mode !== "reference"
  ) {
    return true;
  }
  const inputs = message.images.filter((i) => i.role === "input");
  return (
    coerceVideoModeForMedia(mode, inputs, (message.text || "").trim()) != null
  );
}
