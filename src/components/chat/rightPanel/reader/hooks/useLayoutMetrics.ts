import { useLayoutEffect, useState } from "react";

export interface ReaderLayoutMetrics {
  lineHeight: number;
  /** Approximate width of one CJK glyph at the reader font size. */
  charWidth: number;
  /** Width available to text (scroll width minus gutters). */
  usableWidth: number;
}

const FALLBACK: ReaderLayoutMetrics = { lineHeight: 26, charWidth: 15, usableWidth: 420 };

/** Rough height of one document line once soft-wrapped (placeholder sizing only). */
export function estimateLineHeight(charCount: number, m: ReaderLayoutMetrics): number {
  const rows = Math.max(1, Math.ceil((charCount * m.charWidth) / Math.max(1, m.usableWidth)));
  return rows * m.lineHeight;
}

function measure(root: HTMLElement): ReaderLayoutMetrics {
  const probe = document.createElement("div");
  probe.style.cssText = [
    "position:absolute",
    "visibility:hidden",
    "pointer-events:none",
    "white-space:pre",
    "font-size:var(--chat-font-size,14px)",
    "font-family:var(--chat-font-family,inherit)",
    "line-height:var(--chat-line-height,1.6)",
  ].join(";");
  probe.textContent = "汉".repeat(20);
  root.appendChild(probe);
  const rect = probe.getBoundingClientRect();
  const fontSize = parseFloat(getComputedStyle(probe).fontSize) || 14;
  root.removeChild(probe);

  const charWidth = rect.width > 0 ? rect.width / 20 : fontSize;
  const lineHeight = rect.height > 0 ? rect.height : fontSize * 1.6;
  // Line-number + diff-sign gutters take roughly 4.5em.
  const usableWidth = Math.max(charWidth * 4, root.clientWidth - fontSize * 4.5);
  return { lineHeight, charWidth, usableWidth };
}

/**
 * Measures reader typography for estimating placeholder heights of
 * not-yet-mounted segments. Re-measures when the scroll root is resized.
 */
export function useLayoutMetrics(root: HTMLElement | null): ReaderLayoutMetrics {
  const [metrics, setMetrics] = useState<ReaderLayoutMetrics>(FALLBACK);

  useLayoutEffect(() => {
    if (!root) return;
    const apply = () => {
      const next = measure(root);
      setMetrics((prev) =>
        Math.abs(prev.lineHeight - next.lineHeight) < 0.5 &&
        Math.abs(prev.charWidth - next.charWidth) < 0.5 &&
        Math.abs(prev.usableWidth - next.usableWidth) < 4
          ? prev
          : next,
      );
    };
    apply();
    if (typeof ResizeObserver === "undefined") return;
    let frame = 0;
    const ro = new ResizeObserver(() => {
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(apply);
    });
    ro.observe(root);
    return () => {
      cancelAnimationFrame(frame);
      ro.disconnect();
    };
  }, [root]);

  return metrics;
}
