import { RangeSetBuilder, StateEffect, StateField, type Range } from "@codemirror/state";
import { Decoration, DecorationSet, EditorView, WidgetType } from "@codemirror/view";
import type { InlineWordDiffResult } from "../../../../../utils/inlineDiff";

export const setInlineDiffEffect = StateEffect.define<InlineWordDiffResult | null>();

const inlineAddedMark = Decoration.mark({ class: "cm-diff-inline-added" });

class RemovedSpanWidget extends WidgetType {
  constructor(readonly text: string) {
    super();
  }

  eq(other: RemovedSpanWidget): boolean {
    return other.text === this.text;
  }

  toDOM(): HTMLElement {
    const span = document.createElement("span");
    span.className = "cm-diff-inline-removed";
    span.setAttribute("aria-hidden", "true");
    span.textContent = this.text;
    return span;
  }

  ignoreEvent(): boolean {
    return true;
  }
}

function buildDecorations(
  diff: InlineWordDiffResult | null,
  docLength: number,
): DecorationSet {
  if (!diff || (diff.added.length === 0 && diff.removed.length === 0)) {
    return Decoration.none;
  }

  const items: Array<Range<Decoration>> = [];

  for (const r of diff.added) {
    const from = Math.max(0, Math.min(r.from, docLength));
    const to = Math.max(0, Math.min(r.to, docLength));
    if (to > from) {
      items.push(inlineAddedMark.range(from, to));
    }
  }

  for (const rem of diff.removed) {
    if (!rem.text) continue;
    const pos = Math.max(0, Math.min(rem.pos, docLength));
    items.push(
      Decoration.widget({
        widget: new RemovedSpanWidget(rem.text),
        side: -1,
      }).range(pos),
    );
  }

  items.sort((a, b) => a.from - b.from || a.to - b.to);
  const builder = new RangeSetBuilder<Decoration>();
  for (const item of items) {
    builder.add(item.from, item.to, item.value);
  }
  return builder.finish();
}

export const inlineDiffField = StateField.define<DecorationSet>({
  create() {
    return Decoration.none;
  },
  update(deco, tr) {
    deco = deco.map(tr.changes);
    for (const effect of tr.effects) {
      if (!effect.is(setInlineDiffEffect)) continue;
      return buildDecorations(effect.value, tr.state.doc.length);
    }
    return deco;
  },
  provide: (field) => EditorView.decorations.from(field),
});

export function applyInlineDiffDecorations(
  view: EditorView,
  diff: InlineWordDiffResult | null,
) {
  view.dispatch({
    effects: setInlineDiffEffect.of(diff),
  });
}
