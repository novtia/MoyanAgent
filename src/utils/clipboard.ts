import { invoke } from "@tauri-apps/api/core";
import { writeImage } from "@tauri-apps/plugin-clipboard-manager";

const IMAGE_MIME = new Set(["image/png", "image/jpeg", "image/webp"]);

/**
 * Write plain text to the OS clipboard.
 * On Windows this goes through a host-window-owned OpenClipboard so Win+V
 * clipboard history records the item (plugin/arboard uses a NULL owner).
 */
export async function copyText(text: string): Promise<void> {
  if (!text) return;
  await invoke("clipboard_write_text", { text });
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

/**
 * Re-own selection copies (Ctrl+C) with the host window HWND so they appear
 * in Windows Clipboard History. WebView2's built-in copy is otherwise ignored.
 */
export function installClipboardHistoryFix(): void {
  if (typeof document === "undefined") return;
  document.addEventListener("copy", (e) => {
    const text = window.getSelection()?.toString();
    if (!text) return;
    e.preventDefault();
    // Keep event clipboard filled for immediate paste; then re-write via host HWND.
    e.clipboardData?.setData("text/plain", text);
    void invoke("clipboard_write_text", { text }).catch((err) => {
      console.warn("[clipboard] history fix failed", err);
    });
  });
}
