import { useCallback, useEffect, useState } from "react";
import {
  isMediaFileType,
  type ReaderFileTab,
} from "../../../../../store/reader";
import type { RewriteMode } from "../../../../../store/readerRewrite";
import { ReaderDiffHeaderBar } from "../ReaderDiffHeaderBar";
import { ReaderEditor } from "../ReaderEditor";
import { ReaderMarkdownPreview } from "../ReaderMarkdownPreview";
import type { ChunkRewriteTarget } from "./ChunkedEditorView";
import { MediaReader } from "./MediaReader";

/** Editor pane body: media viewer, markdown preview, or the source/diff editor. */
export function ReaderFilePane({
  tab,
  preview,
  chunked,
  rewriteRunning,
  onRewriteChunk,
}: {
  tab: ReaderFileTab;
  preview: boolean;
  chunked?: boolean;
  rewriteRunning?: boolean;
  onRewriteChunk?: (mode: RewriteMode, target: ChunkRewriteTarget) => void;
}) {
  // -1 means "not on a hunk yet" (shown as 0/N) so a lone change can still be jumped to.
  const [activeHunkIndex, setActiveHunkIndex] = useState(() =>
    tab.pendingDiffs.length > 1 ? 0 : -1,
  );
  const media = isMediaFileType(tab.fileType);
  const hasPendingDiff = !media && tab.pendingDiffs.length > 0;
  // Keep preview + source mounted together so toggling does not reset scroll.
  // Pending diffs must not kick the user out of preview — they are reading.
  const canPreview = tab.fileType === "markdown";
  const showPreview = preview && canPreview;
  // Mount Markdown preview lazily on first toggle: rendering 170KB+ of markdown
  // into thousands of DOM nodes while hidden costs ~9s of render + ~16s of paint
  // on initial file open. Once mounted, keep it alive so toggling preserves scroll.
  const [previewMounted, setPreviewMounted] = useState(showPreview);
  useEffect(() => {
    if (showPreview) setPreviewMounted(true);
  }, [showPreview]);

  // Reset only when switching files — not after each confirm (that used to
  // force activeHunkIndex → 0 and scroll the editor back to the top).
  useEffect(() => {
    setActiveHunkIndex(tab.pendingDiffs.length > 1 ? 0 : -1);
    setPreviewMounted(showPreview);
  }, [tab.id]);

  useEffect(() => {
    const total = tab.pendingDiffs.length;
    if (total === 0) return;
    setActiveHunkIndex((prev) => {
      if (total > 1 && prev < 0) return 0;
      return Math.min(prev, total - 1);
    });
  }, [tab.pendingDiffs.length]);

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

  if (isMediaFileType(tab.fileType)) {
    return (
      <div className="document-reader reader-file-pane reader-file-pane--media">
        <div className="document-reader-body reader-file-body">
          <MediaReader path={tab.path} fileType={tab.fileType} />
        </div>
      </div>
    );
  }

  return (
    <div className="document-reader reader-file-pane">
      {hasPendingDiff && !showPreview && (
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
        {canPreview && (previewMounted || showPreview) && (
          <div
            className={`reader-pane-layer${showPreview ? " is-visible" : ""}`}
            style={showPreview ? undefined : { display: "none" }}
            aria-hidden={!showPreview}
          >
            <ReaderMarkdownPreview text={tab.text} path={tab.path} />
          </div>
        )}
        <div
          className={`reader-pane-layer${!showPreview ? " is-visible" : ""}`}
          style={!showPreview ? undefined : { display: "none" }}
          aria-hidden={showPreview}
        >
          <ReaderEditor
            tab={tab}
            chunked={chunked}
            rewriteRunning={rewriteRunning}
            onRewriteChunk={onRewriteChunk}
            activeHunkIndex={hasPendingDiff ? activeHunkIndex : undefined}
            onActiveHunkChange={setActiveHunkIndex}
          />
        </div>
      </div>
    </div>
  );
}
