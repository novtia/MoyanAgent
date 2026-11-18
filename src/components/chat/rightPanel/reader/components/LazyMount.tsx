import { useEffect, useRef, useState, type ReactNode } from "react";

type VisibleListener = () => void;

const PRELOAD_PX = 900;
/** Mount a little before a segment scrolls into view so it is ready in time. */
const PRELOAD_MARGIN = `${PRELOAD_PX}px 0px`;

interface RootObserver {
  io: IntersectionObserver;
  listeners: Map<Element, VisibleListener>;
  raf: number;
  cleanupScroll: () => void;
}

/** One IntersectionObserver + scroll fallback per scroll root (hundreds of segments share it). */
const rootObservers = new WeakMap<Element, RootObserver>();

function isNearViewport(root: Element, target: Element): boolean {
  const rootRect = root.getBoundingClientRect();
  if (rootRect.height <= 0 || rootRect.width <= 0) return false;
  const rect = target.getBoundingClientRect();
  return (
    rect.bottom >= rootRect.top - PRELOAD_PX &&
    rect.top <= rootRect.bottom + PRELOAD_PX
  );
}

function flushVisible(entry: RootObserver, root: Element) {
  for (const [el, cb] of Array.from(entry.listeners.entries())) {
    if (!isNearViewport(root, el)) continue;
    entry.listeners.delete(el);
    entry.io.unobserve(el);
    cb();
  }
}

function observeNearViewport(
  root: Element,
  target: Element,
  onNear: VisibleListener,
): () => void {
  if (isNearViewport(root, target)) {
    onNear();
    return () => {};
  }

  let entry = rootObservers.get(root);
  if (!entry) {
    const listeners = new Map<Element, VisibleListener>();
    const io = new IntersectionObserver(
      (records) => {
        for (const rec of records) {
          if (!rec.isIntersecting) continue;
          const cb = listeners.get(rec.target);
          if (!cb) continue;
          listeners.delete(rec.target);
          io.unobserve(rec.target);
          cb();
        }
      },
      { root, rootMargin: PRELOAD_MARGIN },
    );
    const created: RootObserver = {
      io,
      listeners,
      raf: 0,
      cleanupScroll: () => {},
    };
    const onScrollOrResize = () => {
      if (created.raf) return;
      created.raf = requestAnimationFrame(() => {
        created.raf = 0;
        flushVisible(created, root);
      });
    };
    root.addEventListener("scroll", onScrollOrResize, { passive: true });
    created.cleanupScroll = () => {
      if (created.raf) cancelAnimationFrame(created.raf);
      root.removeEventListener("scroll", onScrollOrResize);
    };
    entry = created;
    rootObservers.set(root, entry);
  }
  const { io, listeners } = entry;
  listeners.set(target, onNear);
  io.observe(target);
  return () => {
    listeners.delete(target);
    io.unobserve(target);
    if (listeners.size === 0) {
      entry.cleanupScroll();
      io.disconnect();
      rootObservers.delete(root);
    }
  };
}

/**
 * Renders a cheap, estimated-height placeholder until the slot is close to the
 * scroll viewport, then mounts the real (expensive) children once and keeps
 * them. Lets long chapters with hundreds of edits open without building every
 * CodeMirror editor up front.
 */
export function LazyMount({
  root,
  estimateHeight,
  children,
}: {
  /** Scroll container used as the intersection root. */
  root: HTMLElement | null;
  /** Called only while the slot is still a placeholder. */
  estimateHeight: () => number;
  /** Render function so unmounted slots never build their subtree. */
  children: () => ReactNode;
}) {
  const placeholderRef = useRef<HTMLDivElement>(null);
  const [mounted, setMounted] = useState(() => typeof IntersectionObserver === "undefined");

  useEffect(() => {
    if (mounted || !root) return;
    const el = placeholderRef.current;
    if (!el) return;
    return observeNearViewport(root, el, () => setMounted(true));
  }, [mounted, root]);

  if (mounted) return <>{children()}</>;
  return (
    <div
      ref={placeholderRef}
      className="reader-lazy-placeholder"
      style={{ height: Math.max(0, Math.round(estimateHeight())) }}
      aria-hidden="true"
    />
  );
}
