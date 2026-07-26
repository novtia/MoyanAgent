import { useCallback, useEffect, useRef, useState } from "react";

import {
  createSpring,
  prefersReducedMotion,
  setSpringTarget,
  stepSpring,
  type SpringState,
} from "./spring";

const DRAG_THRESHOLD_PX = 5;

export type RoleDragDropZone = "list" | "composer" | "none";

export interface RoleListReorderHandlers {
  onReorder: (orderedIds: string[]) => void;
  onCite: (id: string) => void;
}

export interface RoleListReorderApi {
  listRef: React.RefObject<HTMLDivElement>;
  /** Slot wrapper style (keeps gap + sibling spring offsets). */
  itemStyle: (id: string) => React.CSSProperties;
  /** Floating card style while dragging (fixed, follows pointer). */
  floatingStyle: (id: string) => React.CSSProperties | undefined;
  isDragging: boolean;
  draggingId: string | null;
  dropZone: RoleDragDropZone;
  onCardPointerDown: (id: string, e: React.PointerEvent) => void;
}

function hitDropZone(clientX: number, clientY: number): RoleDragDropZone {
  const el = document.elementFromPoint(clientX, clientY);
  if (!el) return "none";
  // Composer first — it sits outside the right panel.
  if (el.closest(".composer-card")) return "composer";
  // Anywhere in the role archive panel counts as a reorder drop target.
  if (el.closest(".arc-panel")) return "list";
  return "none";
}

function setComposerDragOver(on: boolean) {
  document.querySelector(".composer-card")?.classList.toggle("drag-over", on);
}

/**
 * Unified pointer drag: card follows the cursor anywhere in the window.
 * Drop on the role list → reorder; drop on composer → cite; else cancel.
 */
export function useRoleListReorder(
  ids: string[],
  handlers: RoleListReorderHandlers,
): RoleListReorderApi {
  const listRef = useRef<HTMLDivElement>(null);
  const idsRef = useRef(ids);
  idsRef.current = ids;
  const handlersRef = useRef(handlers);
  handlersRef.current = handlers;

  const [draggingId, setDraggingId] = useState<string | null>(null);
  const [dropZone, setDropZone] = useState<RoleDragDropZone>("none");
  const [tick, setTick] = useState(0);

  const offsetsRef = useRef<Map<string, SpringState>>(new Map());
  const scaleRef = useRef(createSpring(1));
  const originIndexRef = useRef(0);
  const hoverIndexRef = useRef(0);
  const itemTopsRef = useRef<number[]>([]);
  const itemHeightsRef = useRef<number[]>([]);
  const itemWidthRef = useRef(0);
  const dragHeightRef = useRef(0);
  const pointerOriginXRef = useRef(0);
  const pointerOriginYRef = useRef(0);
  const grabOffsetXRef = useRef(0);
  const grabOffsetYRef = useRef(0);
  const floatLeftRef = useRef(0);
  const floatTopRef = useRef(0);
  const rafRef = useRef(0);
  const activeRef = useRef(false);
  const armedRef = useRef(false);
  const dropZoneRef = useRef<RoleDragDropZone>("none");

  const ensureSprings = useCallback((list: string[]) => {
    const map = offsetsRef.current;
    for (const id of list) {
      if (!map.has(id)) map.set(id, createSpring(0));
    }
    for (const id of [...map.keys()]) {
      if (!list.includes(id)) map.delete(id);
    }
  }, []);

  useEffect(() => {
    ensureSprings(ids);
  }, [ids, ensureSprings]);

  const measure = useCallback(() => {
    const root = listRef.current;
    if (!root) return;
    const nodes = Array.from(root.querySelectorAll<HTMLElement>("[data-role-id]"));
    const order = idsRef.current;
    const tops: number[] = [];
    const heights: number[] = [];
    const rootTop = root.getBoundingClientRect().top;
    let width = 0;
    for (const id of order) {
      const el = nodes.find((n) => n.dataset.roleId === id);
      if (!el) {
        tops.push(0);
        heights.push(0);
        continue;
      }
      const rect = el.getBoundingClientRect();
      tops.push(rect.top - rootTop + root.scrollTop);
      heights.push(rect.height);
      if (!width) width = rect.width;
    }
    itemTopsRef.current = tops;
    itemHeightsRef.current = heights;
    itemWidthRef.current = width;
  }, []);

  const slotOffsetFor = useCallback((from: number, to: number, index: number, height: number) => {
    if (from === to) return 0;
    if (from < to) {
      if (index > from && index <= to) return -height;
      return 0;
    }
    if (index >= to && index < from) return height;
    return 0;
  }, []);

  const stopLoop = useCallback(() => {
    if (rafRef.current) {
      cancelAnimationFrame(rafRef.current);
      rafRef.current = 0;
    }
  }, []);

  const startLoop = useCallback(() => {
    stopLoop();
    let last = performance.now();
    const frame = (now: number) => {
      const dt = (now - last) / 1000;
      last = now;
      let moving = false;
      for (const spring of offsetsRef.current.values()) {
        if (stepSpring(spring, dt)) moving = true;
      }
      if (stepSpring(scaleRef.current, dt)) moving = true;
      setTick((n) => n + 1);
      if (moving || activeRef.current) {
        rafRef.current = requestAnimationFrame(frame);
      } else {
        rafRef.current = 0;
      }
    };
    rafRef.current = requestAnimationFrame(frame);
  }, [stopLoop]);

  const applySiblingTargets = useCallback(
    (from: number, to: number, dragHeight: number, hard = false) => {
      const order = idsRef.current;
      const dragId = order[from];
      for (let i = 0; i < order.length; i++) {
        const id = order[i];
        const spring = offsetsRef.current.get(id);
        if (!spring) continue;
        if (id === dragId) {
          if (activeRef.current) setSpringTarget(spring, 0, true);
          continue;
        }
        const target = slotOffsetFor(from, to, i, dragHeight);
        setSpringTarget(spring, target, hard);
      }
    },
    [slotOffsetFor],
  );

  const indexFromPointerY = useCallback((clientY: number) => {
    const root = listRef.current;
    if (!root) return 0;
    const rootTop = root.getBoundingClientRect().top;
    const y = clientY - rootTop + root.scrollTop;
    const heights = itemHeightsRef.current;
    const tops = itemTopsRef.current;
    if (!heights.length) return 0;
    for (let i = 0; i < heights.length; i++) {
      const mid = tops[i] + heights[i] / 2;
      if (y < mid) return i;
    }
    return heights.length - 1;
  }, []);

  const resetSiblingSprings = useCallback(() => {
    for (const spring of offsetsRef.current.values()) {
      setSpringTarget(spring, 0, true);
    }
  }, []);

  const finishDrag = useCallback(
    (clientX: number, clientY: number) => {
      if (!activeRef.current) return;
      activeRef.current = false;
      armedRef.current = false;

      const order = [...idsRef.current];
      const from = originIndexRef.current;
      const to = hoverIndexRef.current;
      const dragId = order[from];
      const zone = hitDropZone(clientX, clientY);

      setComposerDragOver(false);
      dropZoneRef.current = "none";
      setDropZone("none");
      setSpringTarget(scaleRef.current, 1, false);

      if (zone === "composer" && dragId) {
        resetSiblingSprings();
        floatLeftRef.current = 0;
        floatTopRef.current = 0;
        setDraggingId(null);
        handlersRef.current.onCite(dragId);
        startLoop();
        return;
      }

      if (zone === "list" && dragId && from !== to) {
        const next = [...order];
        const [item] = next.splice(from, 1);
        next.splice(to, 0, item);
        resetSiblingSprings();
        floatLeftRef.current = 0;
        floatTopRef.current = 0;
        setDraggingId(null);
        handlersRef.current.onReorder(next);
        startLoop();
        return;
      }

      // Cancel / same slot / drop outside list
      floatLeftRef.current = 0;
      floatTopRef.current = 0;
      applySiblingTargets(from, from, dragHeightRef.current, false);
      setDraggingId(null);
      startLoop();
    },
    [applySiblingTargets, resetSiblingSprings, startLoop],
  );

  const beginDrag = useCallback(
    (id: string, clientX: number, clientY: number) => {
      const order = idsRef.current;
      const index = order.indexOf(id);
      if (index < 0) return;

      measure();
      ensureSprings(order);
      originIndexRef.current = index;
      hoverIndexRef.current = index;
      dragHeightRef.current = itemHeightsRef.current[index] ?? 0;
      activeRef.current = true;
      setDraggingId(id);
      setSpringTarget(scaleRef.current, prefersReducedMotion() ? 1 : 1.03, false);

      floatLeftRef.current = clientX - grabOffsetXRef.current;
      floatTopRef.current = clientY - grabOffsetYRef.current;
      dropZoneRef.current = "list";
      setDropZone("list");

      applySiblingTargets(index, index, dragHeightRef.current, true);
      startLoop();
    },
    [applySiblingTargets, ensureSprings, measure, startLoop],
  );

  const onCardPointerDown = useCallback(
    (id: string, e: React.PointerEvent) => {
      if (e.button !== 0) return;
      const target = e.target as HTMLElement | null;
      if (target?.closest("button, a, input, textarea, select, label, [role='button']")) {
        return;
      }

      const order = idsRef.current;
      const index = order.indexOf(id);
      if (index < 0) return;

      const slot = (e.currentTarget as HTMLElement).closest("[data-role-id]") as HTMLElement | null;
      const rect = (slot ?? (e.currentTarget as HTMLElement)).getBoundingClientRect();
      grabOffsetXRef.current = e.clientX - rect.left;
      grabOffsetYRef.current = e.clientY - rect.top;
      pointerOriginXRef.current = e.clientX;
      pointerOriginYRef.current = e.clientY;
      armedRef.current = true;
      activeRef.current = false;

      const pointerId = e.pointerId;

      const onMove = (ev: PointerEvent) => {
        if (!armedRef.current) return;
        const dx = ev.clientX - pointerOriginXRef.current;
        const dy = ev.clientY - pointerOriginYRef.current;

        if (!activeRef.current) {
          if (Math.hypot(dx, dy) < DRAG_THRESHOLD_PX) return;
          beginDrag(id, ev.clientX, ev.clientY);
          try {
            (e.currentTarget as HTMLElement).setPointerCapture?.(pointerId);
          } catch {
            /* ignore */
          }
        }

        if (!activeRef.current) return;
        floatLeftRef.current = ev.clientX - grabOffsetXRef.current;
        floatTopRef.current = ev.clientY - grabOffsetYRef.current;

        const zone = hitDropZone(ev.clientX, ev.clientY);
        if (zone !== dropZoneRef.current) {
          dropZoneRef.current = zone;
          setDropZone(zone);
          setComposerDragOver(zone === "composer");
        }

        if (zone === "list") {
          const to = indexFromPointerY(ev.clientY);
          if (to !== hoverIndexRef.current) {
            hoverIndexRef.current = to;
            applySiblingTargets(originIndexRef.current, to, dragHeightRef.current, false);
          }
        } else {
          // Outside list: clear slot preview
          if (hoverIndexRef.current !== originIndexRef.current) {
            hoverIndexRef.current = originIndexRef.current;
            applySiblingTargets(
              originIndexRef.current,
              originIndexRef.current,
              dragHeightRef.current,
              false,
            );
          }
        }
        startLoop();
      };

      const onUp = (ev: PointerEvent) => {
        window.removeEventListener("pointermove", onMove);
        window.removeEventListener("pointerup", onUp);
        window.removeEventListener("pointercancel", onUp);
        const wasActive = activeRef.current;
        armedRef.current = false;
        if (wasActive) {
          finishDrag(ev.clientX, ev.clientY);
        }
      };

      window.addEventListener("pointermove", onMove);
      window.addEventListener("pointerup", onUp);
      window.addEventListener("pointercancel", onUp);
    },
    [applySiblingTargets, beginDrag, finishDrag, indexFromPointerY, startLoop],
  );

  useEffect(() => () => {
    stopLoop();
    setComposerDragOver(false);
  }, [stopLoop]);

  const itemStyle = useCallback(
    (id: string): React.CSSProperties => {
      const spring = offsetsRef.current.get(id);
      const base = spring?.current ?? 0;
      const isDrag = draggingId === id;
      if (isDrag) {
        return {
          height: dragHeightRef.current || undefined,
          position: "relative",
          zIndex: 1,
          transition: "none",
        };
      }
      return {
        transform: `translate3d(0, ${base}px, 0)`,
        zIndex: 1,
        position: "relative",
        willChange: draggingId ? "transform" : undefined,
        transition: "none",
      };
    },
    [draggingId],
  );

  const floatingStyle = useCallback(
    (id: string): React.CSSProperties | undefined => {
      if (draggingId !== id) return undefined;
      const scale = scaleRef.current.current;
      const width = itemWidthRef.current;
      return {
        position: "fixed",
        left: floatLeftRef.current,
        top: floatTopRef.current,
        width: width > 0 ? width : undefined,
        maxWidth: "min(360px, calc(100vw - 24px))",
        zIndex: 2147483000,
        margin: 0,
        pointerEvents: "none",
        transform: `translate3d(0,0,0) scale(${scale})`,
        transformOrigin: "top left",
        willChange: "left, top, transform",
        transition: "none",
        // Ensure the portaled layer is painted above app chrome
        isolation: "isolate",
      };
    },
    // tick forces fresh left/top/scale from refs each animation frame
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [draggingId, tick],
  );

  return {
    listRef,
    itemStyle,
    floatingStyle,
    isDragging: draggingId != null,
    draggingId,
    dropZone,
    onCardPointerDown,
  };
}
