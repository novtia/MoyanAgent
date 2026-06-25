import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type DragEvent as ReactDragEvent,
  type KeyboardEvent as ReactKeyboardEvent,
  type MouseEvent as ReactMouseEvent,
} from "react";
import { useTranslation } from "react-i18next";
import { api } from "../../api/tauri";
import { openContextMenu } from "../context-menu";
import { dialog } from "../ui/Dialog";
import { toast } from "../ui/Toast";
import {
  baseName,
  joinPath,
  relativePathSegments,
  useFileExplorer,
} from "../../store/fileExplorer";
import { useProject } from "../../store/project";
import { useSession } from "../../store/session";
import {
  countChars,
  inferFileType,
  useReader,
} from "../../store/reader";
import type { ProjectDirEntry } from "../../types";

const TEXT_EXTENSIONS = [
  ".txt",
  ".md",
  ".markdown",
  ".mdx",
  ".json",
  ".yaml",
  ".yml",
  ".csv",
  ".log",
  ".ini",
  ".toml",
  ".xml",
  ".html",
  ".css",
  ".js",
  ".ts",
  ".tsx",
  ".jsx",
  ".rs",
  ".py",
];

function isReaderTextFile(path: string): boolean {
  const lower = path.toLowerCase();
  return TEXT_EXTENSIONS.some((ext) => lower.endsWith(ext));
}

function pathSep(path: string): string {
  return path.includes("\\") ? "\\" : "/";
}

function RefreshIcon() {
  return (
    <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden>
      <path d="M3 12a9 9 0 1 0 9-9 9.75 9.75 0 0 0-6.74 2.74L3 8" />
      <path d="M3 3v5h5" />
    </svg>
  );
}

function ChevronRightIcon() {
  return (
    <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden>
      <path d="m9 18 6-6-6-6" />
    </svg>
  );
}

function GridIcon() {
  return (
    <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden>
      <rect x="3" y="3" width="7" height="7" />
      <rect x="14" y="3" width="7" height="7" />
      <rect x="14" y="14" width="7" height="7" />
      <rect x="3" y="14" width="7" height="7" />
    </svg>
  );
}

function ListIcon() {
  return (
    <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden>
      <line x1="8" y1="6" x2="21" y2="6" />
      <line x1="8" y1="12" x2="21" y2="12" />
      <line x1="8" y1="18" x2="21" y2="18" />
      <line x1="3" y1="6" x2="3.01" y2="6" />
      <line x1="3" y1="12" x2="3.01" y2="12" />
      <line x1="3" y1="18" x2="3.01" y2="18" />
    </svg>
  );
}

function FolderIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1" strokeLinecap="round" strokeLinejoin="round" aria-hidden className="reader-files-icon-svg">
      <path 
        d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z" 
        fill="color-mix(in srgb, var(--accent) 15%, transparent)"
      />
    </svg>
  );
}

function FileIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1" strokeLinecap="round" strokeLinejoin="round" aria-hidden className="reader-files-icon-svg">
      <path 
        d="M15 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7Z" 
        fill="color-mix(in srgb, var(--ink) 5%, transparent)"
      />
      <path d="M14 2v4a2 2 0 0 0 2 2h4" />
    </svg>
  );
}

export function ReaderFileExplorer() {
  const { t } = useTranslation();
  const activeId = useSession((s) => s.activeId);
  const projectId = useSession((s) => s.active?.session.project_id ?? null);
  const projects = useProject((s) => s.projects);
  const bindSession = useFileExplorer((s) => s.bindSession);
  const projectRoot = useFileExplorer((s) => s.projectRoot);
  const currentDir = useFileExplorer((s) => s.currentDir);
  const entries = useFileExplorer((s) => s.entries);
  const selectedPath = useFileExplorer((s) => s.selectedPath);
  const selectedPaths = useFileExplorer((s) => s.selectedPaths);
  const clipboard = useFileExplorer((s) => s.clipboard);
  const loading = useFileExplorer((s) => s.loading);
  const error = useFileExplorer((s) => s.error);
  const setSelection = useFileExplorer((s) => s.setSelection);
  const toggleSelection = useFileExplorer((s) => s.toggleSelection);
  const selectRange = useFileExplorer((s) => s.selectRange);
  const selectAll = useFileExplorer((s) => s.selectAll);
  const clearSelection = useFileExplorer((s) => s.clearSelection);
  const setClipboard = useFileExplorer((s) => s.setClipboard);
  const navigate = useFileExplorer((s) => s.navigate);
  const navigateUp = useFileExplorer((s) => s.navigateUp);
  const refresh = useFileExplorer((s) => s.refresh);
  const createDir = useFileExplorer((s) => s.createDir);
  const createFile = useFileExplorer((s) => s.createFile);
  const renameEntry = useFileExplorer((s) => s.renameEntry);
  const deleteEntries = useFileExplorer((s) => s.deleteEntries);
  const duplicateEntries = useFileExplorer((s) => s.duplicateEntries);
  const dropMove = useFileExplorer((s) => s.dropMove);
  const pasteInto = useFileExplorer((s) => s.pasteInto);
  const openDoc = useReader((s) => s.openDoc);
  const lastClickRef = useRef<{ path: string; at: number } | null>(null);
  const dragPathsRef = useRef<string[]>([]);

  const [dropTarget, setDropTarget] = useState<string | null>(null);

  const [viewMode, setViewMode] = useState<"grid" | "list">(() => {
    return (localStorage.getItem("reader-file-explorer-view") as "grid" | "list") || "grid";
  });

  const toggleViewMode = useCallback(() => {
    setViewMode((prev) => {
      const next = prev === "grid" ? "list" : "grid";
      localStorage.setItem("reader-file-explorer-view", next);
      return next;
    });
  }, []);

  const resolvedRoot = useMemo(() => {
    if (!projectId) return null;
    const project = projects.find((p) => p.id === projectId);
    const path = project?.path?.trim();
    return path || null;
  }, [projectId, projects]);

  useEffect(() => {
    bindSession(activeId, resolvedRoot);
  }, [activeId, resolvedRoot, bindSession]);

  const entryMap = useMemo(() => {
    const map = new Map<string, ProjectDirEntry>();
    for (const entry of entries) map.set(entry.path, entry);
    return map;
  }, [entries]);

  const cutPaths = useMemo(() => {
    if (!clipboard || clipboard.mode !== "cut") return new Set<string>();
    return new Set(clipboard.paths);
  }, [clipboard]);

  const openEntry = useCallback(
    async (entry: ProjectDirEntry) => {
      if (entry.isDir) {
        await navigate(entry.path);
        return;
      }
      if (!activeId) return;
      if (isReaderTextFile(entry.path)) {
        try {
          const file = await api.readProjectFile(activeId, entry.path);
          openDoc({
            path: entry.path,
            text: file.text,
            fileType: inferFileType(entry.path),
            encoding: file.encoding,
            hadBom: file.hadBom,
            chars: countChars(file.text),
            lines: file.text.split("\n").length,
          });
        } catch (err) {
          toast.error(t("fileExplorer.openFailed"), { description: String(err) });
        }
        return;
      }
      api.openPath(entry.path).catch((err) => {
        toast.error(t("fileExplorer.openFailed"), { description: String(err) });
      });
    },
    [activeId, navigate, openDoc, t],
  );

  const handleRename = useCallback(
    async (entry: ProjectDirEntry) => {
      const name = await dialog.prompt(t("fileExplorer.renamePrompt"), {
        defaultValue: entry.name,
        title: t("fileExplorer.rename"),
      });
      if (!name?.trim() || name.trim() === entry.name) return;
      try {
        await renameEntry(entry.path, name.trim());
        toast.success(t("fileExplorer.renamed"));
      } catch (err) {
        toast.error(t("fileExplorer.renameFailed"), { description: String(err) });
      }
    },
    [renameEntry, t],
  );

  const confirmAndDelete = useCallback(
    async (paths: string[]) => {
      if (paths.length === 0) return;
      let message: string;
      if (paths.length === 1) {
        const entry = entryMap.get(paths[0]);
        const name = entry?.name ?? baseName(paths[0]);
        message = entry?.isDir
          ? t("fileExplorer.deleteFolderConfirm", { name })
          : t("fileExplorer.deleteFileConfirm", { name });
      } else {
        message = t("fileExplorer.deleteSelectedConfirm", { count: paths.length });
      }
      const ok = await dialog.confirm(message, {
        type: "danger",
        confirmLabel: t("fileExplorer.delete"),
        title: t("fileExplorer.delete"),
      });
      if (!ok) return;
      try {
        await deleteEntries(paths);
        toast.success(t("fileExplorer.deleted"));
      } catch (err) {
        toast.error(t("fileExplorer.deleteFailed"), { description: String(err) });
      }
    },
    [deleteEntries, entryMap, t],
  );

  const copyToClipboard = useCallback(
    (paths: string[]) => {
      if (paths.length === 0) return;
      setClipboard({ mode: "copy", paths });
      toast.success(t("fileExplorer.copied"));
    },
    [setClipboard, t],
  );

  const cutToClipboard = useCallback(
    (paths: string[]) => {
      if (paths.length === 0) return;
      setClipboard({ mode: "cut", paths });
      toast.success(t("fileExplorer.cutToClipboard"));
    },
    [setClipboard, t],
  );

  const paste = useCallback(
    async (dir: string | null) => {
      if (!clipboard || !dir) return;
      try {
        await pasteInto(dir);
        toast.success(t("fileExplorer.pasted"));
      } catch (err) {
        toast.error(t("fileExplorer.pasteFailed"), { description: String(err) });
      }
    },
    [clipboard, pasteInto, t],
  );

  const duplicate = useCallback(
    async (paths: string[]) => {
      if (paths.length === 0) return;
      try {
        await duplicateEntries(paths);
        toast.success(t("fileExplorer.duplicated"));
      } catch (err) {
        toast.error(t("fileExplorer.duplicateFailed"), { description: String(err) });
      }
    },
    [duplicateEntries, t],
  );

  const copyPath = useCallback(
    async (path: string) => {
      try {
        await navigator.clipboard.writeText(path);
        toast.success(t("fileExplorer.copiedPath"));
      } catch (err) {
        toast.error(t("fileExplorer.copyFailed"), { description: String(err) });
      }
    },
    [t],
  );

  const handleNewFile = useCallback(async () => {
    const name = await dialog.prompt(t("fileExplorer.newFilePrompt"), {
      title: t("fileExplorer.newFile"),
      defaultValue: t("fileExplorer.newFileDefault"),
    });
    if (!name?.trim()) return;
    try {
      await createFile(name.trim());
      toast.success(t("fileExplorer.created"));
    } catch (err) {
      toast.error(t("fileExplorer.createFailed"), { description: String(err) });
    }
  }, [createFile, t]);

  const handleNewFolder = useCallback(async () => {
    const name = await dialog.prompt(t("fileExplorer.newFolderPrompt"), {
      title: t("fileExplorer.newFolder"),
      defaultValue: t("fileExplorer.newFolderDefault"),
    });
    if (!name?.trim()) return;
    try {
      await createDir(name.trim());
      toast.success(t("fileExplorer.created"));
    } catch (err) {
      toast.error(t("fileExplorer.createFailed"), { description: String(err) });
    }
  }, [createDir, t]);

  const performMove = useCallback(
    async (paths: string[], toDir: string) => {
      if (paths.length === 0) return;
      try {
        const moved = await dropMove(paths, toDir);
        if (moved > 0) toast.success(t("fileExplorer.moved"));
      } catch (err) {
        toast.error(t("fileExplorer.moveFailed"), { description: String(err) });
      }
    },
    [dropMove, t],
  );

  const openEntryMenu = useCallback(
    (event: ReactMouseEvent, entry: ProjectDirEntry) => {
      event.preventDefault();
      event.stopPropagation();
      const targetPaths = selectedPaths.includes(entry.path)
        ? selectedPaths
        : [entry.path];
      if (!selectedPaths.includes(entry.path)) {
        setSelection(entry.path);
      }
      const multi = targetPaths.length > 1;
      openContextMenu(event, [
        {
          id: "open",
          label: t("fileExplorer.open"),
          disabled: multi,
          onSelect: () => void openEntry(entry),
        },
        {
          id: "rename",
          label: t("fileExplorer.rename"),
          disabled: multi,
          onSelect: () => void handleRename(entry),
        },
        { type: "separator" },
        {
          id: "copy",
          label: t("fileExplorer.copy"),
          onSelect: () => copyToClipboard(targetPaths),
        },
        {
          id: "cut",
          label: t("fileExplorer.cut"),
          onSelect: () => cutToClipboard(targetPaths),
        },
        {
          id: "paste",
          label: t("fileExplorer.paste"),
          disabled: !clipboard,
          onSelect: () => void paste(entry.isDir ? entry.path : currentDir),
        },
        {
          id: "duplicate",
          label: t("fileExplorer.duplicate"),
          onSelect: () => void duplicate(targetPaths),
        },
        { type: "separator" },
        {
          id: "copy-path",
          label: t("fileExplorer.copyPath"),
          disabled: multi,
          onSelect: () => void copyPath(entry.path),
        },
        {
          id: "reveal",
          label: t("fileExplorer.reveal"),
          disabled: multi,
          onSelect: () => {
            api.openPath(entry.path).catch((err) => {
              toast.error(t("fileExplorer.openFailed"), { description: String(err) });
            });
          },
        },
        { type: "separator" },
        {
          id: "delete",
          label: t("fileExplorer.delete"),
          danger: true,
          onSelect: () => void confirmAndDelete(targetPaths),
        },
      ]);
    },
    [
      clipboard,
      confirmAndDelete,
      copyPath,
      copyToClipboard,
      currentDir,
      cutToClipboard,
      duplicate,
      handleRename,
      openEntry,
      paste,
      selectedPaths,
      setSelection,
      t,
    ],
  );

  const openBlankMenu = useCallback(
    (event: ReactMouseEvent) => {
      if (!projectRoot) return;
      event.preventDefault();
      openContextMenu(event, [
        {
          id: "new-file",
          label: t("fileExplorer.newFile"),
          onSelect: () => void handleNewFile(),
        },
        {
          id: "new-folder",
          label: t("fileExplorer.newFolder"),
          onSelect: () => void handleNewFolder(),
        },
        {
          id: "paste",
          label: t("fileExplorer.paste"),
          disabled: !clipboard,
          onSelect: () => void paste(currentDir),
        },
        { type: "separator" },
        {
          id: "refresh",
          label: t("fileExplorer.refresh"),
          onSelect: () => void refresh(),
        },
      ]);
    },
    [clipboard, currentDir, handleNewFile, handleNewFolder, paste, projectRoot, refresh, t],
  );

  const onItemClick = useCallback(
    (event: ReactMouseEvent, entry: ProjectDirEntry) => {
      if (event.shiftKey) {
        selectRange(entry.path);
        return;
      }
      if (event.ctrlKey || event.metaKey) {
        toggleSelection(entry.path);
        return;
      }
      setSelection(entry.path);
      const now = Date.now();
      const last = lastClickRef.current;
      if (last && last.path === entry.path && now - last.at < 350) {
        lastClickRef.current = null;
        void openEntry(entry);
        return;
      }
      lastClickRef.current = { path: entry.path, at: now };
    },
    [openEntry, selectRange, setSelection, toggleSelection],
  );

  const onViewKeyDown = useCallback(
    (event: ReactKeyboardEvent) => {
      const mod = event.ctrlKey || event.metaKey;
      const key = event.key.toLowerCase();
      if (mod && key === "a") {
        event.preventDefault();
        selectAll();
        return;
      }
      if (mod && key === "c") {
        event.preventDefault();
        copyToClipboard(selectedPaths);
        return;
      }
      if (mod && key === "x") {
        event.preventDefault();
        cutToClipboard(selectedPaths);
        return;
      }
      if (mod && key === "v") {
        event.preventDefault();
        void paste(currentDir);
        return;
      }
      if (event.key === "Delete") {
        event.preventDefault();
        void confirmAndDelete(selectedPaths);
        return;
      }
      if (event.key === "F2") {
        event.preventDefault();
        const entry = selectedPath ? entryMap.get(selectedPath) : undefined;
        if (entry) void handleRename(entry);
        return;
      }
      if (event.key === "Enter") {
        event.preventDefault();
        const entry = selectedPath ? entryMap.get(selectedPath) : undefined;
        if (entry) void openEntry(entry);
        return;
      }
      if (event.key === "Backspace") {
        event.preventDefault();
        void navigateUp();
      }
    },
    [
      confirmAndDelete,
      copyToClipboard,
      currentDir,
      cutToClipboard,
      entryMap,
      handleRename,
      navigateUp,
      openEntry,
      paste,
      selectAll,
      selectedPath,
      selectedPaths,
    ],
  );

  const onItemDragStart = useCallback(
    (event: ReactDragEvent, entry: ProjectDirEntry) => {
      const paths = selectedPaths.includes(entry.path)
        ? selectedPaths
        : [entry.path];
      if (!selectedPaths.includes(entry.path)) {
        setSelection(entry.path);
      }
      dragPathsRef.current = paths;
      event.dataTransfer.effectAllowed = "move";
      try {
        event.dataTransfer.setData("text/plain", paths.join("\n"));
      } catch {
        /* ignore */
      }
    },
    [selectedPaths, setSelection],
  );

  const onItemDragEnd = useCallback(() => {
    dragPathsRef.current = [];
    setDropTarget(null);
  }, []);

  const onFolderDragOver = useCallback(
    (event: ReactDragEvent, entry: ProjectDirEntry) => {
      if (!entry.isDir) return;
      if (dragPathsRef.current.length === 0) return;
      if (dragPathsRef.current.includes(entry.path)) return;
      event.preventDefault();
      event.stopPropagation();
      event.dataTransfer.dropEffect = "move";
      setDropTarget(entry.path);
    },
    [],
  );

  const onFolderDrop = useCallback(
    (event: ReactDragEvent, entry: ProjectDirEntry) => {
      if (!entry.isDir) return;
      const paths = dragPathsRef.current;
      if (paths.length === 0) return;
      event.preventDefault();
      event.stopPropagation();
      setDropTarget(null);
      dragPathsRef.current = [];
      void performMove(paths, entry.path);
    },
    [performMove],
  );

  const onCrumbDragOver = useCallback((event: ReactDragEvent, dir: string) => {
    if (dragPathsRef.current.length === 0) return;
    event.preventDefault();
    event.dataTransfer.dropEffect = "move";
    setDropTarget(dir);
  }, []);

  const onCrumbDrop = useCallback(
    (event: ReactDragEvent, dir: string) => {
      const paths = dragPathsRef.current;
      if (paths.length === 0) return;
      event.preventDefault();
      setDropTarget(null);
      dragPathsRef.current = [];
      void performMove(paths, dir);
    },
    [performMove],
  );

  const onViewDragOver = useCallback(
    (event: ReactDragEvent) => {
      if (dragPathsRef.current.length === 0) return;
      if (!currentDir) return;
      event.preventDefault();
      event.dataTransfer.dropEffect = "move";
      setDropTarget("__view__");
    },
    [currentDir],
  );

  const onViewDrop = useCallback(
    (event: ReactDragEvent) => {
      const paths = dragPathsRef.current;
      if (paths.length === 0 || !currentDir) return;
      event.preventDefault();
      setDropTarget(null);
      dragPathsRef.current = [];
      void performMove(paths, currentDir);
    },
    [currentDir, performMove],
  );

  const onViewClick = useCallback(
    (event: ReactMouseEvent) => {
      if (event.target === event.currentTarget) clearSelection();
    },
    [clearSelection],
  );

  const breadcrumbSegments = useMemo(() => {
    if (!projectRoot || !currentDir) return [];
    return relativePathSegments(projectRoot, currentDir);
  }, [projectRoot, currentDir]);

  if (!projectRoot) {
    return (
      <div className="reader-files-explorer is-empty">
        <p className="reader-files-empty">{t("fileExplorer.noProject")}</p>
      </div>
    );
  }

  return (
    <div className="reader-files-explorer" onContextMenu={openBlankMenu}>
      <div className="reader-files-toolbar">
        <nav className="reader-files-breadcrumb" aria-label={t("fileExplorer.breadcrumb")}>
          <button
            type="button"
            className={`reader-files-crumb${dropTarget === projectRoot ? " is-drop-target" : ""}`}
            onClick={() => void navigate(projectRoot)}
            onDragOver={(e) => onCrumbDragOver(e, projectRoot)}
            onDragLeave={() => setDropTarget((p) => (p === projectRoot ? null : p))}
            onDrop={(e) => onCrumbDrop(e, projectRoot)}
          >
            {t("fileExplorer.root")}
          </button>
          {breadcrumbSegments.map((seg, idx) => {
            const dir = joinPath(
              projectRoot,
              breadcrumbSegments.slice(0, idx + 1).join(pathSep(projectRoot)),
            );
            return (
              <span key={dir} className="reader-files-crumb-wrap">
                <span className="reader-files-crumb-sep"><ChevronRightIcon /></span>
                <button
                  type="button"
                  className={`reader-files-crumb${dropTarget === dir ? " is-drop-target" : ""}`}
                  onClick={() => void navigate(dir)}
                  onDragOver={(e) => onCrumbDragOver(e, dir)}
                  onDragLeave={() => setDropTarget((p) => (p === dir ? null : p))}
                  onDrop={(e) => onCrumbDrop(e, dir)}
                >
                  {seg}
                </button>
              </span>
            );
          })}
        </nav>
        <div className="reader-files-toolbar-actions">
          <button
            type="button"
            className="reader-files-toolbar-btn"
            title={viewMode === "grid" ? t("fileExplorer.listView") : t("fileExplorer.gridView")}
            onClick={toggleViewMode}
          >
            {viewMode === "grid" ? <ListIcon /> : <GridIcon />}
          </button>
          <button
            type="button"
            className="reader-files-toolbar-btn"
            title={t("fileExplorer.refresh")}
            onClick={() => void refresh()}
          >
            <RefreshIcon />
          </button>
        </div>
      </div>

      {loading ? (
        <p className="reader-files-status">{t("fileExplorer.loading")}</p>
      ) : error ? (
        <p className="reader-files-status reader-files-status-error">{error}</p>
      ) : entries.length === 0 ? (
        <p className="reader-files-status">{t("fileExplorer.emptyDir")}</p>
      ) : (
        <div
          className={`reader-files-view is-${viewMode}${dropTarget === "__view__" ? " is-drop-target" : ""}`}
          role="list"
          tabIndex={0}
          onKeyDown={onViewKeyDown}
          onClick={onViewClick}
          onDragOver={onViewDragOver}
          onDragLeave={(e) => {
            if (e.target === e.currentTarget) setDropTarget((p) => (p === "__view__" ? null : p));
          }}
          onDrop={onViewDrop}
        >
          {entries.map((entry) => {
            const selected = selectedPaths.includes(entry.path);
            const isCut = cutPaths.has(entry.path);
            const isDropTarget = dropTarget === entry.path;
            return (
              <button
                key={entry.path}
                type="button"
                role="listitem"
                draggable
                className={`reader-files-item${selected ? " is-selected" : ""}${isCut ? " is-cut" : ""}${isDropTarget ? " is-drop-target" : ""}`}
                onClick={(e) => onItemClick(e, entry)}
                onContextMenu={(e) => openEntryMenu(e, entry)}
                onDragStart={(e) => onItemDragStart(e, entry)}
                onDragEnd={onItemDragEnd}
                onDragOver={(e) => onFolderDragOver(e, entry)}
                onDragLeave={() => setDropTarget((p) => (p === entry.path ? null : p))}
                onDrop={(e) => onFolderDrop(e, entry)}
                title={entry.path}
              >
                <span className="reader-files-item-icon">
                  {entry.isDir ? <FolderIcon /> : <FileIcon />}
                </span>
                <span className="reader-files-item-name">{entry.name}</span>
              </button>
            );
          })}
        </div>
      )}
    </div>
  );
}
