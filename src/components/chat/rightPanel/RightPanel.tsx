import { useCallback, useEffect, useMemo } from "react";
import { useTranslation } from "react-i18next";
import { useSession } from "../../../store/session";
import { useProject } from "../../../store/project";
import { useRightPanel } from "../../../store/rightPanel";
import { collectSessionGalleryMedia } from "../../../sessionGallery";
import { PanelTabPane } from "./components/PanelTabPane";
import { TabBar } from "./components/TabBar";
import { TypePicker } from "./components/TypePicker";
import { PanelTabProvider } from "./context/PanelTabContext";
import { usePanelResize } from "./hooks/usePanelResize";
import { usePaneMounts } from "./hooks/usePaneMounts";
import { useTabScroll } from "./hooks/useTabScroll";
import type { PickerKind, RightPanelProps, TabKind } from "./types";

export type { RightPanelProps } from "./types";

export function RightPanel({ open, onClose, onPreviewImage }: RightPanelProps) {
  const { t } = useTranslation();
  const active = useSession((s) => s.active);
  const sessionMedia = useSession((s) => s.sessionMedia);
  const projects = useProject((s) => s.projects);

  // Tabs come straight from the panel store, which is bound to exactly one
  // session (see store/panelBindings.ts) — never another conversation's tabs.
  const sessionId = useRightPanel((s) => s.sessionId);
  const tabs = useRightPanel((s) => s.tabs);
  const activeTabId = useRightPanel((s) => s.activeTabId);
  const selectTab = useRightPanel((s) => s.selectTab);
  const addTabStore = useRightPanel((s) => s.addTab);
  const setTabKindStore = useRightPanel((s) => s.setTabKind);
  const closeTab = useRightPanel((s) => s.closeTab);
  const closeOtherTabs = useRightPanel((s) => s.closeOtherTabs);
  const closeTabsToRight = useRightPanel((s) => s.closeTabsToRight);
  const closeAllTabs = useRightPanel((s) => s.closeAllTabs);

  const hasProjectPath = useMemo(() => {
    const projectId = active?.session.project_id ?? null;
    if (!projectId) return false;
    return !!(projects.find((p) => p.id === projectId)?.path?.trim());
  }, [active, projects]);

  const { width, resizing, asideRef, onResizerMouseDown, resetWidth } = usePanelResize(open);

  const { tabsScrollRef, tabOverflow, updateTabOverflow, scrollTabs } = useTabScroll(
    tabs,
    activeTabId,
    width,
    open,
  );


  // Defensive: never leave the body blank if activeTabId ever lags the list.
  const activeId = tabs.some((tb) => tb.id === activeTabId)
    ? activeTabId
    : (tabs[0]?.id ?? null);

  const mountedIds = usePaneMounts(tabs, activeId);

  const addTab = useCallback(
    (kind: TabKind) => {
      if (kind === "reader" && !hasProjectPath) return;
      addTabStore(kind);
    },
    [addTabStore, hasProjectPath],
  );

  const setTabKind = useCallback(
    (id: string, kind: PickerKind) => {
      if (kind === "reader" && !hasProjectPath) return;
      setTabKindStore(id, kind);
    },
    [hasProjectPath, setTabKindStore],
  );

  // Opening a file always focuses its tab: it is an explicit user intent
  // (file tree, nav history, find result), never a passive restore.
  const openFileTab = useCallback((path: string) => {
    useRightPanel.getState().openFileTab(path, { reveal: true });
  }, []);

  const galleryCount = useMemo(
    () => collectSessionGalleryMedia(active, sessionMedia).length,
    [active, sessionMedia],
  );

  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !document.querySelector(".video-preview-lightbox")) {
        onClose();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, onClose]);

  const tabIndex = open ? 0 : -1;
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
        onDoubleClick={resetWidth}
      />

      <div className="chat-gallery-inner">
        <TabBar
          tabs={tabs}
          activeTabId={activeTabId}
          galleryCount={galleryCount}
          tabIndex={tabIndex}
          tabOverflow={tabOverflow}
          tabsScrollRef={tabsScrollRef}
          onScroll={updateTabOverflow}
          onScrollTabs={scrollTabs}
          onSelectTab={selectTab}
          onCloseTab={closeTab}
          onCloseOtherTabs={closeOtherTabs}
          onCloseTabsToRight={closeTabsToRight}
          onCloseAllTabs={closeAllTabs}
          onAddTab={() => addTab("empty")}
          onClosePanel={onClose}
        />

        {/* Keyed by session: switching conversations rebuilds every pane, so no
            editor / canvas instance can survive into another session. */}
        <div className="right-panel-body" key={sessionId ?? "none"}>
          {tabs.length === 0 ? (
            <div className="right-panel-pane">
              <TypePicker
                tab={tabIndex}
                showReader={hasProjectPath}
                onPick={(kind) => addTab(kind)}
              />
            </div>
          ) : (
            tabs.map((tb) =>
              mountedIds.has(tb.id) ? (
                <div
                  key={tb.id}
                  className={`right-panel-pane${tb.id === activeId ? "" : " is-hidden"}`}
                >
                  <PanelTabProvider
                    tabId={tb.id}
                    isActive={tb.id === activeId}
                    sessionId={sessionId}
                  >
                    <PanelTabPane
                      tab={tb}
                      panelOpen={open}
                      showReader={hasProjectPath}
                      onPickKind={(kind) => setTabKind(tb.id, kind)}
                      onPreviewImage={onPreviewImage}
                      onOpenFile={openFileTab}
                    />
                  </PanelTabProvider>
                </div>
              ) : null,
            )
          )}
        </div>
      </div>
    </aside>
  );
}
