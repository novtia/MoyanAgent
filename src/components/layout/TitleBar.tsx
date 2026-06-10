import { useEffect, useRef, useState, type ReactNode } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useTranslation } from "react-i18next";
import { api } from "../../api/tauri";

interface TitleBarProps {
  onToggleSidebar: () => void;
  sidebarCollapsed: boolean;
  canGoBack?: boolean;
  canGoForward?: boolean;
  onBack?: () => void;
  onForward?: () => void;
  onNewChat?: () => void;
  onOpenSearch?: () => void;
  onOpenSettings?: () => void;
}

const WIN_ICONS = {
  minimize: "/horizontal-line.svg",
  maximize: "/maximize-button.svg",
  restore: "/Restore%20Down.svg",
  close: "/close.svg",
} as const;

type MenuItem =
  | { type: "action"; label: string; onClick: () => void; disabled?: boolean }
  | { type: "separator" };

export function TitleBar({
  onToggleSidebar,
  sidebarCollapsed,
  canGoBack = false,
  canGoForward = false,
  onBack,
  onForward,
  onNewChat,
  onOpenSearch,
  onOpenSettings,
}: TitleBarProps) {
  const { t } = useTranslation();
  const [maximized, setMaximized] = useState(false);
  const [openMenu, setOpenMenu] = useState<string | null>(null);
  const menusRef = useRef<HTMLDivElement>(null);

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

  useEffect(() => {
    if (!openMenu) return;
    const onPointerDown = (event: PointerEvent) => {
      if (menusRef.current?.contains(event.target as Node)) return;
      setOpenMenu(null);
    };
    window.addEventListener("pointerdown", onPointerDown);
    return () => window.removeEventListener("pointerdown", onPointerDown);
  }, [openMenu]);

  const onMinimize = () => {
    getCurrentWindow().minimize().catch(console.warn);
  };
  const onToggleMax = () => {
    getCurrentWindow().toggleMaximize().catch(console.warn);
  };
  const onClose = () => {
    getCurrentWindow().close().catch(console.warn);
  };

  const closeMenu = () => setOpenMenu(null);

  const menuDefs: { id: string; label: string; items: MenuItem[] }[] = [
    {
      id: "file",
      label: t("titlebar.menu.file"),
      items: [
        { type: "action", label: t("titlebar.menu.newChat"), onClick: () => { onNewChat?.(); closeMenu(); } },
        { type: "action", label: t("titlebar.menu.settings"), onClick: () => { onOpenSettings?.(); closeMenu(); } },
        { type: "separator" },
        { type: "action", label: t("titlebar.menu.quit"), onClick: () => { closeMenu(); onClose(); } },
      ],
    },
    {
      id: "edit",
      label: t("titlebar.menu.edit"),
      items: [
        { type: "action", label: t("titlebar.menu.search"), onClick: () => { onOpenSearch?.(); closeMenu(); } },
      ],
    },
    {
      id: "view",
      label: t("titlebar.menu.view"),
      items: [
        {
          type: "action",
          label: sidebarCollapsed ? t("titlebar.expandSidebar") : t("titlebar.collapseSidebar"),
          onClick: () => { onToggleSidebar(); closeMenu(); },
        },
        { type: "action", label: t("titlebar.menu.search"), onClick: () => { onOpenSearch?.(); closeMenu(); } },
        {
          type: "action",
          label: t("titlebar.menu.devtools"),
          onClick: () => {
            api.toggleDevtools().catch(console.warn);
            closeMenu();
          },
        },
      ],
    },
    {
      id: "window",
      label: t("titlebar.menu.window"),
      items: [
        { type: "action", label: t("titlebar.minimize"), onClick: () => { onMinimize(); closeMenu(); } },
        {
          type: "action",
          label: maximized ? t("titlebar.restore") : t("titlebar.maximize"),
          onClick: () => { onToggleMax(); closeMenu(); },
        },
      ],
    },
    {
      id: "help",
      label: t("titlebar.menu.help"),
      items: [
        { type: "action", label: t("titlebar.appName"), onClick: closeMenu, disabled: true },
      ],
    },
  ];

  return (
    <div className="titlebar" data-tauri-drag-region>
      <div className="titlebar-left">
        <div className="titlebar-actions">
          <button
            type="button"
            className="titlebar-action"
            aria-label={sidebarCollapsed ? t("titlebar.expandSidebar") : t("titlebar.collapseSidebar")}
            title={sidebarCollapsed ? t("titlebar.expandSidebar") : t("titlebar.collapseSidebar")}
            onClick={onToggleSidebar}
          >
            <SidebarIcon />
          </button>
          <button
            type="button"
            className="titlebar-action"
            aria-label={t("common.back")}
            title={t("common.back")}
            disabled={!canGoBack}
            onClick={onBack}
          >
            <BackIcon />
          </button>
          <button
            type="button"
            className="titlebar-action"
            aria-label={t("titlebar.forward")}
            title={t("titlebar.forward")}
            disabled={!canGoForward}
            onClick={onForward}
          >
            <ForwardIcon />
          </button>
        </div>
        <nav className="titlebar-menus" ref={menusRef} aria-label={t("titlebar.menuBar")}>
          {menuDefs.map((menu) => (
            <TitleBarMenu
              key={menu.id}
              label={menu.label}
              open={openMenu === menu.id}
              onToggle={() => setOpenMenu((current) => (current === menu.id ? null : menu.id))}
            >
              {menu.items.map((item, index) =>
                item.type === "separator" ? (
                  <div key={`sep-${index}`} className="titlebar-menu-separator" role="separator" />
                ) : (
                  <button
                    key={item.label}
                    type="button"
                    className="titlebar-menu-item"
                    role="menuitem"
                    disabled={item.disabled}
                    onClick={item.onClick}
                  >
                    {item.label}
                  </button>
                ),
              )}
            </TitleBarMenu>
          ))}
        </nav>
      </div>
      <div className="titlebar-drag" data-tauri-drag-region />
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

function TitleBarMenu({
  label,
  open,
  onToggle,
  children,
}: {
  label: string;
  open: boolean;
  onToggle: () => void;
  children: ReactNode;
}) {
  return (
    <div className="titlebar-menu">
      <button
        type="button"
        className="titlebar-menu-btn"
        aria-haspopup="menu"
        aria-expanded={open}
        onClick={onToggle}
      >
        {label}
      </button>
      {open && (
        <div className="titlebar-menu-dropdown" role="menu">
          {children}
        </div>
      )}
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

function BackIcon() {
  return (
    <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round">
      <path d="M15 18l-6-6 6-6" />
    </svg>
  );
}

function ForwardIcon() {
  return (
    <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round">
      <path d="M9 18l6-6-6-6" />
    </svg>
  );
}
