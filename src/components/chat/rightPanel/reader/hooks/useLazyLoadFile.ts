import { useEffect, useState } from "react";
import { api } from "../../../../../api/tauri";
import {
  countWords,
  inferFileType,
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
    if (!path || tab || !activeId) return;
    let cancelled = false;
    setLoadError(null);
    api
      .readProjectFile(activeId, path)
      .then((file) => {
        if (cancelled) return;
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
          { activate: false },
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
