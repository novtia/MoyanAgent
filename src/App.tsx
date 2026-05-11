import { useEffect, useLayoutEffect, useState } from "react";
import { Sidebar } from "./components/layout/Sidebar";
import { ChatView } from "./components/chat/ChatView";
import { Dropzone } from "./components/media/Dropzone";
import { ImageEditor } from "./components/editor/ImageEditor";
import { ImagePreview } from "./components/media/ImagePreview";
import { SettingsView } from "./components/settings";
import type { SettingsTab, ThemeMode } from "./components/settings";
import { ContextMenuHost } from "./components/context-menu";
import { SearchDialog } from "./components/search/SearchDialog";
import { TitleBar } from "./components/layout/TitleBar";
import { useSettings } from "./store/settings";
import { useSession } from "./store/session";
import {
  THEME_STORAGE_KEY,
  applyThemeMode,
  readStoredThemeMode,
  watchSystemTheme,
} from "./theme";
import { collectSessionGalleryImages, indexOfImageInGallery } from "./sessionGallery";
import type { AttachmentDraft, ImageRefAbs } from "./types";

type AppRoute =
  | { view: "chat" }
  | { view: "settings"; tab: SettingsTab };

const SETTINGS_TABS: SettingsTab[] = ["appearance", "llm", "system"];

function parseRoute(): AppRoute {
  const [, view, tab] = window.location.hash.match(/^#\/([^/]+)\/?([^/]*)?/) || [];
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
  const setAspectRatio = useSession((s) => s.setAspectRatio);
  const setImageSize = useSession((s) => s.setImageSize);

  const [editorTarget, setEditorTarget] = useState<AttachmentDraft | null>(null);
  const [preview, setPreview] = useState<{ items: ImageRefAbs[]; index: number } | null>(null);
  const [route, setRoute] = useState<AppRoute>(() => parseRoute());
  const [themeMode, setThemeMode] = useState<ThemeMode>(() => readStoredThemeMode());
  const [sidebarCollapsed, setSidebarCollapsed] = useState(false);
  const [searchOpen, setSearchOpen] = useState(false);

  useEffect(() => {
    loadSettings();
    refreshList();
  }, [loadSettings, refreshList]);

  useEffect(() => {
    const onHashChange = () => setRoute(parseRoute());
    window.addEventListener("hashchange", onHashChange);
    return () => window.removeEventListener("hashchange", onHashChange);
  }, []);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "k") {
        event.preventDefault();
        setSearchOpen(true);
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, []);

  useLayoutEffect(() => {
    window.localStorage.setItem(THEME_STORAGE_KEY, themeMode);
    applyThemeMode(themeMode);

    if (themeMode !== "system") return;
    return watchSystemTheme(() => applyThemeMode("system"));
  }, [themeMode]);

  useEffect(() => {
    if (!settings) return;
    if (settings.default_aspect_ratio) setAspectRatio(settings.default_aspect_ratio);
    if (settings.default_image_size) setImageSize(settings.default_image_size);
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
  const openChat = () => {
    window.location.hash = "#/";
    setRoute({ view: "chat" });
  };
  const openSettings = (tab: SettingsTab = "appearance") => {
    window.location.hash = `#/settings/${tab}`;
    setRoute({ view: "settings", tab });
  };

  return (
    <>
      <div className={`app-shell ${sidebarCollapsed ? "sidebar-collapsed" : ""}`}>
        <TitleBar
          onToggleSidebar={() => setSidebarCollapsed((v) => !v)}
          sidebarCollapsed={sidebarCollapsed}
        />
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
              <Sidebar
                onOpenChat={openChat}
                onOpenSearch={() => setSearchOpen(true)}
                onOpenSettings={() => openSettings("appearance")}
                settingsActive={false}
              />
              <ChatView
                onEditAttachment={(a) => setEditorTarget(a)}
                onPreviewImage={(img: ImageRefAbs) => {
                  const session = useSession.getState().active;
                  const items = collectSessionGalleryImages(session);
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
            </>
          )}
        </div>
      </div>
      <Dropzone />
      <SearchDialog
        open={searchOpen}
        onClose={() => setSearchOpen(false)}
        onOpenChat={openChat}
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
      <ContextMenuHost />
    </>
  );
}
