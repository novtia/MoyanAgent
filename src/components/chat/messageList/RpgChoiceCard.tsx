import { useMemo } from "react";
import { useSession } from "../../../store/session";
import type { AssistantBlock } from "../../../types";
import { parseRpgChoiceInput } from "./utils";
import type { RpgOption } from "./types";

export function RpgChoiceCard({
  block,
}: {
  block: Extract<AssistantBlock, { type: "tool_use" }>;
}) {
  const setPrompt = useSession((s) => s.setPrompt);
  const { prompt, options } = useMemo(
    () => parseRpgChoiceInput(block.input),
    [block.input],
  );

  if (options.length === 0) return null;

  const pick = (opt: RpgOption) => {
    setPrompt((opt.text && opt.text.trim()) || opt.label);
    window.dispatchEvent(new CustomEvent("atelier:focus-composer"));
  };

  return (
    <div className="rpg-choice">
      {prompt.trim() && <div className="rpg-choice-prompt">{prompt}</div>}
      <div className="rpg-choice-options">
        {options.map((opt, i) => (
          <button
            type="button"
            key={opt.id || `${opt.label}:${i}`}
            className="rpg-choice-option"
            onClick={() => pick(opt)}
            title={opt.text || opt.label}
          >
            <span className="rpg-choice-index">{i + 1}</span>
            <span className="rpg-choice-label">{opt.label}</span>
          </button>
        ))}
      </div>
    </div>
  );
}
