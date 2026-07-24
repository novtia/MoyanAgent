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
  normalizeReaderPath,
  readerFileName,
  useReader,
} from "../../../../store/reader";
import { useSession } from "../../../../store/session";
import { ReaderFilePane } from "./components/ReaderFilePane";
import { ReaderToolbar } from "./components/ReaderToolbar";
import { ReaderFileTree } from "./fileTree/ReaderFileTree";
import { ReaderFindBar, useReaderFindShortcuts } from "./find/ReaderFindBar";
import { useLazyLoadFile } from "./hooks/useLazyLoadFile";
import { useReaderNavHistory } from "./hooks/useReaderNavHistory";
import { useReaderSplit } from "./hooks/useReaderSplit";
import type { ReaderWorkspaceProps } from "./types";

export type { ReaderWorkspaceProps } from "./types";

export function ReaderWorkspace({ path, onOpenFile }: ReaderWorkspaceProps) {
  const { t } = useTranslation();
  const activeId = useSession((s) => s.activeId);
  const tabs = useReader((s) => s.tabs);

  const tab = useMemo(() => {
    if (!path) return null;
    const key = normalizeReaderPath(path);
    return tabs.find((tb) => normalizeReaderPath(tb.path) === key) ?? null;
  }, [path, tabs]);

  const isMarkdown = tab?.fileType === "markdown";
  const hasPendingDiff = (tab?.pendingDiffs.length ?? 0) > 0;

  useReaderFindShortcuts(!!tab);

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

  const { canBack, canForward, goBack, goForward } = useReaderNavHistory(path, onOpenFile);
  const loadError = useLazyLoadFile(path, tab, activeId);

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
        hasPendingDiff={hasPendingDiff}
        hasFile={hasFile}
        findOpen={findOpen}
        showTree={showTree}
        rightViewIsTree={rightView === "tree"}
        onPreview={() => setPreview(true)}
        onSource={() => setPreview(false)}
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

/** @deprecated Use ReaderWorkspace */
export function DocumentReader() {
  return <ReaderWorkspace onOpenFile={() => {}} />;
}
