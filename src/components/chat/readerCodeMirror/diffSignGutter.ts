import { RangeSetBuilder } from "@codemirror/state";
import { gutter, GutterMarker, type EditorView } from "@codemirror/view";

class DiffSignMarker extends GutterMarker {
  constructor(readonly text: string) {
    super();
  }

  eq(other: GutterMarker): boolean {
    return other instanceof DiffSignMarker && other.text === this.text;
  }

  toDOM() {
    const span = document.createElement("span");
    span.className = "reader-cm-diff-sign";
    span.textContent = this.text;
    span.setAttribute("aria-hidden", "true");
    return span;
  }
}

export function diffSignGutterExtension(sign?: string, firstLineOnly = false) {
  if (!sign) return [];
  const filled = new DiffSignMarker(sign);
  const empty = new DiffSignMarker("");

  return gutter({
    class: "reader-cm-diff-sign-gutter",
    markers(view: EditorView) {
      const builder = new RangeSetBuilder<GutterMarker>();
      for (let i = 1; i <= view.state.doc.lines; i += 1) {
        const line = view.state.doc.line(i);
        const marker = firstLineOnly && i > 1 ? empty : filled;
        builder.add(line.from, line.from, marker);
      }
      return builder.finish();
    },
    initialSpacer: () => filled,
  });
}
