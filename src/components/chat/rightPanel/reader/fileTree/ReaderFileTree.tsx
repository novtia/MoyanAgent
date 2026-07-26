import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
  type DragEvent as ReactDragEvent,
  type MouseEvent as ReactMouseEvent,
} from "react";
import { useTranslation } from "react-i18next";
import { api } from "../../../../../api/tauri";
import { copyText } from "../../../../../utils/clipboard";
import { READER_FILE_DRAG_TYPE } from "../../../../../utils/readerDrag";
import { FileTypeIcon } from "../../../../../utils/fileIcons";
import { openContextMenu } from "../../../../context-menu";
import { dialog } from "../../../../ui/Dialog";
import { toast } from "../../../../ui/Toast";
import { useProject } from "../../../../../store/project";
import { useSession } from "../../../../../store/session";
import { normalizeReaderPath, useReader } from "../../../../../store/reader";
import {
  useFileExplorer,
  baseName,
  isRulesDir,
  joinPath,
  parentDir,
  siblingPath,
  uniqueName,
} from "../../../../../store/fileExplorer";
import type { ProjectDirEntry } from "../../../../../types";
import type { ReaderFileTreeProps } from "../types";
import { TreeContext, useTree, type TreeCtx } from "./context";
import { ChevronIcon, NewFileIcon, NewFolderIcon, RefreshIcon } from "./icons";
import { pasteInto, toggleRule } from "./ops";
import {
  getVisibleTreeEntries,
  getVisibleTreePaths,
  isModifierClick,
  isPathSelected,
  resolveBulkPaths,
  selectVisibleRange,
  TREE_IS_DIR_ATTR,
  TREE_PATH_ATTR,
} from "./selection";
import { useTreeDropTarget } from "./useTreeDropTarget";
import { useTreeMarquee } from "./useTreeMarquee";

export type { ReaderFileTreeProps } from "../types";

export function ReaderFileTree({ activePath, onOpenFile }: ReaderFileTreeProps) {
  const { t } = useTranslation();
  const activeId = useSession((s) => s.activeId);
  const projectId = useSession((s) => s.active?.session.project_id ?? null);
  const projects = useProject((s) => s.projects);
  const clipboard = useFileExplorer((s) => s.clipboard);
  const setClipboard = useFileExplorer((s) => s.setClipboard);
  const refreshNonce = useFileExplorer((s) => s.treeVersion);
  const bumpTree = useFileExplorer((s) => s.bumpTree);
  const [expanded, setExpanded] = useState<Set<string>>(() => new Set());

  const root = useMemo(() => {
    if (!projectId) return null;
    const project = projects.find((p) => p.id === projectId);
    return project?.path?.trim() || null;
  }, [projectId, projects]);

  const refresh = useCallback(() => bumpTree(), [bumpTree]);
  const bindSession = useFileExplorer((s) => s.bindSession);

  useEffect(() => {
    bindSession(activeId, root);
  }, [activeId, root, bindSession]);

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

  const onImported = useCallback(
    (targetDir: string) => {
      expand(targetDir);
      refresh();
    },
    [expand, refresh],
  );

  const ctx = useMemo<TreeCtx | null>(() => {
    if (!activeId || !root) return null;
    return {
      sessionId: activeId,
      root,
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
      <ReaderFileTreeShell
        root={root}
        sessionId={activeId}
        clipboard={clipboard}
        setClipboard={setClipboard}
        newFile={newFile}
        newFolder={newFolder}
        refresh={refresh}
        onImported={onImported}
        expanded={expanded}
        setExpanded={setExpanded}
      />
    </TreeContext.Provider>
  );
}

interface ReaderFileTreeShellProps {
  root: string;
  sessionId: string;
  clipboard: ReturnType<typeof useFileExplorer.getState>["clipboard"];
  setClipboard: ReturnType<typeof useFileExplorer.getState>["setClipboard"];
  newFile: (dir: string) => Promise<void>;
  newFolder: (dir: string) => Promise<void>;
  refresh: () => void;
  onImported: (targetDir: string) => void;
  expanded: Set<string>;
  setExpanded: React.Dispatch<React.SetStateAction<Set<string>>>;
}

function ReaderFileTreeShell({
  root,
  sessionId,
  clipboard,
  setClipboard,
  newFile,
  newFolder,
  refresh,
  onImported,
  expanded,
  setExpanded,
}: ReaderFileTreeShellProps) {
  const { t } = useTranslation();
  const bodyRef = useRef<HTMLDivElement>(null);
  const clearSelection = useFileExplorer((s) => s.clearSelection);
  const { isDropTarget, dropHandlers } = useTreeDropTarget({
    sessionId,
    root,
    onImported,
  });
  const { marquee, marqueeHandlers } = useTreeMarquee(bodyRef);

  return (
      <div
        className={`reader-file-tree${isDropTarget ? " is-drop-target" : ""}`}
        {...dropHandlers}
        onContextMenu={(e) => {
          e.preventDefault();
          openContextMenu(e, [
            { id: "new-file", label: t("fileExplorer.newFile"), onSelect: () => void newFile(root) },
            { id: "new-folder", label: t("fileExplorer.newFolder"), onSelect: () => void newFolder(root) },
            {
              id: "paste",
              label: t("fileExplorer.paste"),
              disabled: !clipboard,
              onSelect: () => void pasteInto(sessionId, clipboard, setClipboard, root, refresh, t),
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
        <div
          ref={bodyRef}
          className="reader-file-tree-body"
          {...marqueeHandlers}
          onClick={(e) => {
            if (e.target === e.currentTarget) clearSelection();
          }}
        >
          <TreeLevel dirPath={root} depth={0} expanded={expanded} setExpanded={setExpanded} />
          {marquee && (
            <div
              className="reader-tree-marquee"
              style={{
                left: marquee.left,
                top: marquee.top,
                width: marquee.width,
                height: marquee.height,
              }}
            />
          )}
        </div>
      </div>
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
  const selectedPaths = useFileExplorer((s) => s.selectedPaths);
  const setSelection = useFileExplorer((s) => s.setSelection);
  const toggleSelection = useFileExplorer((s) => s.toggleSelection);
  const deleteEntries = useFileExplorer((s) => s.deleteEntries);
  const indent = 8 + depth * 14;
  const open = expanded.has(entry.path);
  const isSelected = isPathSelected(entry.path, selectedPaths);
  const isActive =
    !entry.isDir &&
    tree.activePath != null &&
    normalizeReaderPath(entry.path) === normalizeReaderPath(tree.activePath);

  const onImported = useCallback(
    (targetDir: string) => {
      tree.expand(targetDir);
      tree.refresh();
    },
    [tree],
  );

  const { isDropTarget, dropHandlers } = useTreeDropTarget({
    sessionId: tree.sessionId,
    root: tree.root,
    entryPath: entry.path,
    isDir: entry.isDir,
    onImported,
  });

  const toggleOpen = useCallback(() => {
    setExpanded((prev) => {
      const next = new Set(prev);
      if (next.has(entry.path)) next.delete(entry.path);
      else next.add(entry.path);
      return next;
    });
  }, [entry.path, setExpanded]);

  const onRowClick = useCallback(
    (e: ReactMouseEvent) => {
      e.stopPropagation();
      const body = (e.currentTarget as HTMLElement).closest(
        ".reader-file-tree-body",
      ) as HTMLElement | null;

      if (e.shiftKey) {
        e.preventDefault();
        selectVisibleRange(entry.path, getVisibleTreePaths(body));
        return;
      }
      if (isModifierClick(e)) {
        e.preventDefault();
        toggleSelection(entry.path);
        return;
      }

      setSelection(entry.path);
      if (entry.isDir) toggleOpen();
      else tree.onOpenFile(entry.path);
    },
    [entry.isDir, entry.path, setSelection, toggleOpen, toggleSelection, tree],
  );

  const onDragStart = useCallback(
    (e: ReactDragEvent) => {
      e.dataTransfer.effectAllowed = "copyMove";
      const body = (e.currentTarget as HTMLElement).closest(
        ".reader-file-tree-body",
      ) as HTMLElement | null;
      const bulk = resolveBulkPaths(entry.path, useFileExplorer.getState().selectedPaths);
      const visible = getVisibleTreeEntries(body);
      const byPath = new Map(
        visible.map((v) => [normalizeReaderPath(v.path), v] as const),
      );
      const items = bulk.map((path) => {
        const hit = byPath.get(normalizeReaderPath(path));
        return {
          path,
          isDir: hit?.isDir ?? (path === entry.path ? entry.isDir : false),
        };
      });
      // Ensure the dragged row is selected when starting a multi-drag.
      if (bulk.length > 1) {
        useFileExplorer.getState().setSelectedPaths(bulk);
      } else {
        setSelection(entry.path);
      }
      try {
        e.dataTransfer.setData("text/plain", items.map((i) => i.path).join("\n"));
        e.dataTransfer.setData(READER_FILE_DRAG_TYPE, JSON.stringify(items));
      } catch {
        /* ignore */
      }
    },
    [entry.isDir, entry.path, setSelection],
  );

  const rename = useCallback(async () => {
    const name = await dialog.prompt(t("fileExplorer.renamePrompt"), {
      defaultValue: entry.name,
      title: t("fileExplorer.rename"),
    });
    if (!name?.trim() || name.trim() === entry.name) return;
    try {
      const to = siblingPath(entry.path, name.trim());
      await api.renameProjectPath(tree.sessionId, entry.path, to);
      useReader.getState().remapPath(entry.path, to);
      toast.success(t("fileExplorer.renamed"));
      tree.refresh();
    } catch (err) {
      toast.error(t("fileExplorer.renameFailed"), { description: String(err) });
    }
  }, [entry.name, entry.path, t, tree]);

  const del = useCallback(async () => {
    const paths = resolveBulkPaths(entry.path, useFileExplorer.getState().selectedPaths);
    const message =
      paths.length > 1
        ? t("fileExplorer.deleteSelectedConfirm", { count: paths.length })
        : entry.isDir
          ? t("fileExplorer.deleteFolderConfirm", { name: entry.name })
          : t("fileExplorer.deleteFileConfirm", { name: entry.name });
    const ok = await dialog.confirm(message, {
      type: "danger",
      confirmLabel: t("fileExplorer.delete"),
      title: t("fileExplorer.delete"),
    });
    if (!ok) return;
    try {
      await deleteEntries(paths);
      toast.success(t("fileExplorer.deleted"));
      tree.refresh();
    } catch (err) {
      toast.error(t("fileExplorer.deleteFailed"), { description: String(err) });
    }
  }, [deleteEntries, entry.isDir, entry.name, entry.path, t, tree]);

  const duplicate = useCallback(async () => {
    const paths = resolveBulkPaths(entry.path, useFileExplorer.getState().selectedPaths);
    try {
      const existingByParent = new Map<string, Set<string>>();
      for (const from of paths) {
        const parent = parentDir(from);
        if (!parent) continue;
        let existing = existingByParent.get(parent);
        if (!existing) {
          try {
            const list = await api.listProjectDir(tree.sessionId, parent);
            existing = new Set(list.map((e) => e.name));
          } catch {
            existing = new Set();
          }
          existingByParent.set(parent, existing);
        }
        const finalName = uniqueName(existing, baseName(from));
        existing.add(finalName);
        await api.copyProjectPath(tree.sessionId, from, joinPath(parent, finalName));
      }
      toast.success(t("fileExplorer.duplicated"));
      tree.refresh();
    } catch (err) {
      toast.error(t("fileExplorer.duplicateFailed"), { description: String(err) });
    }
  }, [entry.path, t, tree]);

  const openMenu = useCallback(
    (e: ReactMouseEvent) => {
      e.preventDefault();
      e.stopPropagation();
      const selected = useFileExplorer.getState().selectedPaths;
      if (!isPathSelected(entry.path, selected)) {
        setSelection(entry.path);
      }
      const bulk = resolveBulkPaths(entry.path, useFileExplorer.getState().selectedPaths);
      const pasteDir = entry.isDir ? entry.path : parentDir(entry.path) ?? entry.path;
      openContextMenu(e, [
        {
          id: "open",
          label: t("fileExplorer.open"),
          disabled: bulk.length > 1,
          onSelect: () => (entry.isDir ? toggleOpen() : tree.onOpenFile(entry.path)),
        },
        {
          id: "rename",
          label: t("fileExplorer.rename"),
          disabled: bulk.length > 1,
          onSelect: () => void rename(),
        },
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
            setClipboard({ mode: "copy", paths: bulk });
            toast.success(t("fileExplorer.copied"));
          },
        },
        {
          id: "cut",
          label: t("fileExplorer.cut"),
          onSelect: () => {
            setClipboard({ mode: "cut", paths: bulk });
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
            copyText(bulk.join("\n"))
              .then(() => toast.success(t("fileExplorer.copiedPath")))
              .catch((err) =>
                toast.error(t("fileExplorer.copyFailed"), { description: String(err) }),
              );
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
    [
      clipboard,
      del,
      duplicate,
      entry.isDir,
      entry.path,
      rename,
      setClipboard,
      setSelection,
      t,
      toggleOpen,
      tree,
    ],
  );

  const rowProps = {
    style: { paddingLeft: indent } as CSSProperties,
    onClick: onRowClick,
    onContextMenu: openMenu,
    draggable: true as const,
    onDragStart,
    title: entry.path,
    [TREE_PATH_ATTR]: entry.path,
    [TREE_IS_DIR_ATTR]: entry.isDir ? "1" : "0",
    "aria-selected": isSelected as boolean | "true" | "false",
    ...dropHandlers,
  };

  if (entry.isDir) {
    return (
      <div className="reader-file-branch" role="treeitem" aria-expanded={open}>
        <button
          type="button"
          className={`reader-file-row is-dir${isSelected ? " is-selected" : ""}${
            isDropTarget ? " is-drop-target" : ""
          }`}
          {...rowProps}
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
        isSelected ? " is-selected" : ""
      }${isRule && !ruleEnabled ? " is-rule-disabled" : ""}${
        isDropTarget ? " is-drop-target" : ""
      }`}
      role="treeitem"
      {...rowProps}
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
