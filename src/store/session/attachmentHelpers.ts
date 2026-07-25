import {
  ACCEPT_TYPES,
  MAX_AUDIO_BYTES,
  MAX_IMAGE_BYTES,
} from "./constants";
import { activeModelCapabilities } from "./modelCapabilities";
import type { PendingAttachmentDraft } from "./types";

export function makePendingAttachment(label: string, bytes: number | null = null): PendingAttachmentDraft {
  return {
    id: `pending-${Date.now()}-${Math.random().toString(36).slice(2)}`,
    label,
    bytes,
  };
}

export function pathLabel(path: string) {
  return path.split(/[\\/]/).pop() || "image";
}

export function stripMediaMentionTokens(text: string) {
  return text
    .replace(/@(?:"(?:image|音频|视频)\d+"|(?:image|音频|视频)\d+)/g, "")
    .replace(/[ \t]+\n/g, "\n")
    .replace(/[ \t]{2,}/g, " ")
    .trim();
}

export function matchesImageRole(role: string) {
  return role === "input" || role === "output" || role === "edited";
}

export function maxBytesForMime(mime: string) {
  return mime.startsWith("audio/") ? MAX_AUDIO_BYTES : MAX_IMAGE_BYTES;
}

export function uploadMime(file: File) {
  const declared = file.type.toLowerCase();
  if (ACCEPT_TYPES.includes(declared)) return declared;
  const extension = file.name.split(".").pop()?.toLowerCase() ?? "";
  if (extension === "jpg" || extension === "jpeg") return "image/jpeg";
  if (extension === "png") return "image/png";
  if (extension === "webp") return "image/webp";
  if (extension === "bmp") return "image/bmp";
  if (extension === "gif") return "image/gif";
  if (extension === "tif" || extension === "tiff") return "image/tiff";
  if (extension === "wav") return "audio/wav";
  if (extension === "mp3") return "audio/mpeg";
  return declared;
}

export function isVideoModel() {
  return activeModelCapabilities().includes("video");
}

export async function fileToBytes(f: File): Promise<Uint8Array> {
  const buf = await f.arrayBuffer();
  return new Uint8Array(buf);
}
