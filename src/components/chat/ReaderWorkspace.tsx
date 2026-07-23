import {
  useCallback,
  useEffect,
  useMemo,
  useReducer,
  useRef,
  useState,
  type MouseEvent as ReactMouseEvent,
} from "react";
import { useTranslation } from "react-i18next";
import { api } from "../../api/tauri";
import { copyText } from "../../utils/clipboard";
import { openContextMenu } from "../context-menu";
import { toast } from "../ui/Toast";
import {
  useReader,
  applyReaderPathOpsToPath,
  countWords,
  inferFileType,
  normalizeReaderPath,
  readerFileName,
  type ReaderFileTab,
} from "../../store/reader";
import { useSession } from "../../store/session";
import { useReaderFind } from "../../store/readerFind";
import { ReaderEditor } from "./ReaderEditor";
import { ReaderFileTree } from "./ReaderFileTree";
import { ReaderFindBar, useReaderFindShortcuts } from "./ReaderFindBar";
import { ReaderMarkdownPreview } from "./ReaderMarkdownPreview";
import { ReaderDiffHeaderBar } from "./ReaderDiffHeaderBar";

const RATIO_KEY = "atelier:reader-split-ratio";
const SHOW_TREE_KEY = "atelier:reader-split";
const MIN_RATIO = 0.25;
const MAX_RATIO = 0.8;

function readStoredRatio(): number {
  try {
    const raw = window.localStorage.getItem(RATIO_KEY);
    const n = raw ? Number.parseFloat(raw) : NaN;
    if (!Number.isFinite(n)) return 0.58;
    return Math.min(MAX_RATIO, Math.max(MIN_RATIO, n));
  } catch {
    return 0.58;
  }
}

function readStoredShowTree(): boolean {
  if (typeof window === "undefined") return true;
  return window.localStorage.getItem(SHOW_TREE_KEY) !== "0";
}

type RightView = "tree" | "search";

interface ReaderWorkspaceProps {
  path?: string | null;
  onOpenFile: (path: string) => void;
}

/** Editor pane body: rendered markdown preview or the source/diff editor. */
function ReaderFilePane({ tab, preview }: { tab: ReaderFileTab; preview: boolean }) {
  const [activeHunkIndex, setActiveHunkIndex] = useState(0);
  const hasPendingDiff = tab.pendingDiffs.length > 0;

  useEffect(() => {
    setActiveHunkIndex(0);
  }, [tab.id, tab.pendingDiffs.length]);

  const navigateHunk = useCallback(
    (direction: -1 | 1) => {
      setActiveHunkIndex((prev) => {
        const total = tab.pendingDiffs.length;
        if (total === 0) return 0;
        return Math.max(0, Math.min(prev + direction, total - 1));
      });
    },
    [tab.pendingDiffs.length],
  );

  if (preview && tab.fileType === "markdown" && !hasPendingDiff) {
    return (
      <div className="document-reader reader-file-pane">
        <div className="document-reader-body reader-file-body">
          <ReaderMarkdownPreview text={tab.text} />
        </div>
      </div>
    );
  }

  return (
    <div className="document-reader reader-file-pane">
      {hasPendingDiff && (
        <div className="reader-diff-strip">
          <ReaderDiffHeaderBar
            tab={tab}
            activeIndex={activeHunkIndex}
            onNavigate={navigateHunk}
            onAcceptAll={() => setActiveHunkIndex(0)}
            onRejectAll={() => setActiveHunkIndex(0)}
          />
        </div>
      )}
      <div className="document-reader-body reader-file-body">
        <ReaderEditor
          tab={tab}
          activeHunkIndex={hasPendingDiff ? activeHunkIndex : undefined}
          onActiveHunkChange={setActiveHunkIndex}
        />
      </div>
    </div>
  );
}

export function ReaderWorkspace({ path, onOpenFile }: ReaderWorkspaceProps) {
  const { t } = useTranslation();
  const activeId = useSession((s) => s.activeId);
  const tabs = useReader((s) => s.tabs);
  const openDoc = useReader((s) => s.openDoc);
  const findOpen = useReaderFind((s) => s.open);
  const openFind = useReaderFind((s) => s.openFind);
  const closeFind = useReaderFind((s) => s.close);

  const [loadError, setLoadError] = useState<string | null>(null);
  const [ratio, setRatio] = useState<number>(readStoredRatio);
  const [resizing, setResizing] = useState(false);
  const [showTree, setShowTree] = useState<boolean>(readStoredShowTree);
  const [rightView, setRightView] = useState<RightView>("tree");
  const [preview, setPreview] = useState(false);
  const containerRef = useRef<HTMLDivElement | null>(null);

  const tab = useMemo(() => {
    if (!path) return null;
    const key = normalizeReaderPath(path);
    return tabs.find((tb) => normalizeReaderPath(tb.path) === key) ?? null;
  }, [path, tabs]);

  const isMarkdown = tab?.fileType === "markdown";
  const hasPendingDiff = (tab?.pendingDiffs.length ?? 0) > 0;

  useReaderFindShortcuts(!!tab);

  // Preview only makes sense for markdown; reset when switching files.
  useEffect(() => {
    if (!isMarkdown) setPreview(false);
  }, [isMarkdown, path]);

  // Ctrl+F (or the search button) opens find → surface the search panel.
  useEffect(() => {
    if (findOpen) {
      setShowTree(true);
      setRightView("search");
    } else {
      setRightView((v) => (v === "search" ? "tree" : v));
    }
  }, [findOpen]);

  const persistShowTree = useCallback((next: boolean) => {
    setShowTree(next);
    try {
      window.localStorage.setItem(SHOW_TREE_KEY, next ? "1" : "0");
    } catch {
      /* ignore */
    }
  }, []);

  const toggleFileTree = useCallback(() => {
    if (showTree && rightView === "tree") {
      persistShowTree(false);
      return;
    }
    if (findOpen) closeFind();
    setRightView("tree");
    persistShowTree(true);
  }, [showTree, rightView, findOpen, closeFind, persistShowTree]);

  const toggleSearch = useCallback(() => {
    if (findOpen) {
      closeFind();
    } else {
      persistShowTree(true);
      openFind();
    }
  }, [findOpen, closeFind, openFind, persistShowTree]);

  // ---- Back / forward navigation history across opened files. ----
  const histRef = useRef<{ stack: string[]; index: number }>({ stack: [], index: -1 });
  const navPendingRef = useRef(false);
  const [, bumpNav] = useReducer((x: number) => x + 1, 0);

  // Rewrite history entries when files are renamed/moved/deleted.
  const readerPathSeq = useReader((s) => s.pathSeq);
  const lastHistPathSeq = useRef(readerPathSeq);
  useEffect(() => {
    if (readerPathSeq === lastHistPathSeq.current) return;
    lastHistPathSeq.current = readerPathSeq;
    const ops = useReader.getState().lastPathOps;
    if (!ops.length) return;
    const h = histRef.current;
    const nextStack: string[] = [];
    for (const p of h.stack) {
      const rewritten = applyReaderPathOpsToPath(p, ops);
      if (rewritten == null || rewritten === "") continue;
      const key = normalizeReaderPath(rewritten);
      if (nextStack.some((x) => normalizeReaderPath(x) === key)) continue;
      nextStack.push(rewritten);
    }
    let index = h.index;
    if (nextStack.length === 0) {
      h.stack = [];
      h.index = -1;
    } else {
      index = Math.max(0, Math.min(index, nextStack.length - 1));
      // Prefer landing on the current workspace path if it survived.
      if (path) {
        const at = nextStack.findIndex(
          (p) => normalizeReaderPath(p) === normalizeReaderPath(path),
        );
        if (at >= 0) index = at;
      }
      h.stack = nextStack;
      h.index = index;
    }
    bumpNav();
  }, [readerPathSeq, path]);

  useEffect(() => {
    if (!path) return;
    const h = histRef.current;
    if (navPendingRef.current) {
      navPendingRef.current = false;
      bumpNav();
      return;
    }
    const cur = h.index >= 0 ? h.stack[h.index] : null;
    if (cur && normalizeReaderPath(cur) === normalizeReaderPath(path)) return;
    h.stack = h.stack.slice(0, h.index + 1);
    h.stack.push(path);
    h.index = h.stack.length - 1;
    bumpNav();
  }, [path]);

  const canBack = histRef.current.index > 0;
  const canForward = histRef.current.index < histRef.current.stack.length - 1;

  const goBack = useCallback(() => {
    const h = histRef.current;
    if (h.index <= 0) return;
    h.index -= 1;
    navPendingRef.current = true;
    bumpNav();
    onOpenFile(h.stack[h.index]!);
  }, [onOpenFile]);

  const goForward = useCallback(() => {
    const h = histRef.current;
    if (h.index >= h.stack.length - 1) return;
    h.index += 1;
    navPendingRef.current = true;
    bumpNav();
    onOpenFile(h.stack[h.index]!);
  }, [onOpenFile]);

  // Lazily load a restored / freshly-selected file whose content isn't cached
  // yet in the reader store (activate:false keeps the visible tab selection).
  useEffect(() => {
    if (!path || tab || !activeId) return;
    let cancelled = false;
    setLoadError(null);
    api
      .readProjectFile(activeId, path)
      .then((file) => {
        if (cancelled) return;
        openDoc(
          {
            path,
            text: file.text,
            fileType: inferFileType(path),
            encoding: file.encoding,
            hadBom: file.hadBom,
            chars: countWords(file.text),
            lines: file.text.split("\n").length,
          },
          { activate: false },
        );
      })
      .catch((err) => {
        if (!cancelled) setLoadError(String(err));
      });
    return () => {
      cancelled = true;
    };
  }, [path, tab, activeId, openDoc]);

  useEffect(() => {
    if (resizing) return;
    try {
      window.localStorage.setItem(RATIO_KEY, String(ratio));
    } catch {
      /* ignore */
    }
  }, [ratio, resizing]);

  useEffect(() => {
    if (!resizing) return;
    const onMove = (e: MouseEvent) => {
      const el = containerRef.current;
      if (!el) return;
      const rect = el.getBoundingClientRect();
      if (rect.width <= 0) return;
      const next = (e.clientX - rect.left) / rect.width;
      setRatio(Math.min(MAX_RATIO, Math.max(MIN_RATIO, next)));
    };
    const onUp = () => setResizing(false);
    window.addEventListener("mousemove", onMove);
    window.addEventListener("mouseup", onUp);
    return () => {
      window.removeEventListener("mousemove", onMove);
      window.removeEventListener("mouseup", onUp);
    };
  }, [resizing]);

  useEffect(() => {
    if (!resizing) return;
    const prevCursor = document.body.style.cursor;
    const prevSelect = document.body.style.userSelect;
    document.body.style.cursor = "col-resize";
    document.body.style.userSelect = "none";
    return () => {
      document.body.style.cursor = prevCursor;
      document.body.style.userSelect = prevSelect;
    };
  }, [resizing]);

  useEffect(() => {
    if (loadError) toast.error(t("fileExplorer.openFailed"), { description: loadError });
  }, [loadError, t]);

  const fileName = path ? readerFileName(path) : "";

  const onMore = useCallback(
    (e: ReactMouseEvent) => {
      if (!path) return;
      openContextMenu(e, [
        {
          id: "copy-path",
          label: t("reader.copyPath"),
          onSelect: () => void copyText(path).then(() => toast.success(t("fileExplorer.copied"))),
        },
        {
          id: "reveal",
          label: t("reader.reveal"),
          onSelect: () => void api.openPath(path).catch(() => {}),
        },
      ]);
    },
    [path, t],
  );

  const hasFile = !!path;
  const bothPanes = hasFile && showTree;

  const toolbar = (
    <div className="reader-toolbar">
      <div className="reader-toolbar-nav">
        <button
          type="button"
          className="reader-toolbar-btn"
          title={t("reader.back")}
          disabled={!canBack}
          onClick={goBack}
        >
          <BackIcon />
        </button>
        <button
          type="button"
          className="reader-toolbar-btn"
          title={t("reader.forward")}
          disabled={!canForward}
          onClick={goForward}
        >
          <ForwardIcon />
        </button>
      </div>
      <div className="reader-toolbar-title" title={path ?? undefined}>
        <span className="reader-toolbar-name">{fileName || t("rightPanel.readerTab")}</span>
        {tab?.dirty && <span className="reader-tab-dirty" title={t("reader.unsaved")} />}
        {tab?.saveError && <span className="reader-tab-error" title={t("reader.saveFailed")} />}
      </div>
      <div className="reader-toolbar-actions">
        <button
          type="button"
          className={`reader-toolbar-btn${preview && isMarkdown ? " is-active" : ""}`}
          title={t("reader.preview")}
          disabled={!isMarkdown || hasPendingDiff}
          onClick={() => setPreview(true)}
        >
          <EyeIcon />
        </button>
        <button
          type="button"
          className={`reader-toolbar-btn${!preview ? " is-active" : ""}`}
          title={t("reader.source")}
          disabled={!hasFile}
          onClick={() => setPreview(false)}
        >
          <SourceIcon />
        </button>
        <button
          type="button"
          className="reader-toolbar-btn"
          title={t("reader.more")}
          disabled={!hasFile}
          onClick={onMore}
        >
          <MoreIcon />
        </button>
        <span className="reader-toolbar-sep" aria-hidden />
        <button
          type="button"
          className={`reader-toolbar-btn${findOpen ? " is-active" : ""}`}
          title={t("reader.search")}
          aria-pressed={findOpen}
          onClick={toggleSearch}
        >
          <SearchIcon />
        </button>
        <button
          type="button"
          className={`reader-toolbar-btn${showTree && rightView === "tree" ? " is-active" : ""}`}
          title={showTree && rightView === "tree" ? t("reader.hideFiles") : t("reader.showFiles")}
          aria-pressed={showTree && rightView === "tree"}
          onClick={toggleFileTree}
        >
          <FilesIcon />
        </button>
      </div>
    </div>
  );

  const editorNode = (
    <div
      className="reader-split-editor"
      style={bothPanes ? { flex: `0 0 ${ratio * 100}%` } : undefined}
    >
      {tab ? (
        <ReaderFilePane tab={tab} preview={preview} />
      ) : (
        <div className="document-reader is-empty reader-file-pane">
          <p className="document-reader-empty">
            {loadError ? t("fileExplorer.openFailed") : t("rightPanel.readerEmpty")}
          </p>
        </div>
      )}
    </div>
  );

  const rightNode = (
    <div className="reader-split-tree" style={bothPanes ? { flex: "1 1 0" } : undefined}>
      {rightView === "search" ? (
        <div className="reader-search-pane">
          <ReaderFindBar
            disabled={hasPendingDiff}
            disabledReason={hasPendingDiff ? t("readerFind.diffBlocked") : undefined}
          />
        </div>
      ) : (
        <ReaderFileTree activePath={path ?? null} onOpenFile={onOpenFile} />
      )}
    </div>
  );

  return (
    <div className="reader-workspace-outer">
      {toolbar}
      <div ref={containerRef} className="reader-workspace">
        {hasFile ? (
          <>
            {editorNode}
            {bothPanes && (
              <div
                className={`reader-split-divider${resizing ? " is-resizing" : ""}`}
                role="separator"
                aria-orientation="vertical"
                onMouseDown={(e) => {
                  if (e.button !== 0) return;
                  e.preventDefault();
                  setResizing(true);
                }}
                onDoubleClick={() => setRatio(0.58)}
              />
            )}
            {bothPanes && rightNode}
          </>
        ) : (
          <div className="reader-split-tree reader-split-tree--full">
            {rightView === "search" ? (
              <div className="reader-search-pane">
                <ReaderFindBar />
              </div>
            ) : (
              <ReaderFileTree activePath={null} onOpenFile={onOpenFile} />
            )}
          </div>
        )}
      </div>
    </div>
  );
}

function BackIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round">
      <path d="M15 18l-6-6 6-6" />
    </svg>
  );
}

function ForwardIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round">
      <path d="M9 18l6-6-6-6" />
    </svg>
  );
}

function EyeIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round">
      <path d="M2 12s3.5-7 10-7 10 7 10 7-3.5 7-10 7-10-7-10-7Z" />
      <circle cx="12" cy="12" r="3" />
    </svg>
  );
}

function SourceIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round">
      <path d="m8 9-3 3 3 3M16 9l3 3-3 3M13.5 6l-3 12" />
    </svg>
  );
}

function MoreIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="currentColor" aria-hidden>
      <circle cx="5" cy="12" r="1.6" />
      <circle cx="12" cy="12" r="1.6" />
      <circle cx="19" cy="12" r="1.6" />
    </svg>
  );
}

function SearchIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round">
      <circle cx="11" cy="11" r="7" />
      <path d="m20 20-3.5-3.5" />
    </svg>
  );
}

function FilesIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round">
      <rect x="3" y="4" width="18" height="16" rx="2" />
      <path d="M14 4v16" />
    </svg>
  );
}

/** @deprecated Use ReaderWorkspace */
export function DocumentReader() {
  return <ReaderWorkspace onOpenFile={() => {}} />;
}
