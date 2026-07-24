import { EditorView } from "@codemirror/view";

export type ReaderCodeMirrorLayout = "document" | "segment";

export function createReaderCodeMirrorTheme(layout: ReaderCodeMirrorLayout) {
  const segment = layout === "segment";
  return EditorView.theme({
    "&": {
      height: segment ? "auto" : "100%",
      fontSize: "var(--chat-font-size, 14px)",
      fontFamily: "var(--chat-font-family, inherit)",
      lineHeight: "var(--chat-line-height, 1.6)",
      color: "var(--chat-font-color, var(--ink))",
      backgroundColor: "transparent",
    },
    ".cm-scroller": {
      overflow: segment ? "visible" : "auto",
      fontFamily: "inherit",
      padding: segment ? "0" : "16px 0",
    },
    ".cm-content": {
      caretColor: "var(--chat-font-color, var(--ink))",
    },
    ".cm-lineNumbers .cm-gutterElement": {
      padding: "0 6px 0 2px",
      minWidth: "2.5em",
      fontFamily:
        'ui-sans-serif, system-ui, -apple-system, "Segoe UI", "PingFang SC", "Microsoft YaHei", sans-serif',
      fontSize: "var(--chat-font-size, 14px)",
      fontWeight: "400",
      fontVariantNumeric: "tabular-nums",
      display: "flex",
      alignItems: "flex-start",
      justifyContent: "flex-end",
      boxSizing: "border-box",
      color: "var(--ink-mute)",
      opacity: "0.55",
    },
    ".cm-gutters": {
      backgroundColor: "transparent",
      border: "none",
      color: "var(--ink-mute)",
    },
    ".reader-cm-diff-sign-gutter": {
      width: "1.25em",
      minWidth: "1.25em",
    },
    ".reader-cm-diff-sign-gutter .cm-gutterElement": {
      display: "flex",
      alignItems: "flex-start",
      justifyContent: "center",
      padding: "0 4px",
      fontSize: "var(--chat-font-size, 14px)",
      opacity: "0.55",
      userSelect: "none",
    },
    ".cm-activeLineGutter": {
      backgroundColor: "transparent",
    },
    ".cm-activeLine": {
      backgroundColor: "color-mix(in srgb, var(--ink) 4%, transparent)",
    },
    ".cm-selectionBackground, ::selection": {
      backgroundColor: "color-mix(in srgb, var(--blue-600) 28%, transparent) !important",
    },
    "&.cm-focused .cm-selectionBackground": {
      backgroundColor: "color-mix(in srgb, var(--blue-600) 28%, transparent) !important",
    },
    ".cm-find-match": {
      backgroundColor: "color-mix(in srgb, #f59e0b 38%, transparent)",
      borderRadius: "2px",
    },
    ".cm-find-match-active": {
      backgroundColor: "color-mix(in srgb, #f59e0b 72%, #fbbf24)",
      boxShadow: "0 0 0 1px color-mix(in srgb, #f59e0b 80%, var(--ink))",
    },
  });
}

/** Full-viewport reader editor (plain mode). */
export const readerCodeMirrorTheme = createReaderCodeMirrorTheme("document");
