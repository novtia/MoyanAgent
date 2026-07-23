import { useMemo } from "react";
import { themeIcons } from "seti-icons";

/**
 * File-type icons in the Seti-UI style (matching the reference explorer look).
 *
 * `seti-icons` maps a file name to an SVG string + a Seti color keyword; we
 * theme those keywords to concrete colors. Neutral keywords resolve to theme
 * tokens so light/dark both stay legible; vivid keywords use Seti-ish hues.
 */
const getSetiIcon = themeIcons({
  blue: "#519aba",
  grey: "var(--ink-mute)",
  "grey-light": "var(--ink-soft)",
  green: "#8dc149",
  orange: "#e37933",
  pink: "#f55385",
  purple: "#a074c4",
  red: "#cc3e44",
  white: "var(--ink-mute)",
  yellow: "#cbcb41",
  ignore: "var(--ink-mute)",
});

/** Colored, Seti-style file icon. Falls back to the neutral default glyph. */
export function FileTypeIcon({ name, className }: { name: string; className?: string }) {
  const { svg, color } = useMemo(() => getSetiIcon(name || "file"), [name]);
  const sized = useMemo(() => svg.replace("<svg ", '<svg width="16" height="16" '), [svg]);
  return (
    <span
      className={className}
      style={{ color, fill: color, display: "inline-flex", lineHeight: 0 }}
      aria-hidden
      dangerouslySetInnerHTML={{ __html: sized }}
    />
  );
}
