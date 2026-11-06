import { useEffect, useRef, useState, type ReactNode } from "react";

type VisibleListener = () => void;

interface RootObserver {
  io: IntersectionObserver;
  listeners: Map<Element, VisibleListener>;
}

/** One IntersectionObserver per scroll root (hundreds of segments share it). */
const rootObservers = new WeakMap<Element, RootObserver>();

/** Mount a little before a segment scrolls into view so it is ready in time. */
const PRELOAD_MARGIN = "900px 0px";

function observeNearViewport(
  root: Element,
  target: Element,
  onNear: VisibleListener,
): () => void {
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
    entry = { io, listeners };
    rootObservers.set(root, entry);
  }
  const { io, listeners } = entry;
  listeners.set(target, onNear);
  io.observe(target);
  return () => {
    listeners.delete(target);
    io.unobserve(target);
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
