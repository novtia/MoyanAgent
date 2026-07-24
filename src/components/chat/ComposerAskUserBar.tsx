import { useSession } from "../../store/session";
import {
  optionKey,
  optionReplyText,
  questionKey,
  type AskUserOption,
} from "./askUser";

/**
 * Question strip inside the composer card.
 * ○/● select an option without filling the input; the ComposerEditor below is
 * custom-only — empty input means the selected option is the answer.
 */
export function ComposerAskUserBar() {
  const pending = useSession((s) => s.pendingAskUser);
  const prompt = useSession((s) => s.composer.prompt);
  const setAskUserIndex = useSession((s) => s.setAskUserIndex);
  const setAskUserAnswer = useSession((s) => s.setAskUserAnswer);
  const clearAskUserAnswer = useSession((s) => s.clearAskUserAnswer);

  if (!pending || pending.questions.length === 0) return null;

  const total = pending.questions.length;
  const index = pending.activeIndex;
  const question = pending.questions[index];
  if (!question) return null;

  const qKey = questionKey(question, index);
  const answer = pending.answers[qKey];
  const customActive = prompt.trim().length > 0;

  const pick = (opt: AskUserOption, optIndex: number) => {
    const key = optionKey(opt, optIndex);
    // Toggle off when clicking the already-selected option.
    if (!customActive && answer?.optionKey === key) {
      clearAskUserAnswer();
      return;
    }
    setAskUserAnswer(key, optionReplyText(opt));
  };

  const go = (delta: number) => {
    if (total <= 1) return;
    const next = (index + delta + total) % total;
    setAskUserIndex(next);
    window.dispatchEvent(new CustomEvent("atelier:focus-composer"));
  };

  return (
    <div className="composer-ask-user" role="group" aria-label="AskUser">
      <div className="composer-ask-user-nav">
        <button
          type="button"
          className="composer-ask-user-nav-btn"
          onClick={() => go(-1)}
          disabled={total <= 1}
          aria-label="Previous question"
          title="◀"
        >
          ◀
        </button>
        <div className="composer-ask-user-meta">
          <span className="composer-ask-user-count">
            问题 {index + 1}/{total}
          </span>
          <div className="composer-ask-user-prompt">{question.prompt}</div>
          <div className="composer-ask-user-options">
            {question.options.map((opt, i) => {
              const key = optionKey(opt, i);
              // Custom input overrides: when typing, options appear unselected.
              const selected =
                !customActive && !!answer?.optionKey && answer.optionKey === key;
              return (
                <button
                  type="button"
                  key={key}
                  className={`composer-ask-user-option${selected ? " selected" : ""}`}
                  onClick={() => pick(opt, i)}
                  title={opt.text || opt.label}
                >
                  <span className="composer-ask-user-mark" aria-hidden>
                    {selected ? "●" : "○"}
                  </span>
                  <span className="composer-ask-user-label">{opt.label}</span>
                </button>
              );
            })}
          </div>
        </div>
        <button
          type="button"
          className="composer-ask-user-nav-btn"
          onClick={() => go(1)}
          disabled={total <= 1}
          aria-label="Next question"
          title="▶"
        >
          ▶
        </button>
      </div>
    </div>
  );
}
