import type { CSSProperties } from "react";
import type { MentionTriggerAnchor } from "./MentionEditor";

const GAP = 8;
const WIDTH = 320;
const MAX_HEIGHT = 380;
/** Prefer the side with at least this much room; otherwise pick the larger side. */
const MIN_COMFORTABLE = 160;

export function scrollableAncestors(el: HTMLElement | null): HTMLElement[] {
  const out: HTMLElement[] = [];
  let node = el?.parentElement ?? null;
  while (node) {
    const oy = getComputedStyle(node).overflowY;
    if (/(auto|scroll|overlay)/.test(oy)) out.push(node);
    node = node.parentElement;
  }
  return out;
}

/**
 * Position a caret-anchored @ mention popover relative to `wrapEl`.
 * Prefers opening above when space below is tight (composer sits at the bottom).
 */
export function computeCaretMentionStyle(
  anchor: MentionTriggerAnchor,
  wrapEl: HTMLElement | null,
): CSSProperties {
  const wrapRect = wrapEl?.getBoundingClientRect();
  const wrapLeft = wrapRect?.left ?? 0;
  const wrapTop = wrapRect?.top ?? 0;
  const wrapBottom = wrapRect?.bottom ?? window.innerHeight;

  const topbar = document.querySelector(".chat-topbar");
  const topLimit =
    (topbar?.getBoundingClientRect().bottom ?? 0) + GAP;
  const shell =
    (document.querySelector(".chat-main") as HTMLElement | null) ??
    document.documentElement;
  const bottomLimit = shell.getBoundingClientRect().bottom - GAP;

  const spaceBelow = Math.floor(bottomLimit - (anchor.bottom + GAP));
  const spaceAbove = Math.floor(anchor.top - GAP - topLimit);

  const openBelow =
    spaceBelow >= MIN_COMFORTABLE ||
    (spaceBelow >= spaceAbove && spaceBelow >= 72);

  const available = openBelow ? spaceBelow : spaceAbove;
  const maxHeight = Math.max(72, Math.min(MAX_HEIGHT, available));

  const left =
    Math.max(12, Math.min(anchor.left, window.innerWidth - WIDTH - 12)) -
    wrapLeft;

  if (openBelow) {
    return {
      left,
      top: anchor.bottom + GAP - wrapTop,
      bottom: "auto",
      maxHeight,
    };
  }

  return {
    left,
    top: "auto",
    bottom: wrapBottom - (anchor.top - GAP),
    maxHeight,
  };
}
