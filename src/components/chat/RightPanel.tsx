import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { useSession } from "../../store/session";
import { collectSessionGalleryImages } from "../../sessionGallery";
import { GalleryContent } from "./SessionGallery";
import { AgentFlowPanel } from "./AgentFlowPanel";
import type { ImageRefAbs } from "../../types";

type TabKind = "empty" | "gallery" | "agent-flow";

interface PanelTab {
  id: string;
  kind: TabKind;
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
      ? parsed.tabs.filter(
          (tb): tb is PanelTab =>
            !!tb &&
            typeof tb.id === "string" &&
            (tb.kind === "empty" || tb.kind === "gallery" || tb.kind === "agent-flow"),
        )
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

  const galleryCount = useMemo(
    () => collectSessionGalleryImages(active).length,
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
      if (e.key === "Escape") onClose();
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
      default:
        return t("rightPanel.newTab");
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
          <div className="right-panel-tabs" role="tablist">
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
                  onClick={() => setActiveTabId(tb.id)}
                >
                  {tabLabel(tb.kind)}
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
  onPick: (kind: "gallery" | "agent-flow") => void;
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
