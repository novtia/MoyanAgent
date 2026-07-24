import { useEffect, useMemo } from "react";
import { useTranslation } from "react-i18next";
import { useSession } from "../../../store/session";
import { useProject } from "../../../store/project";
import { collectSessionGalleryMedia } from "../../../sessionGallery";
import { GalleryContent } from "./gallery";
import { AgentFlowPanel } from "./agentFlow";
import { RoleStatePanel } from "./roleState";
import { ReaderWorkspace } from "./reader";
import { TabBar } from "./components/TabBar";
import { TypePicker } from "./components/TypePicker";
import { useOpenFileTab } from "./hooks/useOpenFileTab";
import { usePanelResize } from "./hooks/usePanelResize";
import { usePanelTabs } from "./hooks/usePanelTabs";
import { useReaderPanelSync } from "./hooks/useReaderPanelSync";
import { useTabScroll } from "./hooks/useTabScroll";
import type { RightPanelProps } from "./types";

export type { RightPanelProps } from "./types";

export function RightPanel({ open, onClose, onPreviewImage }: RightPanelProps) {
  const { t } = useTranslation();
  const active = useSession((s) => s.active);
  const activeSessionId = useSession((s) => s.activeId);
  const projects = useProject((s) => s.projects);

  const hasProjectPath = useMemo(() => {
    const projectId = active?.session.project_id ?? null;
    if (!projectId) return false;
    return !!(projects.find((p) => p.id === projectId)?.path?.trim());
  }, [active, projects]);

  const {
    width,
    resizing,
    asideRef,
    onResizerMouseDown,
    resetWidth,
  } = usePanelResize(open);

  const {
    tabs,
    setTabs,
    activeTabId,
    setActiveTabId,
    activeTabIdRef,
    addTab,
    setTabKind,
    closeTab,
  } = usePanelTabs(activeSessionId, hasProjectPath);

  const { tabsScrollRef, tabOverflow, updateTabOverflow, scrollTabs } = useTabScroll(
    tabs,
    activeTabId,
    width,
    open,
  );

  const openFileTab = useOpenFileTab(setTabs, setActiveTabId, activeTabIdRef);
  useReaderPanelSync(openFileTab, setTabs, setActiveTabId);

  const galleryCount = useMemo(
    () => collectSessionGalleryMedia(active).length,
    [active],
  );

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

  const activeTab = tabs.find((tb) => tb.id === activeTabId) ?? null;
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
          onSelectTab={setActiveTabId}
          onCloseTab={closeTab}
          onAddTab={() => addTab("empty")}
          onClosePanel={onClose}
        />

        <div className="right-panel-body">
          {!activeTab ? (
            <TypePicker
              tab={tabIndex}
              showReader={hasProjectPath}
              onPick={(kind) => addTab(kind)}
            />
          ) : activeTab.kind === "empty" ? (
            <TypePicker
              tab={tabIndex}
              showReader={hasProjectPath}
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
