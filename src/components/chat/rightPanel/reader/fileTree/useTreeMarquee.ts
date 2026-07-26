import {
  useCallback,
  useRef,
  useState,
  type PointerEvent as ReactPointerEvent,
  type RefObject,
} from "react";
import { normalizeReaderPath } from "../../../../../store/reader";
import { useFileExplorer } from "../../../../../store/fileExplorer";
import { TREE_PATH_ATTR } from "./selection";

export interface MarqueeRect {
  left: number;
  top: number;
  width: number;
  height: number;
}

const THRESHOLD_PX = 4;

function rectsIntersect(a: DOMRect, b: {
  left: number;
  top: number;
  right: number;
  bottom: number;
}): boolean {
  return !(
    a.right < b.left ||
    a.left > b.right ||
    a.bottom < b.top ||
    a.top > b.bottom
  );
}

/**
 * Rubber-band selection on the file-tree body.
 * Starts only when pointerdown is not on a `.reader-file-row` (avoids fighting drag).
 */
export function useTreeMarquee(bodyRef: RefObject<HTMLElement | null>) {
  const [marquee, setMarquee] = useState<MarqueeRect | null>(null);
  const originRef = useRef<{ x: number; y: number; additive: boolean } | null>(
    null,
  );
  const activeRef = useRef(false);
  const baseSelectionRef = useRef<string[]>([]);

  const finish = useCallback(() => {
    originRef.current = null;
    activeRef.current = false;
    setMarquee(null);
  }, []);

  const onPointerDown = useCallback(
    (e: ReactPointerEvent<HTMLElement>) => {
      if (e.button !== 0) return;
      const target = e.target as HTMLElement | null;
      if (target?.closest(".reader-file-row")) return;
      const body = bodyRef.current;
      if (!body) return;

      const additive = e.ctrlKey || e.metaKey;
      baseSelectionRef.current = additive
        ? [...useFileExplorer.getState().selectedPaths]
        : [];
      originRef.current = {
        x: e.clientX,
        y: e.clientY,
        additive,
      };
      activeRef.current = false;
      body.setPointerCapture(e.pointerId);
    },
    [bodyRef],
  );

  const onPointerMove = useCallback(
    (e: ReactPointerEvent<HTMLElement>) => {
      const origin = originRef.current;
      const body = bodyRef.current;
      if (!origin || !body) return;

      const dx = e.clientX - origin.x;
      const dy = e.clientY - origin.y;
      if (!activeRef.current) {
        if (Math.hypot(dx, dy) < THRESHOLD_PX) return;
        activeRef.current = true;
        if (!origin.additive) {
          useFileExplorer.getState().clearSelection();
        }
      }

      e.preventDefault();
      const bodyRect = body.getBoundingClientRect();
      const left = Math.min(origin.x, e.clientX) - bodyRect.left + body.scrollLeft;
      const top = Math.min(origin.y, e.clientY) - bodyRect.top + body.scrollTop;
      const width = Math.abs(dx);
      const height = Math.abs(dy);
      setMarquee({ left, top, width, height });

      const selBox = {
        left: Math.min(origin.x, e.clientX),
        top: Math.min(origin.y, e.clientY),
        right: Math.max(origin.x, e.clientX),
        bottom: Math.max(origin.y, e.clientY),
      };

      const hit: string[] = [];
      body.querySelectorAll<HTMLElement>(`[${TREE_PATH_ATTR}]`).forEach((el) => {
        const path = el.getAttribute(TREE_PATH_ATTR);
        if (!path) return;
        if (rectsIntersect(el.getBoundingClientRect(), selBox)) {
          hit.push(path);
        }
      });

      if (origin.additive) {
        const merged = new Map<string, string>();
        for (const p of baseSelectionRef.current) {
          merged.set(normalizeReaderPath(p), p);
        }
        for (const p of hit) merged.set(normalizeReaderPath(p), p);
        useFileExplorer.getState().setSelectedPaths(Array.from(merged.values()));
      } else {
        useFileExplorer.getState().setSelectedPaths(hit);
      }
    },
    [bodyRef],
  );

  const onPointerUp = useCallback(
    (e: ReactPointerEvent<HTMLElement>) => {
      const body = bodyRef.current;
      if (body?.hasPointerCapture(e.pointerId)) {
        body.releasePointerCapture(e.pointerId);
      }
      if (!activeRef.current && originRef.current && !originRef.current.additive) {
        // Click on empty area — clear selection.
        useFileExplorer.getState().clearSelection();
      }
      finish();
    },
    [bodyRef, finish],
  );

  const onPointerCancel = useCallback(() => {
    finish();
  }, [finish]);

  return {
    marquee,
    marqueeHandlers: {
      onPointerDown,
      onPointerMove,
      onPointerUp,
      onPointerCancel,
    },
  };
}
