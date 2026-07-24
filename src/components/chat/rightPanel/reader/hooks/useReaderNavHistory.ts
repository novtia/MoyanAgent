import { useCallback, useEffect, useReducer, useRef } from "react";
import {
  applyReaderPathOpsToPath,
  normalizeReaderPath,
  useReader,
} from "../../../../../store/reader";

export function useReaderNavHistory(
  path: string | null | undefined,
  onOpenFile: (path: string) => void,
) {
  const histRef = useRef<{ stack: string[]; index: number }>({ stack: [], index: -1 });
  const navPendingRef = useRef(false);
  const [, bumpNav] = useReducer((x: number) => x + 1, 0);

  // Rewrite history entries when files are renamed/moved/deleted.
  const readerPathSeq = useReader((s) => s.pathSeq);
  const lastHistPathSeq = useRef(readerPathSeq);
  useEffect(() => {
    if (readerPathSeq === lastHistPathSeq.current) return;
    lastHistPathSeq.current = readerPathSeq;
    const ops = useReader.getState().lastPathOps;
    if (!ops.length) return;
    const h = histRef.current;
    const nextStack: string[] = [];
    for (const p of h.stack) {
      const rewritten = applyReaderPathOpsToPath(p, ops);
      if (rewritten == null || rewritten === "") continue;
      const key = normalizeReaderPath(rewritten);
      if (nextStack.some((x) => normalizeReaderPath(x) === key)) continue;
      nextStack.push(rewritten);
    }
    let index = h.index;
    if (nextStack.length === 0) {
      h.stack = [];
      h.index = -1;
    } else {
      index = Math.max(0, Math.min(index, nextStack.length - 1));
      // Prefer landing on the current workspace path if it survived.
      if (path) {
        const at = nextStack.findIndex(
          (p) => normalizeReaderPath(p) === normalizeReaderPath(path),
        );
        if (at >= 0) index = at;
      }
      h.stack = nextStack;
      h.index = index;
    }
    bumpNav();
  }, [readerPathSeq, path]);

  useEffect(() => {
    if (!path) return;
    const h = histRef.current;
    if (navPendingRef.current) {
      navPendingRef.current = false;
      bumpNav();
      return;
    }
    const cur = h.index >= 0 ? h.stack[h.index] : null;
    if (cur && normalizeReaderPath(cur) === normalizeReaderPath(path)) return;
    h.stack = h.stack.slice(0, h.index + 1);
    h.stack.push(path);
    h.index = h.stack.length - 1;
    bumpNav();
  }, [path]);

  const canBack = histRef.current.index > 0;
  const canForward = histRef.current.index < histRef.current.stack.length - 1;

  const goBack = useCallback(() => {
    const h = histRef.current;
    if (h.index <= 0) return;
    h.index -= 1;
    navPendingRef.current = true;
    bumpNav();
    onOpenFile(h.stack[h.index]!);
  }, [onOpenFile]);

  const goForward = useCallback(() => {
    const h = histRef.current;
    if (h.index >= h.stack.length - 1) return;
    h.index += 1;
    navPendingRef.current = true;
    bumpNav();
    onOpenFile(h.stack[h.index]!);
  }, [onOpenFile]);

  return { canBack, canForward, goBack, goForward };
}
