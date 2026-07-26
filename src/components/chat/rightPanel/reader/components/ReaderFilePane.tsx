import { useCallback, useEffect, useState } from "react";
import {
  isMediaFileType,
  type ReaderFileTab,
} from "../../../../../store/reader";
import { ReaderDiffHeaderBar } from "../ReaderDiffHeaderBar";
import { ReaderEditor } from "../ReaderEditor";
import { ReaderMarkdownPreview } from "../ReaderMarkdownPreview";
import { MediaReader } from "./MediaReader";

/** Editor pane body: media viewer, markdown preview, or the source/diff editor. */
export function ReaderFilePane({ tab, preview }: { tab: ReaderFileTab; preview: boolean }) {
  const [activeHunkIndex, setActiveHunkIndex] = useState(0);
  const media = isMediaFileType(tab.fileType);
  const hasPendingDiff = !media && tab.pendingDiffs.length > 0;
  // Keep preview + source mounted together so toggling does not reset scroll.
  const canPreview = tab.fileType === "markdown" && !hasPendingDiff;
  const showPreview = preview && canPreview;

  // Reset only when switching files — not after each confirm (that used to
  // force activeHunkIndex → 0 and scroll the editor back to the top).
  useEffect(() => {
    setActiveHunkIndex(0);
  }, [tab.id]);

  useEffect(() => {
    const total = tab.pendingDiffs.length;
    if (total === 0) return;
    setActiveHunkIndex((prev) => Math.min(prev, total - 1));
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
        {canPreview && (
          <div
            className={`reader-pane-layer${showPreview ? " is-visible" : ""}`}
            aria-hidden={!showPreview}
          >
            <ReaderMarkdownPreview text={tab.text} />
          </div>
        )}
        <div
          className={`reader-pane-layer${!showPreview ? " is-visible" : ""}`}
          aria-hidden={showPreview}
        >
          <ReaderEditor
            tab={tab}
            activeHunkIndex={hasPendingDiff ? activeHunkIndex : undefined}
            onActiveHunkChange={setActiveHunkIndex}
          />
        </div>
      </div>
    </div>
  );
}
