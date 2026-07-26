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

/** Lazily load a restored / freshly-selected file whose content isn't cached yet. */
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
      openDoc(
        { path, text: "", fileType, chars: 0, lines: 0 },
        { activate: true },
      );
      return;
    }

    if (tab) return;

    let cancelled = false;
    setLoadError(null);

    if (isMediaFileType(fileType)) {
      // Media tabs do not decode bytes — the viewer loads via asset protocol.
      openDoc(
        {
          path,
          text: "",
          fileType,
          chars: 0,
          lines: 0,
        },
        { activate: true },
      );
      return;
    }

    api
      .readProjectFile(activeId, path)
      .then(async (file) => {
        if (cancelled) return;
        // Activate: the panel is already showing this path, so the reader
        // store's active tab must match — otherwise find/replace searches and
        // highlights the wrong file.
        openDoc(
          {
            path,
            text: file.text,
            fileType: inferFileType(path),
            encoding: file.encoding,
            hadBom: file.hadBom,
            chars: countWords(file.text),
            lines: file.text.split("\n").length,
          },
          { activate: true },
        );
        await syncPendingDiffsForPath(activeId, path);
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
