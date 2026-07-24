import { useTranslation } from "react-i18next";
import { readerFileName } from "../../../../store/reader";
import { FileTypeIcon } from "../../../../utils/fileIcons";
import type { PanelTab } from "../types";
import {
  ChevronLeftIcon,
  ChevronRightIcon,
  CloseIcon,
  FlowIcon,
  GalleryIcon,
  PlusIcon,
  ReaderIcon,
  RoleStateIcon,
} from "./icons";

export interface TabBarProps {
  tabs: PanelTab[];
  activeTabId: string | null;
  galleryCount: number;
  tabIndex: number;
  tabOverflow: { left: boolean; right: boolean };
  tabsScrollRef: React.RefObject<HTMLDivElement | null> | React.MutableRefObject<HTMLDivElement | null>;
  onScroll: () => void;
  onScrollTabs: (dir: -1 | 1) => void;
  onSelectTab: (id: string) => void;
  onCloseTab: (id: string) => void;
  onAddTab: () => void;
  onClosePanel: () => void;
}

function tabLabel(
  kind: PanelTab["kind"],
  t: (key: string) => string,
) {
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
}

function tabTitle(tb: PanelTab, t: (key: string) => string) {
  return tb.kind === "reader" && tb.path ? readerFileName(tb.path) : tabLabel(tb.kind, t);
}

function tabIcon(tb: PanelTab) {
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
}

export function TabBar({
  tabs,
  activeTabId,
  galleryCount,
  tabIndex,
  tabOverflow,
  tabsScrollRef,
  onScroll,
  onScrollTabs,
  onSelectTab,
  onCloseTab,
  onAddTab,
  onClosePanel,
}: TabBarProps) {
  const { t } = useTranslation();

  return (
    <div className="right-panel-tabbar">
      {tabOverflow.left && (
        <button
          type="button"
          className="right-panel-tab-scroll right-panel-tab-scroll--left"
          title={t("rightPanel.scrollLeft")}
          tabIndex={tabIndex}
          onClick={() => onScrollTabs(-1)}
        >
          <ChevronLeftIcon />
        </button>
      )}
      <div
        className="right-panel-tabs"
        role="tablist"
        ref={tabsScrollRef as React.RefObject<HTMLDivElement>}
        onScroll={onScroll}
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
              tabIndex={tabIndex}
              title={tb.kind === "reader" && tb.path ? tb.path : undefined}
              onClick={() => onSelectTab(tb.id)}
            >
              {tabIcon(tb)}
              {tabTitle(tb, t)}
              {tb.kind === "gallery" && galleryCount > 0 && (
                <span className="right-panel-tab-count">{galleryCount}</span>
              )}
            </button>
            <button
              type="button"
              className="right-panel-tab-close"
              title={t("rightPanel.closeTab")}
              tabIndex={tabIndex}
              onClick={() => onCloseTab(tb.id)}
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
          tabIndex={tabIndex}
          onClick={() => onScrollTabs(1)}
        >
          <ChevronRightIcon />
        </button>
      )}
      <div className="right-panel-tabbar-actions">
        <button
          type="button"
          className="ghost-btn"
          title={t("rightPanel.newTab")}
          tabIndex={tabIndex}
          onClick={onAddTab}
        >
          <PlusIcon />
        </button>
        <button
          type="button"
          className="ghost-btn"
          title={t("common.close")}
          tabIndex={tabIndex}
          onClick={onClosePanel}
        >
          <CloseIcon />
        </button>
      </div>
    </div>
  );
}
