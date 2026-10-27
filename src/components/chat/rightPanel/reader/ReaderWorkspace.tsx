import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  type MouseEvent as ReactMouseEvent,
} from "react";
import type { EditorView } from "@codemirror/view";
import { useTranslation } from "react-i18next";
import { api } from "../../../../api/tauri";
import { copyText } from "../../../../utils/clipboard";
import { openContextMenu } from "../../../context-menu";
import { dialog } from "../../../ui/Dialog";
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
import {
  ensureRewriteEvents,
  rewriteModelKey,
  rewriteModelOptions,
  useReaderRewrite,
  type RewriteMode,
} from "../../../../store/readerRewrite";
import { useSettings } from "../../../../store/settings";
import { usePanelTab } from "../context/PanelTabContext";
import { DEFAULT_RATIO } from "./constants";
import { ReaderFilePane } from "./components/ReaderFilePane";
import { ReaderRewritePane } from "./components/ReaderRewritePane";
import { DEFAULT_REWRITE_SETTINGS } from "../../../settings/rewrite/RewriteSection";
import { ReaderToolbar } from "./components/ReaderToolbar";
import { ReaderFileTree } from "./fileTree/ReaderFileTree";
import { ReaderFindBar, useReaderFindShortcuts } from "./find/ReaderFindBar";
import { useLazyLoadFile } from "./hooks/useLazyLoadFile";
import { useReaderNavHistory } from "./hooks/useReaderNavHistory";
import { useReaderSplit } from "./hooks/useReaderSplit";
import { ReaderEditorViewContext } from "./readerCodeMirror/editorViewContext";
import { indentReaderView } from "./readerCodeMirror/indentKeymap";
import type { ReaderWorkspaceProps } from "./types";

export type { ReaderWorkspaceProps } from "./types";

/** Text the user highlighted in the visible reader layer, source or preview. */
function selectedReaderText(root: HTMLElement | null, view: EditorView | null): string {
  if (!root) return "";
  const layer = root.querySelector(".reader-pane-layer.is-visible");
  const scope = layer ?? root;
  if (view && scope.contains(view.dom)) {
    const sel = view.state.selection.main;
    if (!sel.empty) {
      const text = view.state.sliceDoc(sel.from, sel.to).replace(/\u00a0/g, " ").trim();
      if (text) return text;
    }
  }
  const dom = window.getSelection();
  if (!dom || dom.isCollapsed) return "";
  const node = dom.anchorNode;
  if (!node || !scope.contains(node)) return "";
  return dom.toString().replace(/\u00a0/g, " ").trim();
}

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
  const firstLineIndent = useSettings((s) => s.settings?.reader_first_line_indent ?? true);
  const paragraphSpacing = useSettings((s) => s.settings?.reader_paragraph_spacing ?? true);
  const formatOptions = useMemo(
    () => ({ firstLineIndent, paragraphSpacing }),
    [firstLineIndent, paragraphSpacing],
  );
  const formatEnabled = firstLineIndent || paragraphSpacing;
  const canIndent = !!tab && !isMedia && !hasPendingDiff && formatEnabled;
  const editorViewRef = useRef<EditorView | null>(null);
  const editorRootRef = useRef<HTMLDivElement | null>(null);

  const onIndent = useCallback(
    (e: ReactMouseEvent) => {
      if (!canIndent) return;
      indentReaderView(editorViewRef.current, e.shiftKey, !!isMarkdown, formatOptions);
    },
    [canIndent, isMarkdown, formatOptions],
  );

  const onMore = useCallback(
    (e: ReactMouseEvent) => {
      if (!path) return;
      openContextMenu(e, [
        {
          id: "indent-first",
          label: t("reader.format"),
          disabled: !canIndent,
          onSelect: () => {
            indentReaderView(editorViewRef.current, false, !!isMarkdown, formatOptions);
          },
        },
        {
          id: "outdent-first",
          label: t("reader.formatUndo"),
          disabled: !canIndent,
          onSelect: () => {
            indentReaderView(editorViewRef.current, true, !!isMarkdown, formatOptions);
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
    [path, t, canIndent, isMarkdown, formatOptions],
  );

  const settings = useSettings((s) => s.settings);
  const updateSettings = useSettings((s) => s.update);
  const autoChunk = settings?.rewrite?.auto_chunk ?? true;
  const rewriteMode = useReaderRewrite((s) => s.mode);
  const setRewriteMode = useReaderRewrite((s) => s.setMode);
  const storedModelKey = useReaderRewrite((s) => s.modelKey);
  const setRewriteModelKey = useReaderRewrite((s) => s.setModelKey);
  const beginRewrite = useReaderRewrite((s) => s.begin);
  const failRewrite = useReaderRewrite((s) => s.fail);
  const setRewriteView = useReaderRewrite((s) => s.setView);
  const rewriteRun = useReaderRewrite((s) =>
    path ? s.byPath[normalizeReaderPath(path)] : undefined,
  );

  useEffect(() => {
    ensureRewriteEvents();
  }, []);

  const rewriteModels = useMemo(
    () =>
      rewriteModelOptions(settings?.model_services ?? []).map((model) => ({
        key: rewriteModelKey(model.providerId, model.modelId),
        name: model.name,
        sdk: model.sdk,
        providerId: model.providerId,
        modelId: model.modelId,
      })),
    [settings?.model_services],
  );

  useEffect(() => {
    if (rewriteModels.length === 0) return;
    if (rewriteModels.some((model) => model.key === storedModelKey)) return;
    const preferred = rewriteModels.find((model) => model.sdk === "local") ?? rewriteModels[0];
    setRewriteModelKey(preferred.key);
  }, [rewriteModels, storedModelKey, setRewriteModelKey]);

  const selectedRewriteModel =
    rewriteModels.find((model) => model.key === storedModelKey) ?? rewriteModels[0];
  const showFindPane = rightView === "search" && isActive;
  const rewriteVisible =
    !!rewriteRun &&
    rewriteRun.view === "rewrite" &&
    (rewriteRun.running || rewriteRun.text.length > 0 || !!rewriteRun.error);
  const showRewritePane = hasFile && !isMedia && rewriteVisible && !showFindPane;

  const startRewrite = useCallback(
    async (mode: RewriteMode, source: string) => {
      if (!path) return;
      if (rewriteRun?.running) {
        toast.warning(t("reader.rewriteBusy"));
        return;
      }
      if (!selectedRewriteModel) {
        toast.warning(t("reader.noRewriteModel"));
        return;
      }
      const text = source.trim();
      if (!text) {
        toast.warning(t("reader.rewriteEmptySource"));
        return;
      }
      if (rewriteRun?.text) {
        const ok = await dialog.confirm(t("reader.rewriteConfirm"));
        if (!ok) return;
      }
      setRewriteMode(mode);
      const token = beginRewrite(path, mode);
      try {
        await api.startReaderRewrite({
          path,
          text,
          mode,
          providerId: selectedRewriteModel.providerId,
          modelId: selectedRewriteModel.modelId,
          clientToken: token,
        });
      } catch (err) {
        failRewrite(path, token, String(err));
      }
    },
    [
      path,
      rewriteRun?.running,
      rewriteRun?.text,
      selectedRewriteModel,
      setRewriteMode,
      beginRewrite,
      failRewrite,
      t,
    ],
  );

  const onRewriteRun = useCallback(async () => {
    if (!path) return;
    if (rewriteRun?.running) {
      try {
        await api.cancelReaderRewrite();
      } catch (err) {
        toast.error(t("reader.stopRewrite"), { description: String(err) });
      }
      return;
    }
    const selected = selectedReaderText(editorRootRef.current, editorViewRef.current);
    await startRewrite(rewriteMode, selected || tab?.text || "");
  }, [path, rewriteRun?.running, rewriteMode, startRewrite, tab?.text, t]);

  const onEditorContextMenu = useCallback(
    (event: ReactMouseEvent<HTMLDivElement>) => {
      const selected = selectedReaderText(event.currentTarget, editorViewRef.current);
      if (!selected || isMedia) return;
      openContextMenu(event, [
        {
          id: "copy-selection",
          label: t("common.copy"),
          onSelect: () =>
            void copyText(selected).then(() => toast.success(t("common.copied"))),
        },
        { type: "separator", id: "rewrite-selection-sep" },
        {
          id: "rewrite-selection",
          label: t("reader.rewriteMode"),
          disabled: !!rewriteRun?.running,
          onSelect: () => void startRewrite("rewrite", selected),
        },
        {
          id: "expand-selection",
          label: t("reader.expandMode"),
          disabled: !!rewriteRun?.running,
          onSelect: () => void startRewrite("expand", selected),
        },
        {
          id: "pipeline-selection",
          label: t("reader.pipelineMode"),
          disabled: !!rewriteRun?.running,
          onSelect: () => void startRewrite("pipeline", selected),
        },
      ]);
    },
    [isMedia, rewriteRun?.running, startRewrite, t],
  );

  const bothPanes = hasFile && (showRewritePane || showTree);

  const editorNode = (
    <div
      ref={editorRootRef}
      className="reader-split-editor"
      style={bothPanes ? { flex: `0 0 ${ratio * 100}%` } : undefined}
      onContextMenu={onEditorContextMenu}
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
      {showRewritePane && rewriteRun ? (
        <ReaderRewritePane run={rewriteRun} />
      ) : showFindPane ? (
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
    <ReaderEditorViewContext.Provider value={editorViewRef}>
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
          rightViewIsTree={rightView === "tree" && !showRewritePane}
          canIndent={canIndent}
        indentTitle={
          hasPendingDiff
            ? t("reader.indentDiffBlocked")
            : !formatEnabled
              ? t("reader.formatDisabled")
              : firstLineIndent && paragraphSpacing
                ? t("reader.indentFirstLineHint")
                : firstLineIndent
                  ? t("reader.formatHintIndent")
                  : t("reader.formatHintSpacing")
        }
          onPreview={() => setPreview(true)}
          onSource={() => setPreview(false)}
          onIndent={onIndent}
          onMore={onMore}
          onToggleSearch={() => {
            if (path && rewriteVisible) setRewriteView(path, "files");
            toggleSearch();
          }}
          onToggleFileTree={() => {
            if (path && rewriteVisible) {
              setRewriteView(path, "files");
              if (!showTree) toggleFileTree();
              return;
            }
            toggleFileTree();
          }}
          showRewrite={hasFile && !isMedia}
          rewriteMode={rewriteMode}
          rewriteRunning={!!rewriteRun?.running}
          rewriteModels={rewriteModels}
          rewriteModelKey={selectedRewriteModel?.key ?? ""}
          canRewrite={!!selectedRewriteModel && !!(tab?.text ?? "").trim()}
          rewriteRunTitle={
            selectedRewriteModel ? t("reader.regenerate") : t("reader.noRewriteModel")
          }
          onRewriteMode={setRewriteMode}
          onRewriteModel={setRewriteModelKey}
          autoChunk={autoChunk}
          onToggleAutoChunk={() => {
            void updateSettings({
              rewrite: {
                ...DEFAULT_REWRITE_SETTINGS,
                ...settings?.rewrite,
                auto_chunk: !autoChunk,
              },
            });
          }}
          onRewriteRun={() => void onRewriteRun()}
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
    </ReaderEditorViewContext.Provider>
  );
}
