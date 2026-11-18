import { useEffect, useRef } from "react";
import { ChunkedEditorView } from "./components/ChunkedEditorView";
import { DiffEditorView } from "./components/DiffEditorView";
import { PlainReader } from "./components/PlainReader";
import { useEditorSave } from "./hooks/useEditorSave";
import type { ReaderEditorProps } from "./types";
import {
  finishReaderOpenTrace,
  markReaderOpenFrontend,
} from "../../../../utils/readerPerf";

export type { ReaderEditorProps } from "./types";

export function ReaderEditor({
  tab,
  chunked,
  rewriteRunning,
  onRewriteChunk,
  activeHunkIndex,
  onActiveHunkChange,
}: ReaderEditorProps) {
  const applyText = useEditorSave(tab);
  const hasPendingDiff = tab.pendingDiffs.length > 0;
  const mode = chunked ? "chunked" : hasPendingDiff ? "diff" : "plain";
  const renderStartRef = useRef<number>(0);
  renderStartRef.current = performance.now();

  useEffect(() => {
    const renderMs = performance.now() - renderStartRef.current;
    const paintStart = performance.now();
    let raf2 = 0;
    const raf1 = requestAnimationFrame(() => {
      raf2 = requestAnimationFrame(() => {
        markReaderOpenFrontend(tab.path, "frontendRenderMs", renderMs, {
          mode,
          textChars: tab.text.length,
          textLines: tab.lines,
          pendingDiffsCount: tab.pendingDiffs.length,
        });
        markReaderOpenFrontend(tab.path, "frontendPaintMs", performance.now() - paintStart);
        finishReaderOpenTrace(tab.path, mode);
      });
    });
    return () => {
      cancelAnimationFrame(raf1);
      cancelAnimationFrame(raf2);
    };
  }, [tab.path, mode]);

  if (chunked) {
    return (
      <ChunkedEditorView
        tab={tab}
        applyText={applyText}
        rewriteRunning={rewriteRunning}
        onRewriteChunk={onRewriteChunk}
        activeHunkIndex={activeHunkIndex}
        onActiveHunkChange={onActiveHunkChange}
      />
    );
  }

  if (!hasPendingDiff) {
    return <PlainReader tab={tab} applyText={applyText} />;
  }

  return (
    <DiffEditorView
      tab={tab}
      applyText={applyText}
      activeHunkIndex={activeHunkIndex}
      onActiveHunkChange={onActiveHunkChange}
    />
  );
}
