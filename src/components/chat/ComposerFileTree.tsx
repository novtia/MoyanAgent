import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { api } from "../../api/tauri";
import { useFileExplorer } from "../../store/fileExplorer";
import type { ProjectDirEntry } from "../../types";

export interface ComposerFileTreeProps {
  sessionId: string;
  projectRoot: string;
  onPick: (absPath: string, isDir: boolean) => void;
}

/** Lazily-loaded, expandable project file tree used by the @ mention panel. */
export function ComposerFileTree({ sessionId, projectRoot, onPick }: ComposerFileTreeProps) {
  return (
    <div className="composer-mention-tree" role="tree">
      <TreeLevel sessionId={sessionId} dirPath={projectRoot} depth={0} onPick={onPick} />
    </div>
  );
}

interface TreeLevelProps {
  sessionId: string;
  dirPath: string;
  depth: number;
  onPick: (absPath: string, isDir: boolean) => void;
}

function TreeLevel({ sessionId, dirPath, depth, onPick }: TreeLevelProps) {
  const { t } = useTranslation();
  const treeVersion = useFileExplorer((s) => s.treeVersion);
  const [entries, setEntries] = useState<ProjectDirEntry[] | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setError(null);
    api
      .listProjectDir(sessionId, dirPath)
      .then((list) => {
        if (!cancelled) {
          setEntries(list);
          setLoading(false);
        }
      })
      .catch((e) => {
        if (!cancelled) {
          setError(e instanceof Error ? e.message : String(e));
          setLoading(false);
        }
      });
    return () => {
      cancelled = true;
    };
  }, [sessionId, dirPath, treeVersion]);

  const indent = 8 + depth * 14;

  if (loading) {
    return (
      <div className="composer-mention-status" style={{ paddingLeft: indent }}>
        {t("fileExplorer.loading")}
      </div>
    );
  }
  if (error) {
    return (
      <div className="composer-mention-status" style={{ paddingLeft: indent }}>
        {error}
      </div>
    );
  }
  if (!entries || entries.length === 0) {
    return (
      <div className="composer-mention-status" style={{ paddingLeft: indent }}>
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
          sessionId={sessionId}
          depth={depth}
          onPick={onPick}
        />
      ))}
    </>
  );
}

interface TreeNodeProps {
  entry: ProjectDirEntry;
  sessionId: string;
  depth: number;
  onPick: (absPath: string, isDir: boolean) => void;
}

function TreeNode({ entry, sessionId, depth, onPick }: TreeNodeProps) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const indent = 8 + depth * 14;

  if (entry.isDir) {
    return (
      <div className="composer-mention-branch" role="treeitem" aria-expanded={open}>
        <div className="composer-mention-rowwrap">
          <button
            type="button"
            className="composer-mention-row"
            style={{ paddingLeft: indent }}
            onClick={() => setOpen((o) => !o)}
            title={entry.path}
          >
            <span className={`composer-mention-chevron ${open ? "is-open" : ""}`}>
              <ChevronIcon />
            </span>
            <FolderIcon />
            <span className="composer-mention-name">{entry.name}</span>
          </button>
          <button
            type="button"
            className="composer-mention-ref"
            title={t("composer.referenceFolder")}
            aria-label={t("composer.referenceFolder")}
            onClick={() => onPick(entry.path, true)}
          >
            <RefIcon />
          </button>
        </div>
        {open && (
          <TreeLevel
            sessionId={sessionId}
            dirPath={entry.path}
            depth={depth + 1}
            onPick={onPick}
          />
        )}
      </div>
    );
  }

  return (
    <button
      type="button"
      className="composer-mention-row is-file"
      role="treeitem"
      style={{ paddingLeft: indent }}
      onClick={() => onPick(entry.path, false)}
      title={entry.path}
    >
      <span className="composer-mention-chevron" aria-hidden />
      <FileIcon />
      <span className="composer-mention-name">{entry.name}</span>
    </button>
  );
}

function ChevronIcon() {
  return (
    <svg viewBox="0 0 24 24" width="12" height="12" fill="none" stroke="currentColor" strokeWidth="2.2" strokeLinecap="round" strokeLinejoin="round" aria-hidden>
      <path d="m9 18 6-6-6-6" />
    </svg>
  );
}

function FolderIcon() {
  return (
    <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round" aria-hidden className="composer-mention-ficon">
      <path d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z" />
    </svg>
  );
}

function RefIcon() {
  return (
    <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round" aria-hidden>
      <path d="M12 5v14" />
      <path d="M5 12h14" />
    </svg>
  );
}

function FileIcon() {
  return (
    <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round" aria-hidden className="composer-mention-ficon">
      <path d="M15 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7Z" />
      <path d="M14 2v4a2 2 0 0 0 2 2h4" />
    </svg>
  );
}
