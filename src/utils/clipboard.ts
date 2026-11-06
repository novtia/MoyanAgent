import { invoke } from "@tauri-apps/api/core";
import { writeImage } from "@tauri-apps/plugin-clipboard-manager";
import {
  clearRememberedReaderSelection,
  consumeReaderSelectionCopyFlag,
  encodeReaderSelectionHtml,
  peekRememberedReaderSelection,
} from "./readerSelection";

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

/**
 * Write plain text plus an HTML flavor in one clipboard transaction, so other
 * apps / Win+V get the text while in-app paste can read structured metadata.
 */
export async function copyRich(text: string, html: string): Promise<void> {
  if (!text) return;
  await invoke("clipboard_write_rich", { text, html });
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
 *
 * If a more specific handler already called `preventDefault` (e.g. reader
 * multi-line copy with custom MIME), only rewrite plain text for Win+V —
 * do not clear the in-app reader-selection memory used by composer paste.
 */
export function installClipboardHistoryFix(): void {
  if (typeof document === "undefined") return;
  document.addEventListener("copy", (e) => {
    // After target handlers: keep reader→@ memory only for multi-line reader copies.
    const isReaderCopy = consumeReaderSelectionCopyFlag();
    // Snapshot before a non-reader copy clears the memory.
    const readerPayload = isReaderCopy ? peekRememberedReaderSelection() : null;
    if (!isReaderCopy) clearRememberedReaderSelection();

    if (e.defaultPrevented) {
      const text =
        e.clipboardData?.getData("text/plain") ||
        window.getSelection()?.toString() ||
        "";
      if (!text) return;
      // Reader copy: keep text + HTML (with project-scoped metadata) together.
      scheduleClipboardHistoryRewrite(() =>
        readerPayload
          ? copyRich(text, encodeReaderSelectionHtml(readerPayload))
          : copyText(text),
      );
      return;
    }
    const text = window.getSelection()?.toString();
    if (!text) return;
    e.preventDefault();
    // Keep event clipboard filled for immediate paste. The host-HWND rewrite
    // waits until this event has committed that write; doing it here races
    // WebView2 and Win+V, which still hold the clipboard.
    e.clipboardData?.setData("text/plain", text);
    scheduleClipboardHistoryRewrite(() => copyText(text));
  });
}

/** Latest copy wins so an older rewrite cannot clobber a newer selection. */
let historyRewriteSeq = 0;
/** One native write at a time so two copies don't fight over OpenClipboard. */
let historyRewriteChain: Promise<void> = Promise.resolve();

function scheduleClipboardHistoryRewrite(write: () => Promise<unknown>): void {
  const seq = ++historyRewriteSeq;
  window.setTimeout(() => {
    historyRewriteChain = historyRewriteChain
      .catch(() => undefined)
      .then(async () => {
        if (seq !== historyRewriteSeq) return;
        try {
          await write();
        } catch (err) {
          if (seq !== historyRewriteSeq) return;
          console.warn("[clipboard] history fix failed", err);
        }
      });
  }, 32);
}
