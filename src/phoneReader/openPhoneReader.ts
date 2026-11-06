import { WebviewWindow } from "@tauri-apps/api/webviewWindow";
import { phoneWindowSize } from "./prefs";
import { PHONE_READER_LABEL } from "./sync";

let closeBound = false;
let onCloseRef: (() => void) | null = null;

function bindClose(win: WebviewWindow) {
  if (closeBound) return;
  closeBound = true;
  void win.once("tauri://destroyed", () => {
    closeBound = false;
    onCloseRef?.();
  });
}

/** Open the frameless phone window, or focus it when it is already open. */
export async function openPhoneReader(onClose: () => void): Promise<void> {
  onCloseRef = onClose;
  const existing = await WebviewWindow.getByLabel(PHONE_READER_LABEL);
  if (existing) {
    bindClose(existing);
    await existing.setSize(phoneWindowSize());
    await existing.show();
    await existing.setFocus();
    return;
  }

  const url = new URL(window.location.href);
  url.hash = "#/phone-reader";
  url.search = "";

  const size = phoneWindowSize();
  const win = new WebviewWindow(PHONE_READER_LABEL, {
    url: url.href,
    title: "手机阅读",
    width: size.width,
    height: size.height,
    resizable: false,
    decorations: false,
    transparent: true,
    shadow: false,
    center: true,
    focus: true,
  });

  bindClose(win);
  await new Promise<void>((resolve, reject) => {
    void win.once("tauri://created", () => resolve());
    void win.once("tauri://error", (event) => {
      reject(new Error(String(event.payload ?? "phone-reader")));
    });
  });
}

export async function phoneReaderIsOpen(): Promise<boolean> {
  const existing = await WebviewWindow.getByLabel(PHONE_READER_LABEL);
  return existing != null;
}
