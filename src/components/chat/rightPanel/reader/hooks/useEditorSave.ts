import { useCallback, useEffect, useRef } from "react";
import { api } from "../../../../../api/tauri";
import { useReader, type ReaderFileTab } from "../../../../../store/reader";
import { useSession } from "../../../../../store/session";
import { SAVE_DEBOUNCE_MS } from "../constants";

export function useEditorSave(tab: ReaderFileTab) {
  const sessionId = useSession((s) => s.activeId);
  const updateTabText = useReader((s) => s.updateTabText);
  const setTabDirty = useReader((s) => s.setTabDirty);
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const latestTextRef = useRef(tab.text);
  const dirtyRef = useRef(false);

  useEffect(() => {
    latestTextRef.current = tab.text;
    if (!tab.dirty) dirtyRef.current = false;
  }, [tab.text, tab.dirty]);

  const flushSave = useCallback(
    async (text: string) => {
      if (!sessionId || !tab.path) return;
      try {
        await api.writeProjectFile(sessionId, tab.path, text, tab.encoding, tab.hadBom);
        dirtyRef.current = false;
        setTabDirty(tab.path, false, false);
      } catch {
        setTabDirty(tab.path, true, true);
      }
    },
    [sessionId, tab.path, tab.encoding, tab.hadBom, setTabDirty],
  );

  const scheduleSave = useCallback(
    (text: string) => {
      if (timerRef.current) clearTimeout(timerRef.current);
      timerRef.current = setTimeout(() => {
        timerRef.current = null;
        void flushSave(text);
      }, SAVE_DEBOUNCE_MS);
    },
    [flushSave],
  );

  useEffect(() => {
    return () => {
      if (timerRef.current) {
        clearTimeout(timerRef.current);
        timerRef.current = null;
      }
      if (dirtyRef.current && sessionId && tab.path) {
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
  }, [sessionId, tab.path, setTabDirty]);

  const applyText = useCallback(
    (text: string) => {
      latestTextRef.current = text;
      dirtyRef.current = true;
      updateTabText(tab.path, text, { dirty: true });
      scheduleSave(text);
    },
    [tab.path, updateTabText, scheduleSave],
  );

  return applyText;
}
