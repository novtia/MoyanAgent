import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { useSession } from "../../store/session";
import { collectSessionGalleryMedia } from "../../sessionGallery";
import { GalleryContent } from "./SessionGallery";
import { AgentFlowPanel } from "./AgentFlowPanel";
import { RoleStatePanel } from "./RoleStatePanel";
import { ReaderWorkspace } from "./ReaderWorkspace";
import { useReader, readerFileName, normalizeReaderPath } from "../../store/reader";
import { FileTypeIcon } from "../../utils/fileIcons";
import type { ImageRefAbs } from "../../types";

type TabKind = "empty" | "gallery" | "agent-flow" | "role-state" | "reader";

interface PanelTab {
  id: string;
  kind: TabKind;
  /** For reader tabs: the absolute file path bound to this tab (null = file picker). */
  path?: string | null;
}

interface RightPanelProps {
  open: boolean;
  onClose: () => void;
  onPreviewImage: (img: ImageRefAbs) => void;
}

const MIN_WIDTH = 240;
const MAX_WIDTH = 640;
const DEFAULT_WIDTH = 340;
const WIDTH_KEY = "atelier:right-panel-width";
const TABS_KEY = "atelier:right-panel-tabs";

function readStoredWidth() {
  if (typeof window === "undefined") return DEFAULT_WIDTH;
  const raw = window.localStorage.getItem(WIDTH_KEY);
  if (!raw) return DEFAULT_WIDTH;
  const v = parseInt(raw, 10);
  if (!Number.isFinite(v)) return DEFAULT_WIDTH;
  return Math.min(MAX_WIDTH, Math.max(MIN_WIDTH, v));
}

function newId() {
  return `tab-${Date.now()}-${Math.random().toString(36).slice(2, 7)}`;
}

function readStoredTabs(): { tabs: PanelTab[]; activeId: string | null } {
  if (typeof window === "undefined") return { tabs: [], activeId: null };
  try {
    const raw = window.localStorage.getItem(TABS_KEY);
    if (!raw) return { tabs: [], activeId: null };
    const parsed = JSON.parse(raw) as { tabs?: PanelTab[]; activeId?: string | null };
    const tabs = Array.isArray(parsed.tabs)
      ? parsed.tabs
          .filter(
            (tb): tb is PanelTab =>
              !!tb &&
              typeof tb.id === "string" &&
              (tb.kind === "empty" ||
                tb.kind === "gallery" ||
                tb.kind === "agent-flow" ||
                tb.kind === "role-state" ||
                tb.kind === "reader"),
          )
          .map((tb) => ({
            id: tb.id,
            kind: tb.kind,
            path: typeof tb.path === "string" ? tb.path : null,
          }))
      : [];
    const activeId = tabs.some((tb) => tb.id === parsed.activeId)
      ? (parsed.activeId as string)
      : tabs[0]?.id ?? null;
    return { tabs, activeId };
  } catch {
    return { tabs: [], activeId: null };
  }
}

export function RightPanel({ open, onClose, onPreviewImage }: RightPanelProps) {
  const { t } = useTranslation();
  const active = useSession((s) => s.active);

  const [width, setWidth] = useState<number>(readStoredWidth);
  const [resizing, setResizing] = useState(false);
  const dragRef = useRef<{ rightEdge: number } | null>(null);
  const asideRef = useRef<HTMLElement | null>(null);

  const initial = useRef(readStoredTabs());
  const [tabs, setTabs] = useState<PanelTab[]>(initial.current.tabs);
  const [activeTabId, setActiveTabId] = useState<string | null>(initial.current.activeId);

  const activeTabIdRef = useRef(activeTabId);
  useEffect(() => {
    activeTabIdRef.current = activeTabId;
  }, [activeTabId]);

  // ---- Horizontal scrolling for the tab strip when tabs overflow. ----
  const tabsScrollRef = useRef<HTMLDivElement | null>(null);
  const [tabOverflow, setTabOverflow] = useState({ left: false, right: false });

  const updateTabOverflow = useCallback(() => {
    const el = tabsScrollRef.current;
    if (!el) return;
    const left = el.scrollLeft > 1;
    const right = el.scrollLeft + el.clientWidth < el.scrollWidth - 1;
    setTabOverflow((prev) =>
      prev.left === left && prev.right === right ? prev : { left, right },
    );
  }, []);

  const scrollTabs = useCallback((dir: -1 | 1) => {
    const el = tabsScrollRef.current;
    if (!el) return;
    el.scrollBy({ left: dir * Math.max(120, el.clientWidth * 0.7), behavior: "smooth" });
  }, []);

  // Keep overflow arrows accurate on tab count / panel width changes.
  useEffect(() => {
    const el = tabsScrollRef.current;
    if (!el) return;
    updateTabOverflow();
    const ro = new ResizeObserver(updateTabOverflow);
    ro.observe(el);
    return () => ro.disconnect();
  }, [updateTabOverflow, tabs.length, width, open]);

  // Translate vertical wheel into horizontal scroll (non-passive so we can
  // preventDefault and stop the wheel from bubbling to the page).
  useEffect(() => {
    const el = tabsScrollRef.current;
    if (!el) return;
    const onWheel = (e: WheelEvent) => {
      if (el.scrollWidth <= el.clientWidth) return;
      if (e.deltaY === 0) return;
      e.preventDefault();
      el.scrollLeft += e.deltaY;
      updateTabOverflow();
    };
    el.addEventListener("wheel", onWheel, { passive: false });
    return () => el.removeEventListener("wheel", onWheel);
  }, [updateTabOverflow]);

  // Scroll the active tab into view when the selection changes.
  useEffect(() => {
    const el = tabsScrollRef.current;
    if (!el) return;
    const activeEl = el.querySelector<HTMLElement>(".right-panel-tab.is-active");
    if (activeEl) {
      const c = el.getBoundingClientRect();
      const a = activeEl.getBoundingClientRect();
      if (a.left < c.left) el.scrollLeft += a.left - c.left - 8;
      else if (a.right > c.right) el.scrollLeft += a.right - c.right + 8;
    }
    updateTabOverflow();
  }, [activeTabId, tabs, updateTabOverflow]);

  // Open a file as a top-level reader tab: reuse an existing tab for the same
  // path, fill the active empty reader tab, otherwise create a new tab.
  const openFileTab = useCallback((filePath: string) => {
    if (!filePath) return;
    const key = normalizeReaderPath(filePath);
    setTabs((prev) => {
      const existing = prev.find(
        (tb) => tb.kind === "reader" && tb.path && normalizeReaderPath(tb.path) === key,
      );
      if (existing) {
        setActiveTabId(existing.id);
        return prev;
      }
      const active = prev.find((tb) => tb.id === activeTabIdRef.current);
      if (active && active.kind === "reader" && !active.path) {
        return prev.map((tb) => (tb.id === active.id ? { ...tb, path: filePath } : tb));
      }
      const tab: PanelTab = { id: newId(), kind: "reader", path: filePath };
      setActiveTabId(tab.id);
      return [...prev, tab];
    });
  }, []);

  // Auto-open: when a document is requested (openSeq bumps), ensure a reader
  // tab exists and make it active so the document shows immediately.
  const readerOpenSeq = useReader((s) => s.openSeq);
  const lastReaderSeq = useRef(readerOpenSeq);
  const lastReaderActive = useRef<string | null>(useReader.getState().activeTabId);
  useEffect(() => {
    if (readerOpenSeq === lastReaderSeq.current) return;
    lastReaderSeq.current = readerOpenSeq;
    const st = useReader.getState();
    // Only surface a tab when the active document actually changed (an agent /
    // user open with activate). Passive lazy-loads keep activeTabId unchanged.
    if (st.activeTabId === lastReaderActive.current) return;
    lastReaderActive.current = st.activeTabId;
    const active = st.tabs.find((tb) => tb.id === st.activeTabId);
    if (active) openFileTab(active.path);
  }, [readerOpenSeq, openFileTab]);

  const galleryCount = useMemo(
    () => collectSessionGalleryMedia(active).length,
    [active],
  );

  useEffect(() => {
    try {
      window.localStorage.setItem(TABS_KEY, JSON.stringify({ tabs, activeId: activeTabId }));
    } catch {
      /* ignore */
    }
  }, [tabs, activeTabId]);

  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (
        e.key === "Escape" &&
        !document.querySelector(".video-preview-lightbox")
      ) {
        onClose();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, onClose]);

  useEffect(() => {
    if (!resizing) return;
    const onMove = (e: MouseEvent) => {
      if (!dragRef.current) return;
      const next = Math.min(
        MAX_WIDTH,
        Math.max(MIN_WIDTH, Math.round(dragRef.current.rightEdge - e.clientX)),
      );
      setWidth(next);
    };
    const onUp = () => {
      setResizing(false);
      dragRef.current = null;
    };
    window.addEventListener("mousemove", onMove);
    window.addEventListener("mouseup", onUp);
    return () => {
      window.removeEventListener("mousemove", onMove);
      window.removeEventListener("mouseup", onUp);
    };
  }, [resizing]);

  useEffect(() => {
    if (resizing) {
      const prevCursor = document.body.style.cursor;
      const prevSelect = document.body.style.userSelect;
      document.body.style.cursor = "col-resize";
      document.body.style.userSelect = "none";
      return () => {
        document.body.style.cursor = prevCursor;
        document.body.style.userSelect = prevSelect;
      };
    }
  }, [resizing]);

  useEffect(() => {
    if (resizing) return;
    try {
      window.localStorage.setItem(WIDTH_KEY, String(width));
    } catch {
      /* ignore */
    }
  }, [resizing, width]);

  const onResizerMouseDown = (e: React.MouseEvent) => {
    if (!open) return;
    if (e.button !== 0) return;
    if (!asideRef.current) return;
    e.preventDefault();
    const rect = asideRef.current.getBoundingClientRect();
    dragRef.current = { rightEdge: rect.right };
    setResizing(true);
  };

  const addTab = (kind: TabKind) => {
    const tab: PanelTab = { id: newId(), kind };
    setTabs((prev) => [...prev, tab]);
    setActiveTabId(tab.id);
  };

  const setTabKind = (id: string, kind: TabKind) => {
    setTabs((prev) => prev.map((tb) => (tb.id === id ? { ...tb, kind } : tb)));
  };

  const closeTab = (id: string) => {
    setTabs((prev) => {
      const next = prev.filter((tb) => tb.id !== id);
      setActiveTabId((cur) => {
        if (cur !== id) return cur;
        const idx = prev.findIndex((tb) => tb.id === id);
        const fallback = next[idx] ?? next[idx - 1] ?? next[0];
        return fallback?.id ?? null;
      });
      return next;
    });
  };

  const activeTab = tabs.find((tb) => tb.id === activeTabId) ?? null;
  const tab = open ? 0 : -1;

  const tabLabel = (kind: TabKind) => {
    switch (kind) {
      case "gallery":
        return t("rightPanel.galleryTab");
      case "agent-flow":
        return t("rightPanel.agentFlowTab");
      case "role-state":
        return t("rightPanel.roleStateTab");
      case "reader":
        return t("rightPanel.readerTab");
      default:
        return t("rightPanel.newTab");
    }
  };

  const tabTitle = (tb: PanelTab) =>
    tb.kind === "reader" && tb.path ? readerFileName(tb.path) : tabLabel(tb.kind);

  // Left-side static icon for each tab: file-type glyph for reader files,
  // kind glyph for the other panel types.
  const tabIcon = (tb: PanelTab) => {
    if (tb.kind === "reader" && tb.path) {
      return <FileTypeIcon name={readerFileName(tb.path)} className="right-panel-tab-icon" />;
    }
    switch (tb.kind) {
      case "gallery":
        return <GalleryIcon />;
      case "agent-flow":
        return <FlowIcon />;
      case "role-state":
        return <RoleStateIcon />;
      case "reader":
        return <ReaderIcon />;
      default:
        return <PlusIcon />;
    }
  };

  const style = { ["--chat-gallery-width" as string]: `${width}px` } as React.CSSProperties;

  return (
    <aside
      ref={asideRef}
      className={`chat-gallery right-panel ${open ? "open" : ""} ${resizing ? "is-resizing" : ""}`}
      aria-hidden={!open}
      aria-label={t("rightPanel.toggle")}
      style={style}
    >
      <div
        className="chat-gallery-resizer"
        role="separator"
        aria-orientation="vertical"
        title={t("chat.galleryResize")}
        onMouseDown={onResizerMouseDown}
        onDoubleClick={() => setWidth(DEFAULT_WIDTH)}
      />

      <div className="chat-gallery-inner">
        <div className="right-panel-tabbar">
          {tabOverflow.left && (
            <button
              type="button"
              className="right-panel-tab-scroll right-panel-tab-scroll--left"
              title={t("rightPanel.scrollLeft")}
              tabIndex={tab}
              onClick={() => scrollTabs(-1)}
            >
              <ChevronLeftIcon />
            </button>
          )}
          <div
            className="right-panel-tabs"
            role="tablist"
            ref={tabsScrollRef}
            onScroll={updateTabOverflow}
          >
            {tabs.map((tb) => (
              <div
                key={tb.id}
                className={`right-panel-tab ${tb.id === activeTabId ? "is-active" : ""}`}
                role="tab"
                aria-selected={tb.id === activeTabId}
              >
                <button
                  type="button"
                  className="right-panel-tab-label"
                  tabIndex={tab}
                  title={tb.kind === "reader" && tb.path ? tb.path : undefined}
                  onClick={() => setActiveTabId(tb.id)}
                >
                  {tabIcon(tb)}
                  {tabTitle(tb)}
                  {tb.kind === "gallery" && galleryCount > 0 && (
                    <span className="right-panel-tab-count">{galleryCount}</span>
                  )}
                </button>
                <button
                  type="button"
                  className="right-panel-tab-close"
                  title={t("rightPanel.closeTab")}
                  tabIndex={tab}
                  onClick={() => closeTab(tb.id)}
                >
                  <CloseIcon />
                </button>
              </div>
            ))}
          </div>
          {tabOverflow.right && (
            <button
              type="button"
              className="right-panel-tab-scroll right-panel-tab-scroll--right"
              title={t("rightPanel.scrollRight")}
              tabIndex={tab}
              onClick={() => scrollTabs(1)}
            >
              <ChevronRightIcon />
            </button>
          )}
          <div className="right-panel-tabbar-actions">
            <button
              type="button"
              className="ghost-btn"
              title={t("rightPanel.newTab")}
              tabIndex={tab}
              onClick={() => addTab("empty")}
            >
              <PlusIcon />
            </button>
            <button
              type="button"
              className="ghost-btn"
              title={t("common.close")}
              tabIndex={tab}
              onClick={onClose}
            >
              <CloseIcon />
            </button>
          </div>
        </div>

        <div className="right-panel-body">
          {!activeTab ? (
            <TypePicker
              tab={tab}
              onPick={(kind) => addTab(kind)}
            />
          ) : activeTab.kind === "empty" ? (
            <TypePicker
              tab={tab}
              onPick={(kind) => setTabKind(activeTab.id, kind)}
            />
          ) : activeTab.kind === "gallery" ? (
            <GalleryContent open={open} onPreviewImage={onPreviewImage} />
          ) : activeTab.kind === "role-state" ? (
            <RoleStatePanel open={open} />
          ) : activeTab.kind === "reader" ? (
            <ReaderWorkspace path={activeTab.path ?? null} onOpenFile={openFileTab} />
          ) : (
            <AgentFlowPanel open={open} />
          )}
        </div>
      </div>
    </aside>
  );
}

function TypePicker({
  tab,
  onPick,
}: {
  tab: number;
  onPick: (kind: "gallery" | "agent-flow" | "role-state" | "reader") => void;
}) {
  const { t } = useTranslation();
  return (
    <div className="right-panel-picker">
      <p className="right-panel-picker-title">{t("rightPanel.emptyTitle")}</p>
      <button
        type="button"
        className="right-panel-picker-card"
        tabIndex={tab}
        onClick={() => onPick("gallery")}
      >
        <span className="right-panel-picker-icon">
          <GalleryIcon />
        </span>
        <span className="right-panel-picker-text">
          <span className="right-panel-picker-name">{t("rightPanel.createGallery")}</span>
          <span className="right-panel-picker-desc">{t("rightPanel.createGalleryDesc")}</span>
        </span>
      </button>
      <button
        type="button"
        className="right-panel-picker-card"
        tabIndex={tab}
        onClick={() => onPick("agent-flow")}
      >
        <span className="right-panel-picker-icon">
          <FlowIcon />
        </span>
        <span className="right-panel-picker-text">
          <span className="right-panel-picker-name">{t("rightPanel.createAgentFlow")}</span>
          <span className="right-panel-picker-desc">{t("rightPanel.createAgentFlowDesc")}</span>
        </span>
      </button>
      <button
        type="button"
        className="right-panel-picker-card"
        tabIndex={tab}
        onClick={() => onPick("role-state")}
      >
        <span className="right-panel-picker-icon">
          <RoleStateIcon />
        </span>
        <span className="right-panel-picker-text">
          <span className="right-panel-picker-name">{t("rightPanel.createRoleState")}</span>
          <span className="right-panel-picker-desc">{t("rightPanel.createRoleStateDesc")}</span>
        </span>
      </button>
      <button
        type="button"
        className="right-panel-picker-card"
        tabIndex={tab}
        onClick={() => onPick("reader")}
      >
        <span className="right-panel-picker-icon">
          <ReaderIcon />
        </span>
        <span className="right-panel-picker-text">
          <span className="right-panel-picker-name">{t("rightPanel.createReader")}</span>
          <span className="right-panel-picker-desc">{t("rightPanel.createReaderDesc")}</span>
        </span>
      </button>
    </div>
  );
}

function CloseIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round">
      <path d="M6 6 18 18" />
      <path d="m18 6-12 12" />
    </svg>
  );
}

function ChevronLeftIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round">
      <path d="M15 18l-6-6 6-6" />
    </svg>
  );
}

function ChevronRightIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round">
      <path d="M9 18l6-6-6-6" />
    </svg>
  );
}

function PlusIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round">
      <path d="M12 5v14M5 12h14" />
    </svg>
  );
}

function GalleryIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round">
      <rect x="3" y="3" width="18" height="18" rx="2" />
      <circle cx="8.5" cy="8.5" r="1.5" />
      <path d="m21 15-5-5L5 21" />
    </svg>
  );
}

function FlowIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round">
      <rect x="3" y="4" width="7" height="5" rx="1" />
      <rect x="14" y="10" width="7" height="5" rx="1" />
      <rect x="3" y="16" width="7" height="5" rx="1" />
      <path d="M10 6.5h2a2 2 0 0 1 2 2v2M10 18.5h2a2 2 0 0 0 2-2v-2" />
    </svg>
  );
}

function RoleStateIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round">
      <path d="M12 3 4 7v5c0 4.4 3.2 7.6 8 9 4.8-1.4 8-4.6 8-9V7l-8-4Z" />
      <circle cx="12" cy="10" r="2.4" />
      <path d="M8.4 16c.5-1.8 2-2.8 3.6-2.8s3.1 1 3.6 2.8" />
    </svg>
  );
}

function ReaderIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round">
      <path d="M4 5.5A1.5 1.5 0 0 1 5.5 4H11v15H5.5A1.5 1.5 0 0 1 4 17.5Z" />
      <path d="M20 5.5A1.5 1.5 0 0 0 18.5 4H13v15h5.5a1.5 1.5 0 0 0 1.5-1.5Z" />
    </svg>
  );
}
