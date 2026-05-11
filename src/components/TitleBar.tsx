import { useEffect, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useTranslation } from "react-i18next";

interface TitleBarProps {
  onToggleSidebar: () => void;
  sidebarCollapsed: boolean;
}

const WIN_ICONS = {
  minimize: "/horizontal-line.svg",
  maximize: "/maximize-button.svg",
  restore: "/Restore%20Down.svg",
  close: "/close.svg",
} as const;

export function TitleBar({ onToggleSidebar, sidebarCollapsed }: TitleBarProps) {
  const { t } = useTranslation();
  const [maximized, setMaximized] = useState(false);

  useEffect(() => {
    const win = getCurrentWindow();
    let unlisten: (() => void) | undefined;
    (async () => {
      try {
        setMaximized(await win.isMaximized());
        unlisten = await win.onResized(async () => {
          setMaximized(await win.isMaximized());
        });
      } catch (e) {
        console.warn(e);
      }
    })();
    return () => {
      if (unlisten) unlisten();
    };
  }, []);

  const onMinimize = () => {
    getCurrentWindow().minimize().catch(console.warn);
  };
  const onToggleMax = () => {
    getCurrentWindow().toggleMaximize().catch(console.warn);
  };
  const onClose = () => {
    getCurrentWindow().close().catch(console.warn);
  };

  return (
    <div className="titlebar" data-tauri-drag-region>
      <div className="titlebar-left">
        <button
          type="button"
          className="titlebar-action"
          aria-label={sidebarCollapsed ? t("titlebar.expandSidebar") : t("titlebar.collapseSidebar")}
          title={sidebarCollapsed ? t("titlebar.expandSidebar") : t("titlebar.collapseSidebar")}
          onClick={onToggleSidebar}
        >
          <SidebarIcon />
        </button>
      </div>
      <div className="titlebar-drag" data-tauri-drag-region>
        <span className="titlebar-title" data-tauri-drag-region>
          {t("titlebar.appName")}
        </span>
      </div>
      <div className="titlebar-controls">
        <button
          type="button"
          className="titlebar-btn"
          aria-label={t("titlebar.minimize")}
          title={t("titlebar.minimize")}
          onClick={onMinimize}
        >
          <img
            className="titlebar-win-icon"
            src={WIN_ICONS.minimize}
            alt=""
            width={11}
            height={11}
            draggable={false}
          />
        </button>
        <button
          type="button"
          className="titlebar-btn"
          aria-label={maximized ? t("titlebar.restore") : t("titlebar.maximize")}
          title={maximized ? t("titlebar.restore") : t("titlebar.maximize")}
          onClick={onToggleMax}
        >
          <img
            className="titlebar-win-icon"
            src={maximized ? WIN_ICONS.restore : WIN_ICONS.maximize}
            alt=""
            width={11}
            height={11}
            draggable={false}
          />
        </button>
        <button
          type="button"
          className="titlebar-btn close"
          aria-label={t("titlebar.close")}
          title={t("titlebar.close")}
          onClick={onClose}
        >
          <img
            className="titlebar-win-icon"
            src={WIN_ICONS.close}
            alt=""
            width={11}
            height={11}
            draggable={false}
          />
        </button>
      </div>
    </div>
  );
}

function SidebarIcon() {
  return (
    <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round">
      <rect x="3" y="4" width="18" height="16" rx="2" />
      <path d="M9 4v16" />
    </svg>
  );
}
