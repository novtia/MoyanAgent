import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  type MouseEvent as ReactMouseEvent,
} from "react";
import { useTranslation } from "react-i18next";
import { api } from "../../api/tauri";
import { openContextMenu } from "../context-menu";
import { dialog } from "../ui/Dialog";
import { toast } from "../ui/Toast";
import {
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

function FolderIcon() {
  return (
    <svg viewBox="0 0 48 48" fill="none" aria-hidden className="reader-files-icon-svg">
      <path
        d="M8 14h12l3 3h19v21a2 2 0 0 1-2 2H10a2 2 0 0 1-2-2V14z"
        fill="color-mix(in srgb, var(--accent) 12%, var(--surface))"
        stroke="currentColor"
        strokeWidth="1.5"
      />
      <path d="M8 17h32" stroke="currentColor" strokeWidth="1.5" opacity="0.35" />
    </svg>
  );
}

function FileIcon() {
  return (
    <svg viewBox="0 0 48 48" fill="none" aria-hidden className="reader-files-icon-svg">
      <path
        d="M14 6h14l10 10v26a2 2 0 0 1-2 2H14a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2z"
        fill="var(--surface)"
        stroke="currentColor"
        strokeWidth="1.5"
      />
      <path d="M28 6v10h10" stroke="currentColor" strokeWidth="1.5" />
      <path
        d="M18 26h12M18 31h12M18 36h8"
        stroke="currentColor"
        strokeWidth="1.5"
        strokeLinecap="round"
        opacity="0.35"
      />
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
  const loading = useFileExplorer((s) => s.loading);
  const error = useFileExplorer((s) => s.error);
  const setSelectedPath = useFileExplorer((s) => s.setSelectedPath);
  const navigate = useFileExplorer((s) => s.navigate);
  const refresh = useFileExplorer((s) => s.refresh);
  const createDir = useFileExplorer((s) => s.createDir);
  const createFile = useFileExplorer((s) => s.createFile);
  const renameEntry = useFileExplorer((s) => s.renameEntry);
  const deleteEntry = useFileExplorer((s) => s.deleteEntry);
  const openDoc = useReader((s) => s.openDoc);
  const lastClickRef = useRef<{ path: string; at: number } | null>(null);

  const resolvedRoot = useMemo(() => {
    if (!projectId) return null;
    const project = projects.find((p) => p.id === projectId);
    const path = project?.path?.trim();
    return path || null;
  }, [projectId, projects]);

  useEffect(() => {
    bindSession(activeId, resolvedRoot);
  }, [activeId, resolvedRoot, bindSession]);

  const openEntry = useCallback(
    async (entry: ProjectDirEntry) => {
      if (entry.isDir) {
        await navigate(entry.path);
        return;
      }
      if (!activeId) return;
      if (isReaderTextFile(entry.path)) {
        try {
          const text = await api.readProjectFile(activeId, entry.path);
          openDoc({
            path: entry.path,
            text,
            fileType: inferFileType(entry.path),
            chars: countChars(text),
            lines: text.split("\n").length,
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

  const handleDelete = useCallback(
    async (entry: ProjectDirEntry) => {
      const ok = await dialog.confirm(
        entry.isDir
          ? t("fileExplorer.deleteFolderConfirm", { name: entry.name })
          : t("fileExplorer.deleteFileConfirm", { name: entry.name }),
        {
          type: "danger",
          confirmLabel: t("fileExplorer.delete"),
          title: t("fileExplorer.delete"),
        },
      );
      if (!ok) return;
      try {
        await deleteEntry(entry.path);
        toast.success(t("fileExplorer.deleted"));
      } catch (err) {
        toast.error(t("fileExplorer.deleteFailed"), { description: String(err) });
      }
    },
    [deleteEntry, t],
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

  const openEntryMenu = useCallback(
    (event: ReactMouseEvent, entry: ProjectDirEntry) => {
      event.preventDefault();
      event.stopPropagation();
      setSelectedPath(entry.path);
      openContextMenu(event, [
        {
          id: "open",
          label: t("fileExplorer.open"),
          onSelect: () => void openEntry(entry),
        },
        {
          id: "rename",
          label: t("fileExplorer.rename"),
          onSelect: () => void handleRename(entry),
        },
        {
          id: "copy-path",
          label: t("fileExplorer.copyPath"),
          onSelect: () => void copyPath(entry.path),
        },
        {
          id: "reveal",
          label: t("fileExplorer.reveal"),
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
          onSelect: () => void handleDelete(entry),
        },
      ]);
    },
    [copyPath, handleDelete, handleRename, openEntry, setSelectedPath, t],
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
        { type: "separator" },
        {
          id: "refresh",
          label: t("fileExplorer.refresh"),
          onSelect: () => void refresh(),
        },
      ]);
    },
    [handleNewFile, handleNewFolder, projectRoot, refresh, t],
  );

  const onItemClick = useCallback(
    (entry: ProjectDirEntry) => {
      setSelectedPath(entry.path);
      const now = Date.now();
      const last = lastClickRef.current;
      if (last && last.path === entry.path && now - last.at < 350) {
        lastClickRef.current = null;
        void openEntry(entry);
        return;
      }
      lastClickRef.current = { path: entry.path, at: now };
    },
    [openEntry, setSelectedPath],
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
            className="reader-files-crumb"
            onClick={() => void navigate(projectRoot)}
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
                <span className="reader-files-crumb-sep">/</span>
                <button
                  type="button"
                  className="reader-files-crumb"
                  onClick={() => void navigate(dir)}
                >
                  {seg}
                </button>
              </span>
            );
          })}
        </nav>
        <button
          type="button"
          className="reader-files-refresh-btn"
          title={t("fileExplorer.refresh")}
          onClick={() => void refresh()}
        >
          {t("fileExplorer.refresh")}
        </button>
      </div>

      {loading ? (
        <p className="reader-files-status">{t("fileExplorer.loading")}</p>
      ) : error ? (
        <p className="reader-files-status reader-files-status-error">{error}</p>
      ) : entries.length === 0 ? (
        <p className="reader-files-status">{t("fileExplorer.emptyDir")}</p>
      ) : (
        <div className="reader-files-grid" role="list">
          {entries.map((entry) => {
            const selected = selectedPath === entry.path;
            return (
              <button
                key={entry.path}
                type="button"
                role="listitem"
                className={`reader-files-grid-item${selected ? " is-selected" : ""}`}
                onClick={() => onItemClick(entry)}
                onContextMenu={(e) => openEntryMenu(e, entry)}
                title={entry.path}
              >
                <span className="reader-files-grid-icon">
                  {entry.isDir ? <FolderIcon /> : <FileIcon />}
                </span>
                <span className="reader-files-grid-name">{entry.name}</span>
              </button>
            );
          })}
        </div>
      )}
    </div>
  );
}
