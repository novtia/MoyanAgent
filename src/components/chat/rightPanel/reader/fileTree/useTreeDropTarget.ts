import {
  useCallback,
  useState,
  type DragEvent as ReactDragEvent,
} from "react";
import { useTranslation } from "react-i18next";
import { toast } from "../../../../ui/Toast";
import {
  handleTreeDrop,
  isTreeDroppable,
  peekTreeDropKind,
  resolveTreeDropTargetDir,
} from "./importDrop";

export interface UseTreeDropTargetOptions {
  sessionId: string;
  root: string;
  /** Absolute path of the hovered entry; omit for project root / blank area. */
  entryPath?: string | null;
  isDir?: boolean;
  onImported?: (targetDir: string) => void;
}

export function useTreeDropTarget({
  sessionId,
  root,
  entryPath = null,
  isDir = false,
  onImported,
}: UseTreeDropTargetOptions) {
  const { t } = useTranslation();
  const [isDropTarget, setIsDropTarget] = useState(false);
  const targetDir = resolveTreeDropTargetDir({ root, entryPath, isDir });

  const onDragEnter = useCallback((e: ReactDragEvent) => {
    if (!isTreeDroppable(e.dataTransfer)) return;
    e.preventDefault();
    e.stopPropagation();
    setIsDropTarget(true);
  }, []);

  const onDragOver = useCallback((e: ReactDragEvent) => {
    if (!isTreeDroppable(e.dataTransfer)) return;
    e.preventDefault();
    e.stopPropagation();
    const kind = peekTreeDropKind(e.dataTransfer);
    if (e.dataTransfer) {
      e.dataTransfer.dropEffect = kind === "internal" ? "move" : "copy";
    }
    setIsDropTarget(true);
  }, []);

  const onDragLeave = useCallback((e: ReactDragEvent) => {
    if (!isTreeDroppable(e.dataTransfer)) return;
    e.preventDefault();
    e.stopPropagation();
    const related = e.relatedTarget as Node | null;
    if (related && e.currentTarget.contains(related)) return;
    setIsDropTarget(false);
  }, []);

  const onDrop = useCallback(
    async (e: ReactDragEvent) => {
      if (!isTreeDroppable(e.dataTransfer)) return;
      e.preventDefault();
      e.stopPropagation();
      setIsDropTarget(false);
      try {
        const result = await handleTreeDrop({
          sessionId,
          targetDir,
          dataTransfer: e.dataTransfer,
        });
        if (!result) return;

        if (result.kind === "internal") {
          if (result.moved > 0) {
            toast.success(t("fileExplorer.moved"));
            onImported?.(targetDir);
          }
          return;
        }

        if (result.imported > 0) {
          onImported?.(targetDir);
        }

        if (
          result.imported > 0 &&
          result.errors.length === 0 &&
          result.skippedNoPath === 0
        ) {
          toast.success(
            t("fileExplorer.importSuccess", { count: result.imported }),
          );
          return;
        }
        if (
          result.imported > 0 &&
          (result.errors.length > 0 || result.skippedNoPath > 0)
        ) {
          toast.success(
            t("fileExplorer.importPartial", {
              ok: result.imported,
              fail: result.errors.length + result.skippedNoPath,
            }),
            {
              description:
                result.errors[0] ||
                (result.skippedNoPath
                  ? t("fileExplorer.importNoPath")
                  : undefined),
            },
          );
          return;
        }
        if (result.skippedNoPath > 0 && result.errors.length === 0) {
          toast.error(t("fileExplorer.importNoPath"));
          return;
        }
        if (result.errors.length > 0) {
          toast.error(t("fileExplorer.importFailed"), {
            description: result.errors[0],
          });
        }
      } catch (err) {
        toast.error(t("fileExplorer.importFailed"), {
          description: err instanceof Error ? err.message : String(err),
        });
      }
    },
    [onImported, sessionId, t, targetDir],
  );

  return {
    isDropTarget,
    dropHandlers: {
      onDragEnter,
      onDragOver,
      onDragLeave,
      onDrop,
    },
  };
}
