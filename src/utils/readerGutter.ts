/** Measure wrapped visual height for each logical line at a given content width. */
export function measureWrappedLineHeights(
  lines: string[],
  contentWidth: number,
  styleSource: HTMLElement,
): number[] {
  if (contentWidth <= 0 || lines.length === 0) return lines.map(() => 0);

  const cs = getComputedStyle(styleSource);
  const probe = document.createElement("div");
  probe.style.position = "absolute";
  probe.style.visibility = "hidden";
  probe.style.pointerEvents = "none";
  probe.style.left = "-9999px";
  probe.style.top = "0";
  probe.style.width = `${contentWidth}px`;
  probe.style.fontFamily = cs.fontFamily;
  probe.style.fontSize = cs.fontSize;
  probe.style.fontWeight = cs.fontWeight;
  probe.style.lineHeight = cs.lineHeight;
  probe.style.letterSpacing = cs.letterSpacing;
  probe.style.whiteSpace = "pre-wrap";
  probe.style.wordBreak = "break-word";
  probe.style.overflowWrap = "break-word";
  probe.style.boxSizing = "border-box";
  document.body.appendChild(probe);

  const heights = lines.map((line) => {
    probe.textContent = line.length > 0 ? line : "\u00a0";
    return probe.offsetHeight;
  });

  document.body.removeChild(probe);
  return heights;
}

/** Textarea inner width available for line wrapping. */
export function textareaContentWidth(el: HTMLTextAreaElement): number {
  const cs = getComputedStyle(el);
  return (
    el.clientWidth -
    parseFloat(cs.paddingLeft) -
    parseFloat(cs.paddingRight)
  );
}
