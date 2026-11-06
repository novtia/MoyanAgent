import { useCallback, useEffect, useRef } from "react";
import { api } from "../../../../../api/tauri";
import {
  isMediaFileType,
  normalizeReaderPath,
  useReader,
  type ReaderFileTab,
} from "../../../../../store/reader";
import { useSession } from "../../../../../store/session";
import { SAVE_DEBOUNCE_MS } from "../constants";

let saveNowHandler: (() => void) | null = null;

/** Bumped when a tool writes a file, so a debounce already in flight does not
 * put the editor's old text back on top of that write. */
const saveEpoch = new Map<string, number>();
const saveCancelers = new Map<string, () => void>();

function saveKey(path: string): string {
  return normalizeReaderPath(path);
}

/** Drop a pending autosave for `path`. Call this before reloading a tab after
 * a tool has already written the file. */
export function cancelReaderSave(path: string) {
  const key = saveKey(path);
  if (!key) return;
  saveEpoch.set(key, (saveEpoch.get(key) ?? 0) + 1);
  saveCancelers.get(key)?.();
}

export function registerReaderSave(fn: () => void): () => void {
  saveNowHandler = fn;
  return () => {
    if (saveNowHandler === fn) saveNowHandler = null;
  };
}

/** Flush the open document. Always returns true so Ctrl/Cmd+S is consumed. */
export function runRegisteredReaderSave(): boolean {
  saveNowHandler?.();
  return true;
}

export function useEditorSave(tab: ReaderFileTab) {
  const sessionId = useSession((s) => s.activeId);
  const updateTabText = useReader((s) => s.updateTabText);
  const setTabDirty = useReader((s) => s.setTabDirty);
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const latestTextRef = useRef(tab.text);
  const dirtyRef = useRef(false);
  const savingRef = useRef(false);
  const media = isMediaFileType(tab.fileType);

  useEffect(() => {
    latestTextRef.current = tab.text;
    if (!tab.dirty) {
      dirtyRef.current = false;
      if (timerRef.current) {
        clearTimeout(timerRef.current);
        timerRef.current = null;
      }
    }
  }, [tab.text, tab.dirty]);

  useEffect(() => {
    const key = saveKey(tab.path);
    const cancel = () => {
      if (timerRef.current) {
        clearTimeout(timerRef.current);
        timerRef.current = null;
      }
      dirtyRef.current = false;
    };
    saveCancelers.set(key, cancel);
    return () => {
      if (saveCancelers.get(key) === cancel) saveCancelers.delete(key);
    };
  }, [tab.path]);

  const flushSave = useCallback(
    async (text: string) => {
      if (media || !sessionId || !tab.path) return;
      const key = saveKey(tab.path);
      const epoch = saveEpoch.get(key) ?? 0;
      if (!dirtyRef.current) return;
      try {
        if ((saveEpoch.get(key) ?? 0) !== epoch) return;
        await api.writeProjectFile(sessionId, tab.path, text, tab.encoding, tab.hadBom);
        if ((saveEpoch.get(key) ?? 0) !== epoch) return;
        dirtyRef.current = false;
        setTabDirty(tab.path, false, false);
      } catch {
        if ((saveEpoch.get(key) ?? 0) !== epoch) return;
        setTabDirty(tab.path, true, true);
      }
    },
    [media, sessionId, tab.path, tab.encoding, tab.hadBom, setTabDirty],
  );

  const scheduleSave = useCallback(
    (text: string) => {
      if (media) return;
      if (timerRef.current) clearTimeout(timerRef.current);
      timerRef.current = setTimeout(() => {
        timerRef.current = null;
        void flushSave(text);
      }, SAVE_DEBOUNCE_MS);
    },
    [flushSave, media],
  );

  useEffect(() => {
    return () => {
      if (timerRef.current) {
        clearTimeout(timerRef.current);
        timerRef.current = null;
      }
      if (!media && dirtyRef.current && sessionId && tab.path) {
        void api
          .writeProjectFile(
            sessionId,
            tab.path,
            latestTextRef.current,
            tab.encoding,
            tab.hadBom,
          )
          .catch(() => {
            setTabDirty(tab.path, true, true);
          });
      }
    };
  }, [media, sessionId, tab.path, tab.encoding, tab.hadBom, setTabDirty]);

  const saveNow = useCallback(() => {
    if (media || !sessionId || !tab.path || savingRef.current) return;
    if (timerRef.current) {
      clearTimeout(timerRef.current);
      timerRef.current = null;
    }
    if (!dirtyRef.current) return;
    savingRef.current = true;
    void flushSave(latestTextRef.current).finally(() => {
      savingRef.current = false;
    });
  }, [media, sessionId, tab.path, flushSave]);

  useEffect(() => {
    const unreg = registerReaderSave(saveNow);
    const onKeyDown = (e: KeyboardEvent) => {
      if (!(e.ctrlKey || e.metaKey) || e.altKey || e.repeat) return;
      if (e.key.toLowerCase() !== "s") return;
      e.preventDefault();
      saveNow();
    };
    window.addEventListener("keydown", onKeyDown);
    return () => {
      unreg();
      window.removeEventListener("keydown", onKeyDown);
    };
  }, [saveNow]);

  const applyText = useCallback(
    (text: string) => {
      if (media) return;
      latestTextRef.current = text;
      dirtyRef.current = true;
      updateTabText(tab.path, text, { dirty: true });
      scheduleSave(text);
    },
    [media, tab.path, updateTabText, scheduleSave],
  );

  return applyText;
}
