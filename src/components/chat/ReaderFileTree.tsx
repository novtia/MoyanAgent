import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useState,
  type DragEvent as ReactDragEvent,
  type MouseEvent as ReactMouseEvent,
} from "react";
import { useTranslation } from "react-i18next";
import { api } from "../../api/tauri";
import { copyText } from "../../utils/clipboard";
import { READER_FILE_DRAG_TYPE } from "../../utils/readerDrag";
import { FileTypeIcon } from "../../utils/fileIcons";
import { openContextMenu } from "../context-menu";
import { dialog } from "../ui/Dialog";
import { toast } from "../ui/Toast";
import { useProject } from "../../store/project";
import { useSession } from "../../store/session";
import { normalizeReaderPath } from "../../store/reader";
import {
  useFileExplorer,
  baseName,
  isRulesDir,
  joinPath,
  parentDir,
  siblingPath,
} from "../../store/fileExplorer";
import type { ProjectDirEntry } from "../../types";

interface ReaderFileTreeProps {
  activePath?: string | null;
  onOpenFile: (path: string) => void;
}

interface TreeCtx {
  sessionId: string;
  refreshNonce: number;
  activePath: string | null;
  onOpenFile: (path: string) => void;
  refresh: () => void;
  expand: (dir: string) => void;
  newFile: (dir: string) => void;
  newFolder: (dir: string) => void;
}

const TreeContext = createContext<TreeCtx | null>(null);

function useTree(): TreeCtx {
  const ctx = useContext(TreeContext);
  if (!ctx) throw new Error("TreeContext missing");
  return ctx;
}

function splitNameExt(name: string): { base: string; ext: string } {
  const dot = name.lastIndexOf(".");
  if (dot <= 0) return { base: name, ext: "" };
  return { base: name.slice(0, dot), ext: name.slice(dot) };
}

export function ReaderFileTree({ activePath, onOpenFile }: ReaderFileTreeProps) {
  const { t } = useTranslation();
  const activeId = useSession((s) => s.activeId);
  const projectId = useSession((s) => s.active?.session.project_id ?? null);
  const projects = useProject((s) => s.projects);
  const clipboard = useFileExplorer((s) => s.clipboard);
  const setClipboard = useFileExplorer((s) => s.setClipboard);
  const [refreshNonce, setRefreshNonce] = useState(0);
  const [expanded, setExpanded] = useState<Set<string>>(() => new Set());

  const root = useMemo(() => {
    if (!projectId) return null;
    const project = projects.find((p) => p.id === projectId);
    return project?.path?.trim() || null;
  }, [projectId, projects]);

  const refresh = useCallback(() => setRefreshNonce((n) => n + 1), []);
  const expand = useCallback((dir: string) => {
    setExpanded((prev) => {
      if (prev.has(dir)) return prev;
      const next = new Set(prev);
      next.add(dir);
      return next;
    });
  }, []);

  const newFile = useCallback(
    async (dir: string) => {
      if (!activeId) return;
      const name = await dialog.prompt(t("fileExplorer.newFilePrompt"), {
        title: t("fileExplorer.newFile"),
        defaultValue: t("fileExplorer.newFileDefault"),
      });
      if (!name?.trim()) return;
      try {
        await api.createProjectFile(activeId, joinPath(dir, name.trim()), "");
        toast.success(t("fileExplorer.created"));
        expand(dir);
        refresh();
      } catch (err) {
        toast.error(t("fileExplorer.createFailed"), { description: String(err) });
      }
    },
    [activeId, expand, refresh, t],
  );

  const newFolder = useCallback(
    async (dir: string) => {
      if (!activeId) return;
      const name = await dialog.prompt(t("fileExplorer.newFolderPrompt"), {
        title: t("fileExplorer.newFolder"),
        defaultValue: t("fileExplorer.newFolderDefault"),
      });
      if (!name?.trim()) return;
      try {
        await api.createProjectDir(activeId, joinPath(dir, name.trim()));
        toast.success(t("fileExplorer.created"));
        expand(dir);
        refresh();
      } catch (err) {
        toast.error(t("fileExplorer.createFailed"), { description: String(err) });
      }
    },
    [activeId, expand, refresh, t],
  );

  const ctx = useMemo<TreeCtx | null>(() => {
    if (!activeId || !root) return null;
    return {
      sessionId: activeId,
      refreshNonce,
      activePath: activePath ?? null,
      onOpenFile,
      refresh,
      expand,
      newFile,
      newFolder,
    };
  }, [activeId, root, refreshNonce, activePath, onOpenFile, refresh, expand, newFile, newFolder]);

  if (!root || !activeId || !ctx) {
    return (
      <div className="reader-file-tree is-empty">
        <p className="reader-file-tree-status">{t("fileExplorer.noProject")}</p>
      </div>
    );
  }

  return (
    <TreeContext.Provider value={ctx}>
      <div
        className="reader-file-tree"
        onContextMenu={(e) => {
          e.preventDefault();
          openContextMenu(e, [
            { id: "new-file", label: t("fileExplorer.newFile"), onSelect: () => void newFile(root) },
            { id: "new-folder", label: t("fileExplorer.newFolder"), onSelect: () => void newFolder(root) },
            {
              id: "paste",
              label: t("fileExplorer.paste"),
              disabled: !clipboard,
              onSelect: () => void pasteInto(activeId, clipboard, setClipboard, root, refresh, t),
            },
            { type: "separator" },
            { id: "refresh", label: t("fileExplorer.refresh"), onSelect: () => refresh() },
          ]);
        }}
      >
        <div className="reader-file-tree-head reader-file-tree-head--actions">
          <div className="reader-file-tree-actions">
            <button
              type="button"
              className="reader-file-tree-btn"
              title={t("fileExplorer.newFile")}
              onClick={() => void newFile(root)}
            >
              <NewFileIcon />
            </button>
            <button
              type="button"
              className="reader-file-tree-btn"
              title={t("fileExplorer.newFolder")}
              onClick={() => void newFolder(root)}
            >
              <NewFolderIcon />
            </button>
            <button
              type="button"
              className="reader-file-tree-btn"
              title={t("fileExplorer.refresh")}
              onClick={() => refresh()}
            >
              <RefreshIcon />
            </button>
          </div>
        </div>
        <div className="reader-file-tree-body">
          <TreeLevel dirPath={root} depth={0} expanded={expanded} setExpanded={setExpanded} />
        </div>
      </div>
    </TreeContext.Provider>
  );
}

interface TreeLevelProps {
  dirPath: string;
  depth: number;
  expanded: Set<string>;
  setExpanded: React.Dispatch<React.SetStateAction<Set<string>>>;
}

function TreeLevel({ dirPath, depth, expanded, setExpanded }: TreeLevelProps) {
  const { t } = useTranslation();
  const { sessionId, refreshNonce } = useTree();
  const [entries, setEntries] = useState<ProjectDirEntry[] | null>(null);
  const [ruleStates, setRuleStates] = useState<Record<string, boolean>>({});
  const [error, setError] = useState<string | null>(null);
  const indent = 8 + depth * 14;
  const rulesDir = isRulesDir(dirPath);

  useEffect(() => {
    let cancelled = false;
    api
      .listProjectDir(sessionId, dirPath)
      .then((list) => {
        if (!cancelled) {
          setEntries(list);
          setError(null);
        }
      })
      .catch((e) => {
        if (!cancelled) setError(e instanceof Error ? e.message : String(e));
      });
    if (rulesDir) {
      api
        .listProjectRules(sessionId)
        .then((rules) => {
          if (!cancelled) {
            setRuleStates(
              Object.fromEntries(rules.map((r) => [baseName(r.path).toLowerCase(), r.enabled])),
            );
          }
        })
        .catch(() => {
          if (!cancelled) setRuleStates({});
        });
    }
    return () => {
      cancelled = true;
    };
  }, [sessionId, dirPath, refreshNonce, rulesDir]);

  if (error) {
    return (
      <div className="reader-file-tree-status" style={{ paddingLeft: indent }}>
        {error}
      </div>
    );
  }
  if (!entries) {
    return (
      <div className="reader-file-tree-status" style={{ paddingLeft: indent }}>
        {t("fileExplorer.loading")}
      </div>
    );
  }
  if (entries.length === 0) {
    return (
      <div className="reader-file-tree-status" style={{ paddingLeft: indent }}>
        {t("fileExplorer.emptyDir")}
      </div>
    );
  }

  return (
    <>
      {entries.map((entry) => (
        <TreeNode
          key={entry.path}
          entry={entry}
          depth={depth}
          expanded={expanded}
          setExpanded={setExpanded}
          rulesDir={rulesDir}
          ruleEnabled={rulesDir ? ruleStates[entry.name.toLowerCase()] ?? true : true}
        />
      ))}
    </>
  );
}

interface TreeNodeProps {
  entry: ProjectDirEntry;
  depth: number;
  expanded: Set<string>;
  setExpanded: React.Dispatch<React.SetStateAction<Set<string>>>;
  rulesDir: boolean;
  ruleEnabled: boolean;
}

function TreeNode({ entry, depth, expanded, setExpanded, rulesDir, ruleEnabled }: TreeNodeProps) {
  const { t } = useTranslation();
  const tree = useTree();
  const clipboard = useFileExplorer((s) => s.clipboard);
  const setClipboard = useFileExplorer((s) => s.setClipboard);
  const indent = 8 + depth * 14;
  const open = expanded.has(entry.path);
  const isActive =
    !entry.isDir &&
    tree.activePath != null &&
    normalizeReaderPath(entry.path) === normalizeReaderPath(tree.activePath);

  const toggleOpen = useCallback(() => {
    setExpanded((prev) => {
      const next = new Set(prev);
      if (next.has(entry.path)) next.delete(entry.path);
      else next.add(entry.path);
      return next;
    });
  }, [entry.path, setExpanded]);

  const onDragStart = useCallback(
    (e: ReactDragEvent) => {
      e.dataTransfer.effectAllowed = "copy";
      try {
        e.dataTransfer.setData("text/plain", entry.path);
        e.dataTransfer.setData(
          READER_FILE_DRAG_TYPE,
          JSON.stringify([{ path: entry.path, isDir: entry.isDir }]),
        );
      } catch {
        /* ignore */
      }
    },
    [entry.isDir, entry.path],
  );

  const rename = useCallback(async () => {
    const name = await dialog.prompt(t("fileExplorer.renamePrompt"), {
      defaultValue: entry.name,
      title: t("fileExplorer.rename"),
    });
    if (!name?.trim() || name.trim() === entry.name) return;
    try {
      await api.renameProjectPath(tree.sessionId, entry.path, siblingPath(entry.path, name.trim()));
      toast.success(t("fileExplorer.renamed"));
      tree.refresh();
    } catch (err) {
      toast.error(t("fileExplorer.renameFailed"), { description: String(err) });
    }
  }, [entry.name, entry.path, t, tree]);

  const del = useCallback(async () => {
    const message = entry.isDir
      ? t("fileExplorer.deleteFolderConfirm", { name: entry.name })
      : t("fileExplorer.deleteFileConfirm", { name: entry.name });
    const ok = await dialog.confirm(message, {
      type: "danger",
      confirmLabel: t("fileExplorer.delete"),
      title: t("fileExplorer.delete"),
    });
    if (!ok) return;
    try {
      await api.deleteProjectPath(tree.sessionId, entry.path);
      toast.success(t("fileExplorer.deleted"));
      tree.refresh();
    } catch (err) {
      toast.error(t("fileExplorer.deleteFailed"), { description: String(err) });
    }
  }, [entry.isDir, entry.name, entry.path, t, tree]);

  const duplicate = useCallback(async () => {
    const parent = parentDir(entry.path);
    if (!parent) return;
    const { base, ext } = splitNameExt(entry.name);
    try {
      await api.copyProjectPath(
        tree.sessionId,
        entry.path,
        joinPath(parent, `${base} (2)${ext}`),
      );
      toast.success(t("fileExplorer.duplicated"));
      tree.refresh();
    } catch (err) {
      toast.error(t("fileExplorer.duplicateFailed"), { description: String(err) });
    }
  }, [entry.name, entry.path, t, tree]);

  const openMenu = useCallback(
    (e: ReactMouseEvent) => {
      e.preventDefault();
      e.stopPropagation();
      const pasteDir = entry.isDir ? entry.path : parentDir(entry.path) ?? entry.path;
      openContextMenu(e, [
        {
          id: "open",
          label: t("fileExplorer.open"),
          onSelect: () => (entry.isDir ? toggleOpen() : tree.onOpenFile(entry.path)),
        },
        { id: "rename", label: t("fileExplorer.rename"), onSelect: () => void rename() },
        { type: "separator" },
        {
          id: "new-file",
          label: t("fileExplorer.newFile"),
          onSelect: () => void tree.newFile(pasteDir),
        },
        {
          id: "new-folder",
          label: t("fileExplorer.newFolder"),
          onSelect: () => void tree.newFolder(pasteDir),
        },
        { type: "separator" },
        {
          id: "copy",
          label: t("fileExplorer.copy"),
          onSelect: () => {
            setClipboard({ mode: "copy", paths: [entry.path] });
            toast.success(t("fileExplorer.copied"));
          },
        },
        {
          id: "cut",
          label: t("fileExplorer.cut"),
          onSelect: () => {
            setClipboard({ mode: "cut", paths: [entry.path] });
            toast.success(t("fileExplorer.cutToClipboard"));
          },
        },
        {
          id: "paste",
          label: t("fileExplorer.paste"),
          disabled: !clipboard,
          onSelect: () =>
            void pasteInto(tree.sessionId, clipboard, setClipboard, pasteDir, tree.refresh, t),
        },
        { id: "duplicate", label: t("fileExplorer.duplicate"), onSelect: () => void duplicate() },
        { type: "separator" },
        {
          id: "copy-path",
          label: t("fileExplorer.copyPath"),
          onSelect: () => {
            copyText(entry.path)
              .then(() => toast.success(t("fileExplorer.copiedPath")))
              .catch((err) => toast.error(t("fileExplorer.copyFailed"), { description: String(err) }));
          },
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
        { id: "delete", label: t("fileExplorer.delete"), danger: true, onSelect: () => void del() },
      ]);
    },
    [clipboard, del, duplicate, entry.isDir, entry.path, rename, setClipboard, t, toggleOpen, tree],
  );

  if (entry.isDir) {
    return (
      <div className="reader-file-branch" role="treeitem" aria-expanded={open}>
        <button
          type="button"
          className="reader-file-row is-dir"
          style={{ paddingLeft: indent }}
          onClick={toggleOpen}
          onContextMenu={openMenu}
          draggable
          onDragStart={onDragStart}
          title={entry.path}
        >
          <span className={`reader-file-chevron ${open ? "is-open" : ""}`}>
            <ChevronIcon />
          </span>
          <span className="reader-file-name">{entry.name}</span>
        </button>
        {open && (
          <TreeLevel
            dirPath={entry.path}
            depth={depth + 1}
            expanded={expanded}
            setExpanded={setExpanded}
          />
        )}
      </div>
    );
  }

  const isRule = rulesDir && entry.name.toLowerCase().endsWith(".md");

  return (
    <button
      type="button"
      className={`reader-file-row is-file${isActive ? " is-active" : ""}${
        isRule && !ruleEnabled ? " is-rule-disabled" : ""
      }`}
      role="treeitem"
      style={{ paddingLeft: indent }}
      onClick={() => tree.onOpenFile(entry.path)}
      onContextMenu={openMenu}
      draggable
      onDragStart={onDragStart}
      title={entry.path}
    >
      <span className="reader-file-chevron" aria-hidden />
      <FileTypeIcon name={entry.name} className="reader-file-icon" />
      <span className="reader-file-name">{entry.name}</span>
      {isRule && (
        <span
          role="switch"
          aria-checked={ruleEnabled}
          tabIndex={0}
          className={`reader-file-rule-toggle${ruleEnabled ? " is-on" : ""}`}
          title={ruleEnabled ? t("fileExplorer.ruleEnabled") : t("fileExplorer.ruleDisabled")}
          onClick={(e) => {
            e.stopPropagation();
            void toggleRule(tree.sessionId, entry.path, !ruleEnabled, tree.refresh);
          }}
          onKeyDown={(e) => {
            if (e.key === "Enter" || e.key === " ") {
              e.preventDefault();
              e.stopPropagation();
              void toggleRule(tree.sessionId, entry.path, !ruleEnabled, tree.refresh);
            }
          }}
          onDragStart={(e) => {
            e.preventDefault();
            e.stopPropagation();
          }}
        >
          <span className="reader-file-rule-toggle-knob" />
        </span>
      )}
    </button>
  );
}

async function toggleRule(
  sessionId: string,
  path: string,
  enabled: boolean,
  refresh: () => void,
) {
  try {
    await api.setProjectRuleEnabled(sessionId, path, enabled);
    refresh();
  } catch {
    /* ignore */
  }
}

async function pasteInto(
  sessionId: string,
  clipboard: { mode: "copy" | "cut"; paths: string[] } | null,
  setClipboard: (c: { mode: "copy" | "cut"; paths: string[] } | null) => void,
  dir: string,
  refresh: () => void,
  t: (k: string) => string,
) {
  if (!clipboard || clipboard.paths.length === 0) return;
  try {
    for (const from of clipboard.paths) {
      const target = joinPath(dir, baseName(from));
      if (clipboard.mode === "cut") {
        await api.renameProjectPath(sessionId, from, target);
      } else {
        await api.copyProjectPath(sessionId, from, target);
      }
    }
    if (clipboard.mode === "cut") setClipboard(null);
    toast.success(t("fileExplorer.pasted"));
    refresh();
  } catch (err) {
    toast.error(t("fileExplorer.pasteFailed"), { description: String(err) });
  }
}

function ChevronIcon() {
  return (
    <svg viewBox="0 0 24 24" width="12" height="12" fill="none" stroke="currentColor" strokeWidth="2.2" strokeLinecap="round" strokeLinejoin="round" aria-hidden>
      <path d="m9 18 6-6-6-6" />
    </svg>
  );
}

function RefreshIcon() {
  return (
    <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden>
      <path d="M3 12a9 9 0 1 0 9-9 9.75 9.75 0 0 0-6.74 2.74L3 8" />
      <path d="M3 3v5h5" />
    </svg>
  );
}

function NewFileIcon() {
  return (
    <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round" aria-hidden>
      <path d="M14 3H7a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8Z" />
      <path d="M14 3v5h5" />
      <path d="M12 11v6M9 14h6" />
    </svg>
  );
}

function NewFolderIcon() {
  return (
    <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round" aria-hidden>
      <path d="M20 20a2 2 0 0 0 2-2v-8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 4.9A2 2 0 0 0 7.93 4H4a2 2 0 0 0-2 2v12a2 2 0 0 0 2 2Z" />
      <path d="M12 11v5M9.5 13.5h5" />
    </svg>
  );
}
