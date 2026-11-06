import {
  memo,
  useEffect,
  useMemo,
  useRef,
  useState,
  type MouseEvent as ReactMouseEvent,
  type ReactNode,
} from "react";
import { emit, listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useTranslation } from "react-i18next";
import { readerFileName } from "../../store/reader";
import {
  PHONE_READER_EVENT,
  PHONE_READER_READY,
  type PhoneReaderPayload,
} from "../../phoneReader/sync";
import {
  PHONE_WINDOW_LANDSCAPE,
  PHONE_WINDOW_PORTRAIT,
  loadPhonePrefs,
  savePhonePrefs,
  type PhoneFont,
  type PhonePrefs,
  type PhoneTheme,
} from "../../phoneReader/prefs";
import { copyText } from "../../utils/clipboard";

type Chrome = "hidden" | "menu" | "settings";

type PassageItem = {
  id: string;
  text: string;
  blank: boolean;
  indent: boolean;
  spaced: boolean;
};

type TocItem = { id: string; label: string };

const FONTS: PhoneFont[] = ["system", "serif", "sans", "kai"];
const SWATCHES: { id: PhoneTheme; key: string }[] = [
  { id: "white", key: "reader.phoneThemeWhite" },
  { id: "dark", key: "reader.phoneThemeDark" },
  { id: "warm", key: "reader.phoneThemeWarm" },
  { id: "green", key: "reader.phoneThemeGreen" },
  { id: "pink", key: "reader.phoneThemePink" },
  { id: "parchment", key: "reader.phoneThemeParchment" },
];

const FONT_LABEL: Record<PhoneFont, string> = {
  system: "reader.phoneFontSystem",
  serif: "reader.phoneFontSerif",
  sans: "reader.phoneFontSans",
  kai: "reader.phoneFontKai",
};

function chapterTitle(doc: PhoneReaderPayload | null, fallback: string): string {
  const raw = doc?.title?.trim() || (doc?.path ? readerFileName(doc.path) : "");
  const stripped = raw.replace(/\.(md|markdown|txt|text)$/i, "").trim();
  return stripped || fallback;
}

function headingOf(text: string): string | null {
  const trimmed = text.trim();
  if (!trimmed || trimmed === "\u00a0") return null;
  const md = /^(#{1,6})\s+(\S.*)$/.exec(trimmed);
  if (md) return md[2].trim().slice(0, 42);
  if (
    /^第[0-9零〇一二三四五六七八九十百千两]+[章节回部卷集]/.test(trimmed) &&
    trimmed.length <= 42
  ) {
    return trimmed;
  }
  return null;
}

function buildToc(items: PassageItem[]): TocItem[] {
  const heads: TocItem[] = [];
  for (const item of items) {
    const label = headingOf(item.text);
    if (label) heads.push({ id: item.id, label });
  }
  if (heads.length > 0) return heads.slice(0, 120);
  const rough: TocItem[] = [];
  let count = 0;
  for (const item of items) {
    if (item.blank) continue;
    if (count % 20 === 0) {
      const label = item.text.replace(/\s+/g, " ").trim().slice(0, 22);
      if (label) rough.push({ id: item.id, label });
    }
    count += 1;
    if (rough.length >= 80) break;
  }
  return rough;
}

function fillOf(value: number, min: number, max: number): string {
  const span = max - min;
  if (span <= 0) return "0%";
  return `${((value - min) / span) * 100}%`;
}

function clockLabel(date: Date): string {
  return `${String(date.getHours()).padStart(2, "0")}:${String(date.getMinutes()).padStart(2, "0")}`;
}

export function PhoneReaderView() {
  const { t } = useTranslation();
  const [doc, setDoc] = useState<PhoneReaderPayload | null>(null);
  const [prefs, setPrefs] = useState<PhonePrefs>(() => loadPhonePrefs());
  const [chrome, setChrome] = useState<Chrome>("hidden");
  const [tocOpen, setTocOpen] = useState(false);
  const [moreOpen, setMoreOpen] = useState(false);
  const [formatOpen, setFormatOpen] = useState(false);
  const [progress, setProgress] = useState(0);
  const [pinned, setPinned] = useState(false);
  const [battery, setBattery] = useState<number | null>(null);
  const [now, setNow] = useState(() => new Date());
  const [hint, setHint] = useState<string | null>(null);

  const bodyRef = useRef<HTMLElement | null>(null);
  const seeking = useRef(false);
  const hintTimer = useRef(0);
  const textRef = useRef("");
  const pathRef = useRef<string | null>(null);

  const indentOn = prefs.indentOverride ?? Boolean(doc?.firstLineIndent);
  const spacingOn = prefs.spacingOverride ?? Boolean(doc?.paragraphSpacing);
  const text = doc?.text;
  textRef.current = text ?? "";

  const items = useMemo(() => {
    if (text == null) return [];
    return text
      .split("\n")
      .map((line, index) => ({ line, index }))
      .filter(({ line }) => (spacingOn ? line.trim() !== "" : true))
      .map(({ line, index }) => {
        const blank = line.trim() === "";
        const shown = !blank && indentOn ? line.replace(/^[ \t\u3000]+/, "") : line;
        return {
          id: `phone-p-${index}`,
          text: blank ? "\u00a0" : shown,
          blank,
          indent: !blank && indentOn,
          spaced: !blank && spacingOn,
        };
      });
  }, [text, indentOn, spacingOn]);

  const toc = useMemo(() => buildToc(items), [items]);
  const title = chapterTitle(doc, t("reader.phone"));
  const brightness = prefs.followSystem ? 100 : prefs.brightness;
  const dim = ((100 - brightness) / 100) * 0.72;
  const hm = clockLabel(now);

  useEffect(() => {
    document.documentElement.classList.add("phone-reader-root");
    return () => {
      document.documentElement.classList.remove("phone-reader-root");
      window.clearTimeout(hintTimer.current);
    };
  }, []);

  useEffect(() => {
    savePhonePrefs(prefs);
  }, [prefs]);

  useEffect(() => {
    try {
      void getCurrentWindow()
        .setSize(prefs.landscape ? PHONE_WINDOW_LANDSCAPE : PHONE_WINDOW_PORTRAIT)
        .catch(() => {});
    } catch {
      /* opened outside the phone window */
    }
  }, [prefs.landscape]);

  useEffect(() => {
    try {
      void getCurrentWindow()
        .isAlwaysOnTop()
        .then(setPinned)
        .catch(() => {});
    } catch {
      /* ignore */
    }
  }, []);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    void listen<PhoneReaderPayload>(PHONE_READER_EVENT, (event) => {
      setDoc(event.payload);
    }).then((stop) => {
      if (cancelled) {
        stop();
        return;
      }
      unlisten = stop;
      void emit(PHONE_READER_READY);
    });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  useEffect(() => {
    const tick = () => {
      const next = new Date();
      setNow((prev) =>
        prev.getHours() === next.getHours() && prev.getMinutes() === next.getMinutes() ? prev : next,
      );
    };
    tick();
    const id = window.setInterval(tick, 1000);
    return () => window.clearInterval(id);
  }, []);

  useEffect(() => {
    const nav = navigator as Navigator & {
      getBattery?: () => Promise<{
        level: number;
        addEventListener: (type: "levelchange", listener: () => void) => void;
        removeEventListener: (type: "levelchange", listener: () => void) => void;
      }>;
    };
    if (!nav.getBattery) return;
    let batteryRef: {
      level: number;
      removeEventListener: (type: "levelchange", listener: () => void) => void;
    } | null = null;
    let dead = false;
    const sync = () => {
      if (!dead && batteryRef) setBattery(Math.round(batteryRef.level * 100));
    };
    void nav
      .getBattery()
      .then((manager) => {
        if (dead) return;
        batteryRef = manager;
        sync();
        manager.addEventListener("levelchange", sync);
      })
      .catch(() => {});
    return () => {
      dead = true;
      batteryRef?.removeEventListener("levelchange", sync);
    };
  }, []);

  useEffect(() => {
    if (!doc?.path) return;
    if (pathRef.current === doc.path) return;
    pathRef.current = doc.path;
    const el = bodyRef.current;
    if (el) el.scrollTop = 0;
    setProgress(0);
  }, [doc?.path]);

  useEffect(() => {
    const el = bodyRef.current;
    if (!el || seeking.current) return;
    const max = el.scrollHeight - el.clientHeight;
    const next = max <= 0 ? 0 : (el.scrollTop / max) * 100;
    setProgress((prev) => (Math.abs(prev - next) < 0.15 ? prev : next));
  }, [text, prefs.fontSize, indentOn, spacingOn]);

  useEffect(() => {
    if (!prefs.autoRead) return;
    let raf = 0;
    let last = performance.now();
    const tick = (nowMs: number) => {
      const el = bodyRef.current;
      const max = el ? el.scrollHeight - el.clientHeight : 0;
      if (!el || max <= 1) {
        if (textRef.current && max <= 1) {
          setPrefs((prev) => (prev.autoRead ? { ...prev, autoRead: false } : prev));
          return;
        }
        raf = requestAnimationFrame(tick);
        return;
      }
      if (el.scrollTop >= max - 1) {
        setPrefs((prev) => (prev.autoRead ? { ...prev, autoRead: false } : prev));
        return;
      }
      const dt = Math.min(40, nowMs - last);
      last = nowMs;
      el.scrollTop += dt * 0.042;
      raf = requestAnimationFrame(tick);
    };
    raf = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(raf);
  }, [prefs.autoRead]);

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      setTocOpen(false);
      setMoreOpen(false);
      setFormatOpen(false);
      setChrome("hidden");
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  function showHint(message: string) {
    setHint(message);
    window.clearTimeout(hintTimer.current);
    hintTimer.current = window.setTimeout(() => setHint(null), 1700);
  }

  function page(dir: -1 | 1) {
    const el = bodyRef.current;
    if (!el) return;
    el.scrollBy({ top: dir * Math.max(120, el.clientHeight * 0.86), behavior: "smooth" });
  }

  function seek(value: number) {
    const el = bodyRef.current;
    setProgress(value);
    if (!el) return;
    const max = el.scrollHeight - el.clientHeight;
    el.scrollTop = max <= 0 ? 0 : (max * value) / 100;
  }

  function onScroll() {
    if (seeking.current) return;
    const el = bodyRef.current;
    if (!el) return;
    const max = el.scrollHeight - el.clientHeight;
    const next = max <= 0 ? 0 : (el.scrollTop / max) * 100;
    setProgress((prev) => (Math.abs(prev - next) < 0.15 ? prev : next));
  }

  function beginDrag(event: ReactMouseEvent) {
    if (event.button !== 0) return;
    event.preventDefault();
    try {
      void getCurrentWindow().startDragging().catch(() => {});
    } catch {
      /* ignore */
    }
  }

  async function togglePin() {
    try {
      const win = getCurrentWindow();
      await win.setAlwaysOnTop(!pinned);
      setPinned(await win.isAlwaysOnTop());
    } catch {
      showHint(t("reader.phonePinFailed"));
    }
  }

  function closeWindow() {
    try {
      void getCurrentWindow().close();
    } catch {
      /* ignore */
    }
  }

  function releaseFocusWithin(selector: string) {
    const active = document.activeElement;
    if (active instanceof HTMLElement && active.closest(selector)) active.blur();
  }

  function dismissChrome() {
    releaseFocusWithin(".phone-menu-sheet, .phone-settings-sheet, .phone-toc, .phone-appbar");
    setChrome("hidden");
    setTocOpen(false);
    setMoreOpen(false);
    setFormatOpen(false);
  }

  function onScreenPointerDown(event: ReactMouseEvent<HTMLDivElement>) {
    const target = event.target as HTMLElement;
    if (
      target.closest(
        "button, input, a, textarea, .phone-sheet, .phone-appbar, .phone-toc, .phone-listen, .phone-speaker",
      )
    ) {
      return;
    }
    if (!prefs.autoRead) return;
    setPrefs((prev) => ({ ...prev, autoRead: false }));
  }

  function onScreenClick(event: ReactMouseEvent<HTMLDivElement>) {
    const target = event.target as HTMLElement;
    if (
      target.closest(
        "button, input, a, textarea, .phone-sheet, .phone-appbar, .phone-toc, .phone-listen, .phone-speaker, .phone-more-pop",
      )
    ) {
      return;
    }
    if ((window.getSelection()?.toString() ?? "").trim()) return;
    if (prefs.autoRead) {
      setPrefs((prev) => ({ ...prev, autoRead: false }));
      showHint(t("reader.phonePaused"));
      return;
    }
    if (chrome !== "hidden" || tocOpen || moreOpen || formatOpen) {
      dismissChrome();
      return;
    }
    const rect = event.currentTarget.getBoundingClientRect();
    const ratio = rect.height > 0 ? (event.clientY - rect.top) / rect.height : 0.5;
    if (ratio < 0.25) page(-1);
    else if (ratio > 0.75) page(1);
    else setChrome("menu");
  }

  function toggleAutoRead() {
    if (prefs.autoRead) {
      setPrefs((prev) => ({ ...prev, autoRead: false }));
      return;
    }
    const el = bodyRef.current;
    const max = el ? el.scrollHeight - el.clientHeight : 0;
    if (el && max > 1 && el.scrollTop >= max - 1) {
      showHint(t("reader.phoneAtEnd"));
      return;
    }
    if (textRef.current && max <= 1) {
      showHint(t("reader.phoneAtEnd"));
      return;
    }
    setPrefs((prev) => ({ ...prev, autoRead: true }));
  }

  function toggleNight() {
    setPrefs((prev) => {
      if (prev.nightMode) {
        const day = prev.dayTheme === "dark" ? "parchment" : prev.dayTheme;
        return { ...prev, nightMode: false, theme: day };
      }
      return {
        ...prev,
        nightMode: true,
        dayTheme: prev.theme === "dark" ? prev.dayTheme : prev.theme,
        theme: "dark",
      };
    });
    setChrome("menu");
  }

  function pickTheme(theme: PhoneTheme) {
    setPrefs((prev) => {
      if (theme === "dark") {
        return {
          ...prev,
          theme: "dark",
          nightMode: true,
          dayTheme: prev.theme === "dark" ? prev.dayTheme : prev.theme,
        };
      }
      return { ...prev, theme, nightMode: false, dayTheme: theme };
    });
  }

  function cycleFont() {
    setPrefs((prev) => {
      const index = FONTS.indexOf(prev.font);
      return { ...prev, font: FONTS[(index + 1) % FONTS.length] };
    });
  }

  function jump(id: string) {
    if (prefs.autoRead) setPrefs((prev) => ({ ...prev, autoRead: false }));
    const scroller = bodyRef.current;
    const node = document.getElementById(id);
    if (scroller && node && scroller.contains(node)) {
      const top =
        node.getBoundingClientRect().top - scroller.getBoundingClientRect().top + scroller.scrollTop;
      scroller.scrollTo({ top: Math.max(0, top), behavior: "smooth" });
    }
    const screen = scroller?.closest(".phone-screen");
    if (screen instanceof HTMLElement) screen.scrollTop = 0;
    document.documentElement.scrollTop = 0;
    document.body.scrollTop = 0;
    dismissChrome();
  }

  async function copyAll() {
    setMoreOpen(false);
    if (!doc?.text.trim()) {
      showHint(t("reader.phoneEmpty"));
      return;
    }
    try {
      await copyText(doc.text);
      showHint(t("reader.phoneCopied"));
    } catch {
      showHint(t("reader.phoneCopyFailed"));
    }
  }

  function goEdge(edge: "start" | "end") {
    const el = bodyRef.current;
    if (!el) return;
    if (prefs.autoRead) setPrefs((prev) => ({ ...prev, autoRead: false }));
    el.scrollTo({ top: edge === "start" ? 0 : el.scrollHeight, behavior: "smooth" });
  }

  const screenClass = [
    "phone-screen",
    chrome === "menu" ? "is-menu" : "",
    chrome === "settings" ? "is-settings" : "",
    tocOpen ? "is-toc" : "",
    prefs.eyeCare ? "is-eye" : "",
  ]
    .filter(Boolean)
    .join(" ");

  return (
    <div className={`phone-stage${prefs.landscape ? " is-landscape" : ""}`}>
      <div className="phone-device">
        <span className="phone-side phone-side-silent" />
        <span className="phone-side phone-side-volume-up" />
        <span className="phone-side phone-side-volume-down" />
        <span className="phone-side phone-side-power" />
        <div
          className={screenClass}
          data-theme={prefs.theme}
          data-font={prefs.font}
          style={{ ["--phone-dim" as string]: String(dim) }}
          onClick={onScreenClick}
          onMouseDown={onScreenPointerDown}
        >
          <div
            className="phone-speaker"
            data-tauri-drag-region
            onMouseDown={beginDrag}
            title={t("reader.phoneDrag")}
          />
          <header className="phone-status">
            {chrome === "menu" ? (
              <div className="phone-sys">
                <span className="phone-sys-left">
                  <SignalIcon />
                  <WifiIcon />
                </span>
                <span className="phone-sys-right">
                  {battery != null ? <span className="phone-battery-num">{battery}</span> : null}
                  <BatteryIcon level={battery ?? 100} />
                  <time dateTime={hm}>{hm}</time>
                </span>
              </div>
            ) : (
              <div className="phone-readbar">
                <span className="phone-status-title">{title}</span>
                <span className="phone-status-meta">
                  <BatteryIcon level={battery ?? 100} />
                  <time dateTime={hm}>{hm}</time>
                </span>
              </div>
            )}
          </header>

          {chrome === "menu" ? (
            <div className="phone-appbar">
              <button
                type="button"
                className="phone-icon-btn"
                aria-label={t("reader.phoneCollapse")}
                title={t("reader.phoneCollapse")}
                onClick={dismissChrome}
              >
                <ChevronLeftIcon />
              </button>
              <span className="phone-appbar-gap" />
              <span className="phone-more-wrap">
                <button
                  type="button"
                  className="phone-icon-btn"
                  aria-expanded={moreOpen}
                  aria-label={t("reader.more")}
                  onClick={() => setMoreOpen((open) => !open)}
                >
                  <DotsIcon />
                </button>
                {moreOpen ? (
                  <div className="phone-more-pop" role="menu">
                    <button type="button" role="menuitem" onClick={() => void copyAll()}>
                      {t("reader.phoneCopyAll")}
                    </button>
                    <button
                      type="button"
                      role="menuitem"
                      onClick={() => {
                        setMoreOpen(false);
                        goEdge("start");
                      }}
                    >
                      {t("reader.phoneBackTop")}
                    </button>
                  </div>
                ) : null}
              </span>
            </div>
          ) : null}

          <article
            ref={bodyRef}
            className="phone-reader-body"
            style={{ fontSize: prefs.fontSize }}
            onScroll={onScroll}
          >
            {!doc || items.length === 0 ? (
              <p className="phone-reader-empty">{t("reader.phoneEmpty")}</p>
            ) : (
              <Passage items={items} />
            )}
          </article>
          <div className="phone-home" />

          <div className="phone-grain" />
          <div className="phone-eye" />

          <div className="phone-sheet phone-menu-sheet" {...(chrome !== "menu" ? { inert: "" } : {})}>
            <div className="phone-progress">
              <button type="button" className="phone-edge" disabled={!doc} onClick={() => goEdge("start")}>
                {t("reader.phonePrev")}
              </button>
              <input
                className="phone-slider"
                type="range"
                min={0}
                max={100}
                step="any"
                value={progress}
                disabled={!doc}
                aria-label={t("reader.phoneProgress")}
                style={{ ["--fill" as string]: `${progress}%` }}
                onPointerDown={() => {
                  seeking.current = true;
                  if (prefs.autoRead) setPrefs((prev) => ({ ...prev, autoRead: false }));
                  const stop = () => {
                    seeking.current = false;
                    window.removeEventListener("pointerup", stop);
                    window.removeEventListener("pointercancel", stop);
                  };
                  window.addEventListener("pointerup", stop);
                  window.addEventListener("pointercancel", stop);
                }}
                onChange={(event) => seek(Number(event.target.value))}
              />
              <button type="button" className="phone-edge" disabled={!doc} onClick={() => goEdge("end")}>
                {t("reader.phoneNext")}
              </button>
            </div>
            <div className="phone-nav">
              <button type="button" className="phone-nav-btn" onClick={() => setTocOpen(true)}>
                <ListIcon />
                {t("reader.phoneToc")}
              </button>
              <button type="button" className="phone-nav-btn" onClick={toggleNight}>
                {prefs.nightMode ? <SunIcon /> : <MoonIcon />}
                {prefs.nightMode ? t("reader.phoneDay") : t("reader.phoneNight")}
              </button>
              <button
                type="button"
                className="phone-nav-btn"
                onClick={() => {
                  releaseFocusWithin(".phone-menu-sheet");
                  setMoreOpen(false);
                  setTocOpen(false);
                  setChrome("settings");
                }}
              >
                <GearIcon />
                {t("reader.phoneSettings")}
              </button>
            </div>
          </div>

          <div className="phone-sheet phone-settings-sheet" {...(chrome !== "settings" ? { inert: "" } : {})}>
            <div className="phone-setting-row">
              <span className="phone-setting-label">{t("reader.phoneBrightness")}</span>
              <div className="phone-setting-control">
                <SunIcon small />
                <input
                  className="phone-slider"
                  type="range"
                  min={25}
                  max={100}
                  value={prefs.brightness}
                  disabled={prefs.followSystem}
                  aria-label={t("reader.phoneBrightness")}
                  style={{ ["--fill" as string]: fillOf(prefs.brightness, 25, 100) }}
                  onChange={(event) =>
                    setPrefs((prev) => ({ ...prev, brightness: Number(event.target.value) }))
                  }
                />
                <SunIcon />
                <button
                  type="button"
                  className={`phone-check${prefs.followSystem ? " is-on" : ""}`}
                  aria-pressed={prefs.followSystem}
                  onClick={() => setPrefs((prev) => ({ ...prev, followSystem: !prev.followSystem }))}
                >
                  <span className="phone-check-dot" />
                  {t("reader.phoneFollowSystem")}
                </button>
              </div>
            </div>
            <div className="phone-setting-row">
              <span className="phone-setting-label">{t("reader.phoneFontLabel")}</span>
              <div className="phone-setting-control">
                <button
                  type="button"
                  className="phone-step"
                  aria-label="A-"
                  disabled={prefs.fontSize <= 12}
                  onClick={() =>
                    setPrefs((prev) => ({ ...prev, fontSize: Math.max(12, prev.fontSize - 1) }))
                  }
                >
                  A-
                </button>
                <span className="phone-step-value">{prefs.fontSize}</span>
                <button
                  type="button"
                  className="phone-step"
                  aria-label="A+"
                  disabled={prefs.fontSize >= 32}
                  onClick={() =>
                    setPrefs((prev) => ({ ...prev, fontSize: Math.min(32, prev.fontSize + 1) }))
                  }
                >
                  A+
                </button>
                <button type="button" className="phone-font-btn" onClick={cycleFont}>
                  {t(FONT_LABEL[prefs.font])}
                  <ChevronRightIcon />
                </button>
              </div>
            </div>
            <div className="phone-setting-row">
              <span className="phone-setting-label">{t("reader.phoneBg")}</span>
              <div className="phone-swatches">
                {SWATCHES.map((swatch) => (
                  <button
                    key={swatch.id}
                    type="button"
                    className={`phone-swatch${prefs.theme === swatch.id ? " is-on" : ""}`}
                    data-swatch={swatch.id}
                    aria-label={t(swatch.key)}
                    aria-pressed={prefs.theme === swatch.id}
                    title={t(swatch.key)}
                    onClick={() => pickTheme(swatch.id)}
                  />
                ))}
              </div>
            </div>
            {formatOpen ? (
              <div className="phone-format">
                <button
                  type="button"
                  className={indentOn ? "is-on" : ""}
                  aria-pressed={indentOn}
                  onClick={() => setPrefs((prev) => ({ ...prev, indentOverride: !indentOn }))}
                >
                  {t("reader.phoneIndent")}
                </button>
                <button
                  type="button"
                  className={spacingOn ? "is-on" : ""}
                  aria-pressed={spacingOn}
                  onClick={() => setPrefs((prev) => ({ ...prev, spacingOverride: !spacingOn }))}
                >
                  {t("reader.phoneSpacing")}
                </button>
              </div>
            ) : null}
            <div className="phone-tools">
              <button
                type="button"
                className={`phone-tool${prefs.eyeCare ? " is-on" : ""}`}
                aria-pressed={prefs.eyeCare}
                onClick={() => setPrefs((prev) => ({ ...prev, eyeCare: !prev.eyeCare }))}
              >
                <EyeIcon />
                {t("reader.phoneEye")}
              </button>
              <button
                type="button"
                className={`phone-tool${prefs.autoRead ? " is-on" : ""}`}
                aria-pressed={prefs.autoRead}
                onClick={toggleAutoRead}
              >
                <span className="phone-tool-icon">
                  <PlayIcon />
                  <span className="phone-dot" />
                </span>
                {t("reader.phoneAuto")}
              </button>
              <button
                type="button"
                className={`phone-tool${prefs.landscape ? " is-on" : ""}`}
                aria-pressed={prefs.landscape}
                onClick={() => setPrefs((prev) => ({ ...prev, landscape: !prev.landscape }))}
              >
                <RotateIcon />
                {t("reader.phoneLandscape")}
              </button>
              <button
                type="button"
                className={`phone-tool${formatOpen ? " is-on" : ""}`}
                aria-expanded={formatOpen}
                onClick={() => setFormatOpen((open) => !open)}
              >
                <SlidersIcon />
                {t("reader.phoneMoreSettings")}
              </button>
            </div>
          </div>

          <button
            type="button"
            className="phone-toc-backdrop"
            aria-label={t("reader.phoneCollapse")}
            tabIndex={tocOpen ? 0 : -1}
            onClick={() => {
              releaseFocusWithin(".phone-toc");
              setTocOpen(false);
            }}
          />
          <aside className="phone-toc" aria-label={t("reader.phoneToc")} {...(!tocOpen ? { inert: "" } : {})}>
            <div className="phone-toc-head">
              <span>{t("reader.phoneToc")}</span>
              <button
                type="button"
                className="phone-icon-btn"
                aria-label={t("reader.phoneCollapse")}
                onClick={() => {
                  releaseFocusWithin(".phone-toc");
                  setTocOpen(false);
                }}
              >
                <ChevronLeftIcon />
              </button>
            </div>
            {toc.length === 0 ? (
              <p className="phone-toc-empty">{t("reader.phoneTocEmpty")}</p>
            ) : (
              <ul className="phone-toc-list">
                {toc.map((item) => (
                  <li key={item.id}>
                    <button type="button" className="phone-toc-item" onClick={() => jump(item.id)}>
                      {item.label}
                    </button>
                  </li>
                ))}
              </ul>
            )}
          </aside>

          <div className="phone-dim" />
          {hint ? <div className="phone-hint">{hint}</div> : null}
        </div>
      </div>

      <div className="phone-side-dock" role="toolbar" aria-label={t("reader.phone")}>
        <button
          type="button"
          className="phone-dock-btn is-drag"
          data-tauri-drag-region
          title={t("reader.phoneDrag")}
          aria-label={t("reader.phoneDrag")}
          onMouseDown={beginDrag}
        >
          <GripIcon />
        </button>
        <button
          type="button"
          className={`phone-dock-btn${pinned ? " is-on" : ""}`}
          title={pinned ? t("reader.phoneUnpin") : t("reader.phonePin")}
          aria-label={pinned ? t("reader.phoneUnpin") : t("reader.phonePin")}
          aria-pressed={pinned}
          onClick={() => void togglePin()}
        >
          <PinIcon />
        </button>
        <button
          type="button"
          className="phone-dock-btn is-close"
          title={t("reader.phoneClose")}
          aria-label={t("reader.phoneClose")}
          onClick={closeWindow}
        >
          <CloseIcon />
        </button>
      </div>
    </div>
  );
}

const Passage = memo(function Passage({ items }: { items: PassageItem[] }) {
  return (
    <>
      {items.map((item) => {
        const className = [
          "phone-reader-p",
          item.blank ? "is-blank" : "",
          item.indent ? "is-indent" : "",
          item.spaced ? "is-spaced" : "",
        ]
          .filter(Boolean)
          .join(" ");
        return (
          <p key={item.id} id={item.id} className={className}>
            {item.text}
          </p>
        );
      })}
    </>
  );
});

function IconSvg({
  children,
  size = 20,
}: {
  children: ReactNode;
  size?: number;
}) {
  return (
    <svg viewBox="0 0 24 24" width={size} height={size} aria-hidden="true">
      {children}
    </svg>
  );
}

function strokeProps() {
  return {
    fill: "none" as const,
    stroke: "currentColor",
    strokeWidth: 1.7,
    strokeLinecap: "round" as const,
    strokeLinejoin: "round" as const,
  };
}

function CloseIcon() {
  return (
    <IconSvg size={16}>
      <path d="M7 7l10 10M17 7L7 17" {...strokeProps()} />
    </IconSvg>
  );
}

function GripIcon() {
  return (
    <IconSvg size={16}>
      <circle cx="9" cy="8" r="1.2" fill="currentColor" />
      <circle cx="15" cy="8" r="1.2" fill="currentColor" />
      <circle cx="9" cy="12" r="1.2" fill="currentColor" />
      <circle cx="15" cy="12" r="1.2" fill="currentColor" />
      <circle cx="9" cy="16" r="1.2" fill="currentColor" />
      <circle cx="15" cy="16" r="1.2" fill="currentColor" />
    </IconSvg>
  );
}

function PinIcon() {
  return (
    <IconSvg size={16}>
      <path d="M8.5 4.5h7l-1.1 6.1 2.3 2.2H7.3l2.3-2.2L8.5 4.5z" {...strokeProps()} />
      <path d="M12 12.8V20" {...strokeProps()} />
    </IconSvg>
  );
}

function ChevronLeftIcon() {
  return (
    <IconSvg size={22}>
      <path d="M14.5 5.5L8 12l6.5 6.5" {...strokeProps()} />
    </IconSvg>
  );
}

function ChevronRightIcon() {
  return (
    <IconSvg size={14}>
      <path d="M9 6l5 6-5 6" {...strokeProps()} />
    </IconSvg>
  );
}

function DotsIcon() {
  return (
    <IconSvg size={18}>
      <circle cx="6" cy="12" r="1.3" fill="currentColor" />
      <circle cx="12" cy="12" r="1.3" fill="currentColor" />
      <circle cx="18" cy="12" r="1.3" fill="currentColor" />
    </IconSvg>
  );
}

function SignalIcon() {
  return (
    <svg viewBox="0 0 16 12" width="15" height="11" aria-hidden="true">
      <rect x="0" y="7" width="2.3" height="5" rx="0.4" fill="currentColor" />
      <rect x="3.5" y="5" width="2.3" height="7" rx="0.4" fill="currentColor" />
      <rect x="7" y="2.6" width="2.3" height="9.4" rx="0.4" fill="currentColor" />
      <rect x="10.5" y="0.4" width="2.3" height="11.6" rx="0.4" fill="currentColor" opacity="0.35" />
    </svg>
  );
}

function WifiIcon() {
  return (
    <svg viewBox="0 0 24 24" width="14" height="14" aria-hidden="true">
      <path d="M4.5 10.2c4.2-3.7 10.8-3.7 15 0" {...strokeProps()} />
      <path d="M7.4 13.2c2.6-2.2 6.6-2.2 9.2 0" {...strokeProps()} />
      <path d="M10.2 16.1c1-0.9 2.6-0.9 3.6 0" {...strokeProps()} />
      <circle cx="12" cy="18.6" r="1" fill="currentColor" />
    </svg>
  );
}

function BatteryIcon({ level }: { level: number }) {
  const width = 19.4 * (Math.max(0, Math.min(100, level)) / 100);
  return (
    <svg viewBox="0 0 28 14" width="22" height="11" aria-hidden="true">
      <rect x="0.7" y="0.8" width="23.2" height="12.4" rx="2.2" fill="none" stroke="currentColor" strokeWidth="1.3" />
      <rect x="24.6" y="4.3" width="2" height="5.4" rx="0.6" fill="currentColor" />
      <rect x="2.3" y="2.4" width={width} height="9.2" rx="1" fill="currentColor" />
    </svg>
  );
}

function ListIcon() {
  return (
    <IconSvg size={22}>
      <path d="M8 7h11M8 12h11M8 17h11" {...strokeProps()} />
      <circle cx="4.5" cy="7" r="1" fill="currentColor" />
      <circle cx="4.5" cy="12" r="1" fill="currentColor" />
      <circle cx="4.5" cy="17" r="1" fill="currentColor" />
    </IconSvg>
  );
}

function MoonIcon() {
  return (
    <IconSvg size={22}>
      <path d="M15.5 4.8A7 7 0 1 0 19 14.2 5.4 5.4 0 0 1 15.5 4.8z" {...strokeProps()} />
    </IconSvg>
  );
}

function SunIcon({ small = false }: { small?: boolean }) {
  return (
    <IconSvg size={small ? 14 : 20}>
      <circle cx="12" cy="12" r={small ? 2.4 : 3.2} {...strokeProps()} />
      <path d="M12 3.5v2.2M12 18.3v2.2M3.5 12h2.2M18.3 12h2.2M6 6l1.6 1.6M16.4 16.4L18 18M18 6l-1.6 1.6M7.6 16.4L6 18" {...strokeProps()} />
    </IconSvg>
  );
}

function GearIcon() {
  return (
    <IconSvg size={22}>
      <circle cx="12" cy="12" r="3" {...strokeProps()} />
      <path d="M12 3.8v2.1M12 18.1v2.1M3.8 12h2.1M18.1 12h2.1M6.1 6.1l1.5 1.5M16.4 16.4l1.5 1.5M17.9 6.1l-1.5 1.5M7.6 16.4l-1.5 1.5" {...strokeProps()} />
    </IconSvg>
  );
}

function EyeIcon() {
  return (
    <IconSvg size={22}>
      <path d="M3 12s3.2-5.5 9-5.5S21 12 21 12s-3.2 5.5-9 5.5S3 12 3 12z" {...strokeProps()} />
      <circle cx="12" cy="12" r="2.2" {...strokeProps()} />
    </IconSvg>
  );
}

function PlayIcon() {
  return (
    <IconSvg size={22}>
      <path d="M8 6.5v11l9-5.5-9-5.5z" {...strokeProps()} />
    </IconSvg>
  );
}

function RotateIcon() {
  return (
    <IconSvg size={22}>
      <path d="M7 8.5V5.5H4" {...strokeProps()} />
      <path d="M6.2 15.5a6.2 6.2 0 1 0-1-5.2" {...strokeProps()} />
      <rect x="9" y="9" width="8" height="6" rx="1" {...strokeProps()} />
    </IconSvg>
  );
}

function SlidersIcon() {
  return (
    <IconSvg size={22}>
      <path d="M5 8h14M5 16h14" {...strokeProps()} />
      <circle cx="9" cy="8" r="1.7" {...strokeProps()} />
      <circle cx="15" cy="16" r="1.7" {...strokeProps()} />
    </IconSvg>
  );
}
