import { useEffect, useState } from "react";
import { api } from "../../../../../api/tauri";
import {
  countWords,
  inferFileType,
  isMediaFileType,
  syncPendingDiffsForPath,
  useReader,
  type ReaderFileTab,
} from "../../../../../store/reader";
import {
  markReaderOpenBackend,
  markReaderOpenFrontend,
} from "../../../../../utils/readerPerf";

/**
 * Lazily cache a restored / freshly-selected file whose content isn't loaded
 * yet. Purely a content load: it never focuses a tab or opens the panel, so
 * restoring a session cannot pop the reader open behind the user's back.
 */
export function useLazyLoadFile(
  path: string | null | undefined,
  tab: ReaderFileTab | null,
  activeId: string | null,
) {
  const openDoc = useReader((s) => s.openDoc);
  const [loadError, setLoadError] = useState<string | null>(null);

  useEffect(() => {
    if (!path || !activeId) return;

    const fileType = inferFileType(path);
    // Upgrade a stale text tab that was incorrectly opened for a media file.
    if (tab && isMediaFileType(fileType) && !isMediaFileType(tab.fileType)) {
      openDoc({ path, text: "", fileType, chars: 0, lines: 0 });
      return;
    }

    if (tab) {
      return;
    }

    let cancelled = false;
    setLoadError(null);

    if (isMediaFileType(fileType)) {
      // Media tabs do not decode bytes — the viewer loads via asset protocol.
      openDoc({ path, text: "", fileType, chars: 0, lines: 0 });
      return;
    }

    const tReadStart = performance.now();
    api
      .readProjectFile(activeId, path)
      .then(async (file) => {
        const readMs = performance.now() - tReadStart;
        // Drop the result when the session changed while reading from disk:
        // another conversation's reader cache must never receive this doc.
        if (cancelled || useReader.getState().sessionId !== activeId) return;
        const linesCount = file.text.split("\n").length;
        markReaderOpenBackend(path, "backendReadMs", readMs, {
          textChars: file.text.length,
          textLines: linesCount,
        });

        const tStoreStart = performance.now();
        openDoc({
          path,
          text: file.text,
          fileType: inferFileType(path),
          encoding: file.encoding,
          hadBom: file.hadBom,
          chars: countWords(file.text),
          lines: linesCount,
        });
        markReaderOpenFrontend(path, "frontendStoreMs", performance.now() - tStoreStart);

        const tDiffsStart = performance.now();
        await syncPendingDiffsForPath(activeId, path);
        markReaderOpenBackend(
          path,
          "backendDiffsMs",
          performance.now() - tDiffsStart,
          { pendingDiffsCount: useReader.getState().getTabByPath(path)?.pendingDiffs.length ?? 0 },
        );
      })
      .catch((err) => {
        if (!cancelled) setLoadError(String(err));
      });
    return () => {
      cancelled = true;
    };
  }, [path, tab, activeId, openDoc]);

  return loadError;
}
