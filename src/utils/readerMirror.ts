/** Shared mirror-div typography matching a textarea's rendered text. */

export function textareaContentWidth(el: HTMLTextAreaElement): number {
  const cs = getComputedStyle(el);
  return (
    el.clientWidth -
    parseFloat(cs.paddingLeft) -
    parseFloat(cs.paddingRight)
  );
}

export function applyMirrorTypography(
  el: HTMLElement,
  mirror: HTMLDivElement,
  width: number,
) {
  const cs = getComputedStyle(el);
  mirror.style.position = "absolute";
  mirror.style.visibility = "hidden";
  mirror.style.pointerEvents = "none";
  mirror.style.left = "-9999px";
  mirror.style.top = "0";
  mirror.style.width = `${width}px`;
  mirror.style.margin = "0";
  mirror.style.padding = "0";
  mirror.style.border = "0";
  mirror.style.fontFamily = cs.fontFamily;
  mirror.style.fontSize = cs.fontSize;
  mirror.style.fontWeight = cs.fontWeight;
  mirror.style.lineHeight = cs.lineHeight;
  mirror.style.letterSpacing = cs.letterSpacing;
  mirror.style.whiteSpace = "pre-wrap";
  mirror.style.wordBreak = "break-word";
  mirror.style.overflowWrap = "break-word";
  mirror.style.boxSizing = "border-box";
}

/** Pixel offset of a character index inside textarea content (handles wrapping). */
export function measureTextareaCharOffsetTop(
  el: HTMLTextAreaElement,
  index: number,
  text?: string,
): number {
  const contentWidth = textareaContentWidth(el);
  if (contentWidth <= 0) return 0;

  const source = text ?? el.value;
  const clamped = Math.max(0, Math.min(index, source.length));
  const mirror = document.createElement("div");
  applyMirrorTypography(el, mirror, contentWidth);

  const before = source.slice(0, clamped);
  const after = source.slice(clamped);
  const marker = document.createElement("span");
  marker.textContent = "\u200b";
  mirror.append(document.createTextNode(before), marker, document.createTextNode(after));

  document.body.appendChild(mirror);
  const top = marker.offsetTop;
  document.body.removeChild(mirror);
  return top;
}

/** Total rendered height of text in a textarea mirror. */
export function measureTextareaContentHeight(
  el: HTMLTextAreaElement,
  text: string,
): number {
  const contentWidth = textareaContentWidth(el);
  if (contentWidth <= 0) return 0;

  const mirror = document.createElement("div");
  applyMirrorTypography(el, mirror, contentWidth);
  mirror.textContent = text;

  document.body.appendChild(mirror);
  const height = mirror.offsetHeight;
  document.body.removeChild(mirror);
  return height;
}

/** Character index where each logical line starts within `text`. */
export function logicalLineStartIndices(lines: string[]): number[] {
  const starts: number[] = [];
  let offset = 0;
  for (let i = 0; i < lines.length; i++) {
    starts.push(offset);
    offset += lines[i].length + (i < lines.length - 1 ? 1 : 0);
  }
  return starts;
}
