import { useEffect, useLayoutEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Sidebar } from "./components/layout/Sidebar";
import { ChatView } from "./components/chat/ChatView";
import { ImageEditor } from "./components/editor/ImageEditor";
import { ImagePreview } from "./components/media/ImagePreview";
import { VideoPreview } from "./components/media/VideoPreview";
import { SettingsView } from "./components/settings";
import type { SettingsTab, ThemeMode } from "./components/settings";
import { UsageView } from "./components/usage";
import { PluginsView } from "./components/plugins";
import { ContextMenuHost } from "./components/context-menu";
import { ToastHost, DialogHost } from "./components/ui";
import { SearchDialog } from "./components/search/SearchDialog";
import { WebSearchDialog } from "./components/search/WebSearchDialog";
import { TitleBar } from "./components/layout/TitleBar";
import { useSettings } from "./store/settings";
import { useSession } from "./store/session";
import { useProject } from "./store/project";
// Side-effect import: binds right-panel / reader / file-tree state to the
// active session for the whole app lifetime.
import "./store/panelBindings";
import {
  THEME_STORAGE_KEY,
  applyThemeMode,
  readStoredThemeMode,
  watchSystemTheme,
} from "./theme";
import { applyAppearance, useAppearance } from "./store/appearance";
import {
  collectSessionGalleryImages,
  indexOfImageInGallery,
} from "./sessionGallery";
import { api } from "./api/tauri";
import type { AttachmentDraft, ImageRefAbs } from "./types";
import {
  useAppHeight,
  useMobileShell,
  useSyncMobileShellAttr,
} from "./hooks/useMobileShell";

type AppRoute =
  | { view: "chat" }
  | { view: "usage" }
  | { view: "plugins" }
  | { view: "settings"; tab: SettingsTab };

const SETTINGS_TABS: SettingsTab[] = [
  "appearance",
  "llm",
  "default",
  "search",
  "tools",
  "system",
];

function parseRoute(): AppRoute {
  const [, view, tab] = window.location.hash.match(/^#\/([^/]+)\/?([^/]*)?/) || [];
  if (view === "usage") {
    return { view: "usage" };
  }
  if (view === "plugins") {
    return { view: "plugins" };
  }
  if (view === "settings" && SETTINGS_TABS.includes(tab as SettingsTab)) {
    return { view: "settings", tab: tab as SettingsTab };
  }
  if (view === "settings") {
    return { view: "settings", tab: "appearance" };
  }
  return { view: "chat" };
}

export default function App() {
  const loadSettings = useSettings((s) => s.load);
  const settings = useSettings((s) => s.settings);
  const refreshList = useSession((s) => s.refreshList);
  const createNew = useSession((s) => s.createNew);
  const refreshProjects = useProject((s) => s.refreshList);
  const setAspectRatio = useSession((s) => s.setAspectRatio);
  const setImageSize = useSession((s) => s.setImageSize);

  const [editorTarget, setEditorTarget] = useState<AttachmentDraft | null>(null);
  const [preview, setPreview] = useState<{ items: ImageRefAbs[]; index: number } | null>(null);
  const [videoPreview, setVideoPreview] = useState<ImageRefAbs | null>(null);
  const [route, setRoute] = useState<AppRoute>(() => parseRoute());
  const [themeMode, setThemeMode] = useState<ThemeMode>(() => readStoredThemeMode());
  const [sidebarCollapsed, setSidebarCollapsed] = useState(false);
  const [mobileDrawerOpen, setMobileDrawerOpen] = useState(false);
  const [searchOpen, setSearchOpen] = useState(false);
  const [webSearchOpen, setWebSearchOpen] = useState(false);
  const isMobile = useMobileShell();
  useSyncMobileShellAttr(isMobile);
  useAppHeight();
  const { t } = useTranslation();

  useEffect(() => {
    loadSettings();
    refreshList();
    refreshProjects();
  }, [loadSettings, refreshList, refreshProjects]);

  useEffect(() => {
    if (!isMobile) setMobileDrawerOpen(false);
  }, [isMobile]);

  useEffect(() => {
    const onHashChange = () => setRoute(parseRoute());
    window.addEventListener("hashchange", onHashChange);
    return () => window.removeEventListener("hashchange", onHashChange);
  }, []);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "k") {
        event.preventDefault();
        if (event.shiftKey) {
          setWebSearchOpen(true);
        } else {
          setSearchOpen(true);
        }
        return;
      }
      if (event.key === "F12") {
        event.preventDefault();
        api.toggleDevtools().catch(console.warn);
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, []);

  useLayoutEffect(() => {
    window.localStorage.setItem(THEME_STORAGE_KEY, themeMode);
    applyThemeMode(themeMode);
    // Re-derive accent palette after theme resolves (light/dark soft mixes differ).
    applyAppearance(useAppearance.getState());

    if (themeMode !== "system") return;
    return watchSystemTheme(() => {
      applyThemeMode("system");
      applyAppearance(useAppearance.getState());
    });
  }, [themeMode]);

  useEffect(() => {
    if (!settings) return;
    if (settings.default_aspect_ratio) setAspectRatio(settings.default_aspect_ratio);
    if (settings.default_image_size) setImageSize(settings.default_image_size);
    // Thinking is per-session: it is restored from the session on switch
    // (see session store `switchTo`), not driven by the global default here.
  }, [settings, setAspectRatio, setImageSize]);

  const activeProvider = settings?.model_services?.find(
    (provider) => provider.id === settings.active_provider_id,
  );
  const activeModel =
    activeProvider && activeProvider.enabled !== false
      ? activeProvider.models.find((model) => model.id === settings?.model)
      : undefined;
  const needsSetup =
    !activeProvider ||
    activeProvider.enabled === false ||
    !activeProvider.api_key?.trim() ||
    !activeProvider.endpoint?.trim() ||
    !activeModel;
  const closeDrawer = () => setMobileDrawerOpen(false);
  const openChat = () => {
    window.location.hash = "#/";
    setRoute({ view: "chat" });
    closeDrawer();
  };
  const openUsage = () => {
    window.location.hash = "#/usage";
    setRoute({ view: "usage" });
    closeDrawer();
  };
  const openPlugins = () => {
    window.location.hash = "#/plugins";
    setRoute({ view: "plugins" });
    closeDrawer();
  };
  const openSettings = (tab: SettingsTab = "appearance") => {
    window.location.hash = `#/settings/${tab}`;
    setRoute({ view: "settings", tab });
    closeDrawer();
  };
  const onNewChat = async () => {
    await createNew();
    openChat();
  };

  return (
    <>
      <div
        className={`app-shell${
          !isMobile && sidebarCollapsed ? " sidebar-collapsed" : ""
        }${isMobile && mobileDrawerOpen ? " drawer-open" : ""}`}
      >
        {!isMobile && (
          <TitleBar
            onToggleSidebar={() => setSidebarCollapsed((v) => !v)}
            sidebarCollapsed={sidebarCollapsed}
            canGoBack={
              route.view === "settings" ||
              route.view === "usage" ||
              route.view === "plugins"
            }
            onBack={openChat}
            onNewChat={onNewChat}
            onOpenSearch={() => setSearchOpen(true)}
            onOpenSettings={() => openSettings("appearance")}
          />
        )}
        <div className="stage">
          {route.view === "settings" ? (
            <SettingsView
              activeTab={route.tab}
              themeMode={themeMode}
              onTabChange={openSettings}
              onThemeModeChange={setThemeMode}
              onBack={openChat}
            />
          ) : (
            <>
              {isMobile && (
                <button
                  type="button"
                  className="drawer-scrim"
                  aria-label={t("common.close")}
                  onClick={closeDrawer}
                />
              )}
              <Sidebar
                onOpenChat={openChat}
                onOpenSearch={() => {
                  setSearchOpen(true);
                  closeDrawer();
                }}
                onOpenSettings={() => openSettings("appearance")}
                onOpenUsage={openUsage}
                onOpenPlugins={openPlugins}
                usageActive={route.view === "usage"}
                pluginsActive={route.view === "plugins"}
                settingsActive={false}
              />
              {route.view === "usage" ? (
                <UsageView onBack={isMobile ? openChat : undefined} />
              ) : route.view === "plugins" ? (
                <PluginsView onBack={isMobile ? openChat : undefined} />
              ) : (
                <ChatView
                  onOpenMenu={
                    isMobile ? () => setMobileDrawerOpen(true) : undefined
                  }
                  onEditAttachment={(a) => setEditorTarget(a)}
                  onPreviewImage={(img: ImageRefAbs) => {
                    if (img.mime.startsWith("video/")) {
                      setVideoPreview(img);
                      return;
                    }
                    const { active: session, sessionMedia } =
                      useSession.getState();
                    const items = collectSessionGalleryImages(
                      session,
                      sessionMedia,
                    );
                    const idx = indexOfImageInGallery(items, img);
                    if (idx >= 0) {
                      setPreview({ items, index: idx });
                    } else {
                      setPreview({ items: [img], index: 0 });
                    }
                  }}
                  onOpenSettings={() => openSettings("llm")}
                  needsSetup={needsSetup}
                />
              )}
            </>
          )}
        </div>
      </div>
      <SearchDialog
        open={searchOpen}
        onClose={() => setSearchOpen(false)}
        onOpenChat={openChat}
      />
      <WebSearchDialog
        open={webSearchOpen}
        onClose={() => setWebSearchOpen(false)}
      />
      {editorTarget && (
        <ImageEditor
          target={editorTarget}
          onClose={() => setEditorTarget(null)}
          onApplied={(newDraft) => {
            useSession.getState().replaceAttachment(editorTarget.image_id, newDraft);
            setEditorTarget(null);
          }}
        />
      )}
      {preview && (
        <ImagePreview
          items={preview.items}
          initialIndex={preview.index}
          onClose={() => setPreview(null)}
        />
      )}
      {videoPreview && (
        <VideoPreview
          item={videoPreview}
          onClose={() => setVideoPreview(null)}
        />
      )}
      <ContextMenuHost />
      <ToastHost />
      <DialogHost />
    </>
  );
}
