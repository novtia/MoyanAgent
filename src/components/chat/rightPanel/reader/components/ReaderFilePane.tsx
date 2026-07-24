import { useCallback, useEffect, useState } from "react";
import type { ReaderFileTab } from "../../../../../store/reader";
import { ReaderDiffHeaderBar } from "../ReaderDiffHeaderBar";
import { ReaderEditor } from "../ReaderEditor";
import { ReaderMarkdownPreview } from "../ReaderMarkdownPreview";

/** Editor pane body: rendered markdown preview or the source/diff editor. */
export function ReaderFilePane({ tab, preview }: { tab: ReaderFileTab; preview: boolean }) {
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
