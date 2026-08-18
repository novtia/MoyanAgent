import { useEffect, useLayoutEffect, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { api } from "../api/tauri";

const MOBILE_MQ = "(max-width: 900px)";

function mqMatches(): boolean {
  return window.matchMedia(MOBILE_MQ).matches;
}

/** Android/iOS always; otherwise follow a 900px viewport so browser shrink works. */
export function useMobileShell(): boolean {
  const [narrow, setNarrow] = useState(mqMatches);
  const [platform, setPlatform] = useState<string | null>(null);

  useEffect(() => {
    api
      .getAppInfo()
      .then((info) => {
        setPlatform(info.platform);
        if (info.platform) {
          document.documentElement.dataset.platform = info.platform;
        }
      })
      .catch(() => setPlatform(""));
  }, []);

  useEffect(() => {
    const mq = window.matchMedia(MOBILE_MQ);
    const onChange = () => setNarrow(mq.matches);
    mq.addEventListener("change", onChange);
    return () => mq.removeEventListener("change", onChange);
  }, []);

  return platform === "android" || platform === "ios" || narrow;
}

/** True on Android/iOS only — not the 900px desktop mobile shell. */
export function usePePlatform(): boolean {
  const [platform, setPlatform] = useState<string | null>(null);

  useEffect(() => {
    const fromDom = document.documentElement.dataset.platform;
    if (fromDom === "android" || fromDom === "ios") {
      setPlatform(fromDom);
    }
    api
      .getAppInfo()
      .then((info) => {
        setPlatform(info.platform);
        if (info.platform) {
          document.documentElement.dataset.platform = info.platform;
        }
      })
      .catch(() => {
        setPlatform((prev) => prev ?? "");
      });
  }, []);

  return platform === "android" || platform === "ios";
}

export function useSyncMobileShellAttr(isMobile: boolean) {
  useLayoutEffect(() => {
    const root = document.documentElement;
    if (isMobile) root.setAttribute("data-shell", "mobile");
    else root.removeAttribute("data-shell");
    return () => root.removeAttribute("data-shell");
  }, [isMobile]);
}

/** Keep `--app-height` / `--ime-bottom` in sync with the visual viewport. */
export function useAppHeight() {
  useLayoutEffect(() => {
    const root = document.documentElement;
    const syncHeight = () => {
      const vv = window.visualViewport;
      const inner = window.innerHeight;
      const visual = vv?.height ?? inner;
      const offsetTop = vv?.offsetTop ?? 0;
      const isAndroid = root.dataset.platform === "android";

      if (isAndroid) {
        // Edge-to-edge WebView stays full-window; native `--ime-bottom` lifts
        // the shell. Fall back to the visual-viewport gap if insets are 0.
        if (inner > 0) {
          root.style.setProperty("--app-height", `${Math.round(inner)}px`);
        }
        const nativeIme =
          parseFloat(root.style.getPropertyValue("--ime-bottom") || "0") || 0;
        const fromVv = Math.max(0, Math.round(inner - visual - offsetTop));
        if (nativeIme < 24) {
          root.style.setProperty("--ime-bottom", `${fromVv}px`);
          if (fromVv > 40) root.dataset.ime = "open";
          else delete root.dataset.ime;
        }
        return;
      }

      const h = visual || inner;
      if (h > 0) root.style.setProperty("--app-height", `${Math.round(h)}px`);
    };
    syncHeight();
    window.addEventListener("resize", syncHeight);
    const vv = window.visualViewport;
    vv?.addEventListener("resize", syncHeight);
    vv?.addEventListener("scroll", syncHeight);
    let unlisten: (() => void) | undefined;
    getCurrentWindow()
      .onResized(syncHeight)
      .then((fn) => {
        unlisten = fn;
      })
      .catch(() => {});
    return () => {
      window.removeEventListener("resize", syncHeight);
      vv?.removeEventListener("resize", syncHeight);
      vv?.removeEventListener("scroll", syncHeight);
      unlisten?.();
    };
  }, []);
}
