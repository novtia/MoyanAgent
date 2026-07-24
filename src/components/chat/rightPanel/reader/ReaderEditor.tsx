import { DiffEditorView } from "./components/DiffEditorView";
import { PlainReader } from "./components/PlainReader";
import { useEditorSave } from "./hooks/useEditorSave";
import type { ReaderEditorProps } from "./types";

export type { ReaderEditorProps } from "./types";

export function ReaderEditor({ tab, activeHunkIndex, onActiveHunkChange }: ReaderEditorProps) {
  const applyText = useEditorSave(tab);
  const hasPendingDiff = tab.pendingDiffs.length > 0;

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
