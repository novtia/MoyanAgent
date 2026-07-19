import { writeImage, writeText } from "@tauri-apps/plugin-clipboard-manager";

const IMAGE_MIME = new Set(["image/png", "image/jpeg", "image/webp"]);

/** Write plain text to the OS clipboard (Win32 / system APIs on desktop). */
export async function copyText(text: string): Promise<void> {
  if (!text) return;
  await writeText(text);
}

/** Copy an on-disk image file to the OS clipboard. */
export async function copyImageFromPath(path: string): Promise<void> {
  if (!path) return;
  await writeImage(path);
}

/** Copy an in-memory image blob to the OS clipboard. */
export async function copyImageFromBlob(blob: Blob): Promise<void> {
  if (!IMAGE_MIME.has(blob.type)) {
    throw new Error(`unsupported image type: ${blob.type || "unknown"}`);
  }
  await writeImage(await blob.arrayBuffer());
}
