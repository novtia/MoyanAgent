import { RangeSetBuilder, StateEffect, StateField } from "@codemirror/state";
import { Decoration, DecorationSet, EditorView } from "@codemirror/view";
import type { TextRange } from "../../../../../utils/readerFind";

export const setFindHighlightsEffect = StateEffect.define<{
  ranges: TextRange[];
  activeIndex: number;
}>();

const findMatchMark = Decoration.mark({ class: "cm-find-match" });
const findMatchActiveMark = Decoration.mark({ class: "cm-find-match cm-find-match-active" });

export const findHighlightField = StateField.define<DecorationSet>({
  create() {
    return Decoration.none;
  },
  update(deco, tr) {
    deco = deco.map(tr.changes);
    for (const effect of tr.effects) {
      if (!effect.is(setFindHighlightsEffect)) continue;
      const { ranges, activeIndex } = effect.value;
      if (ranges.length === 0) return Decoration.none;

      const sorted = [...ranges].sort((a, b) => a.start - b.start);
      const active = activeIndex >= 0 ? ranges[activeIndex] : null;
      const builder = new RangeSetBuilder<Decoration>();
      for (const range of sorted) {
        const isActive =
          active != null && range.start === active.start && range.end === active.end;
        builder.add(
          range.start,
          range.end,
          isActive ? findMatchActiveMark : findMatchMark,
        );
      }      deco = builder.finish();
    }
    return deco;
  },
  provide: (field) => EditorView.decorations.from(field),
});

export function applyFindHighlights(
  view: EditorView,
  ranges: TextRange[],
  activeIndex: number,
) {
  view.dispatch({
    effects: setFindHighlightsEffect.of({ ranges, activeIndex }),
  });
}
