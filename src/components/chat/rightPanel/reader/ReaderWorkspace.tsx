import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  type MouseEvent as ReactMouseEvent,
} from "react";
import { useTranslation } from "react-i18next";
import { api } from "../../../../api/tauri";
import { copyText } from "../../../../utils/clipboard";
import { openContextMenu } from "../../../context-menu";
import { toast } from "../../../ui/Toast";
import {
  isMediaFileType,
  normalizeReaderPath,
  readerFileName,
  syncPendingDiffsForPath,
  useReader,
} from "../../../../store/reader";
import { useReaderFind } from "../../../../store/readerFind";
import { useSession } from "../../../../store/session";
import { usePanelTab } from "../context/PanelTabContext";
import { DEFAULT_RATIO } from "./constants";
import { ReaderFilePane } from "./components/ReaderFilePane";
import { ReaderToolbar } from "./components/ReaderToolbar";
import { ReaderFileTree } from "./fileTree/ReaderFileTree";
import { ReaderFindBar, useReaderFindShortcuts } from "./find/ReaderFindBar";
import { useLazyLoadFile } from "./hooks/useLazyLoadFile";
import { useReaderNavHistory } from "./hooks/useReaderNavHistory";
import { useReaderSplit } from "./hooks/useReaderSplit";
import { indentRegisteredReaderDocument } from "./readerCodeMirror/indentKeymap";
import type { ReaderWorkspaceProps } from "./types";

export type { ReaderWorkspaceProps } from "./types";

export function ReaderWorkspace({ path, onOpenFile }: ReaderWorkspaceProps) {
  const { t } = useTranslation();
  const { tabId, isActive } = usePanelTab();
  const activeId = useSession((s) => s.activeId);
  const tabs = useReader((s) => s.tabs);

  const tab = useMemo(() => {
    if (!path) return null;
    const key = normalizeReaderPath(path);
    return tabs.find((tb) => normalizeReaderPath(tb.path) === key) ?? null;
  }, [path, tabs]);

  const isMarkdown = tab?.fileType === "markdown";
  const isMedia = isMediaFileType(tab?.fileType);
  const hasPendingDiff = !isMedia && (tab?.pendingDiffs.length ?? 0) > 0;

  // Find belongs to the visible tab: bind it here, and only listen for the
  // shortcut while this pane is the active one.
  const bindFindTab = useReaderFind((s) => s.bindTab);
  useEffect(() => {
    if (isActive) bindFindTab(tabId);
  }, [isActive, tabId, bindFindTab]);

  useReaderFindShortcuts(isActive && !!tab);

  const {
    ratio,
    setRatio,
    resizing,
    setResizing,
    showTree,
    rightView,
    preview,
    setPreview,
    containerRef,
    findOpen,
    toggleFileTree,
    toggleSearch,
  } = useReaderSplit(!!isMarkdown, path);

  const { canBack, canForward, goBack, goForward } = useReaderNavHistory(path);
  const loadError = useLazyLoadFile(path, tab, activeId);

  // Re-open / switch path: restore Keep/Undo from backend (authoritative).
  useEffect(() => {
    if (!path || !tab || !activeId || isMedia) return;
    void syncPendingDiffsForPath(activeId, path);
  }, [path, tab?.id, activeId, isMedia]);

  useEffect(() => {
    if (!isActive || !loadError) return;
    toast.error(t("fileExplorer.openFailed"), { description: loadError });
  }, [isActive, loadError, t]);

  const fileName = path ? readerFileName(path) : "";
  const hasFile = !!path;
  const canIndent = !!tab && !isMedia && !hasPendingDiff;

  const onIndent = useCallback(
    (e: ReactMouseEvent) => {
      if (!canIndent) return;
      indentRegisteredReaderDocument(e.shiftKey, !!isMarkdown);
    },
    [canIndent, isMarkdown],
  );

  const onMore = useCallback(
    (e: ReactMouseEvent) => {
      if (!path) return;
      openContextMenu(e, [
        {
          id: "indent-first",
          label: t("reader.indentFirstLine"),
          disabled: !canIndent,
          onSelect: () => {
            indentRegisteredReaderDocument(false, !!isMarkdown);
          },
        },
        {
          id: "outdent-first",
          label: t("reader.outdentFirstLine"),
          disabled: !canIndent,
          onSelect: () => {
            indentRegisteredReaderDocument(true, !!isMarkdown);
          },
        },
        { type: "separator", id: "indent-sep" },
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
    [path, t, canIndent, isMarkdown],
  );

  const bothPanes = hasFile && showTree;
  // The find bar is a single shared surface bound to the active tab; a hidden
  // pane falls back to its file tree and restores the search on re-activation.
  const showFindPane = rightView === "search" && isActive;

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
      {showFindPane ? (
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
      <ReaderToolbar
        path={path}
        fileName={fileName}
        tab={tab}
        canBack={canBack}
        canForward={canForward}
        onBack={goBack}
        onForward={goForward}
        preview={preview}
        isMarkdown={!!isMarkdown}
        isMedia={!!isMedia}
        hasFile={hasFile}
        findOpen={findOpen}
        showTree={showTree}
        rightViewIsTree={rightView === "tree"}
        canIndent={canIndent}
        indentTitle={
          hasPendingDiff ? t("reader.indentDiffBlocked") : t("reader.indentFirstLineHint")
        }
        onPreview={() => setPreview(true)}
        onSource={() => setPreview(false)}
        onIndent={onIndent}
        onMore={onMore}
        onToggleSearch={toggleSearch}
        onToggleFileTree={toggleFileTree}
      />
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
                onDoubleClick={() => setRatio(DEFAULT_RATIO)}
              />
            )}
            {bothPanes && rightNode}
          </>
        ) : (
          <div className="reader-split-tree reader-split-tree--full">
            {showFindPane ? (
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
