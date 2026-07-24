import { useMemo } from "react";
import type { AssistantBlock } from "../../../types";
import {
  displayAskUserAnswer,
  parseAskUserInput,
  parseAskUserOutput,
} from "../askUser";

/**
 * History card for AskUser: lists every question with only the user's
 * selected option / custom reply (no carousel).
 */
export function AskUserChip({
  block,
}: {
  block: Extract<AssistantBlock, { type: "tool_use" }>;
}) {
  const questions = useMemo(
    () => parseAskUserInput(block.input),
    [block.input],
  );
  const items = useMemo(
    () => parseAskUserOutput(block.output, questions),
    [block.output, questions],
  );

  const total = Math.max(questions.length, items.length);
  if (total <= 0) return null;

  const pending = block.status === "pending" || block.streaming;
  const errored = block.status === "error";

  return (
    <div
      className={`ask-user-card${block.status === "success" ? " is-answered" : ""}${pending ? " is-pending" : ""}`}
    >
      {Array.from({ length: total }, (_, i) => {
        const question = questions[i];
        const item = items[i];
        const promptText = item?.prompt || question?.prompt || "";
        const rawAnswer = item?.answer?.trim() || "";
        const answered = block.status === "success" && !!rawAnswer;
        const displayAnswer = answered
          ? question
            ? displayAskUserAnswer(question, rawAnswer)
            : rawAnswer
          : pending
            ? "等待回答…"
            : errored
              ? "已取消"
              : "";

        return (
          <div className="ask-user-card-item" key={`ask-q:${i}`}>
            {total > 1 ? (
              <div className="ask-user-card-meta">
                <span className="ask-user-card-count">问题 {i + 1}/{total}</span>
              </div>
            ) : null}
            {promptText ? (
              <div className="ask-user-card-prompt">{promptText}</div>
            ) : null}
            {displayAnswer ? (
              <div className="ask-user-card-answer">
                <span className="ask-user-card-answer-mark" aria-hidden>
                  {answered ? "●" : "○"}
                </span>
                <span className="ask-user-card-answer-text">{displayAnswer}</span>
              </div>
            ) : null}
          </div>
        );
      })}
    </div>
  );
}
