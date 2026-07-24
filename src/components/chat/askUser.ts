/** Shared AskUser question/option types and helpers (tool input ↔ composer). */

export interface AskUserOption {
  id?: string;
  label: string;
  text?: string;
}

export interface AskUserQuestion {
  id?: string;
  prompt: string;
  options: AskUserOption[];
}

/**
 * Per-question answer.
 * - Selecting an option stores `optionKey` + `optionText` (does NOT fill the input).
 * - The composer input is **custom only**; if non-empty it overrides the option.
 */
export interface AskUserAnswer {
  /** Stable option key: `option.id` or `opt:${index}`. */
  optionKey?: string;
  /** Reply text from the selected option. */
  optionText?: string;
  /** Custom text from the composer input (optional override). */
  custom?: string;
}

export interface PendingAskUser {
  sessionId: string;
  blockId: string;
  questions: AskUserQuestion[];
  activeIndex: number;
  /** Keyed by {@link questionKey}. */
  answers: Record<string, AskUserAnswer>;
}

export function questionKey(q: AskUserQuestion, index: number): string {
  return q.id?.trim() || `q:${index}`;
}

export function optionKey(opt: AskUserOption, index: number): string {
  return opt.id?.trim() || `opt:${index}`;
}

export function optionReplyText(opt: AskUserOption): string {
  const text = opt.text?.trim();
  return text || opt.label;
}

export function parseAskUserInput(input: unknown): AskUserQuestion[] {
  const obj =
    input && typeof input === "object" ? (input as Record<string, unknown>) : {};
  const raw = Array.isArray(obj.questions) ? obj.questions : [];
  const questions: AskUserQuestion[] = [];
  for (const v of raw) {
    if (!v || typeof v !== "object") continue;
    const o = v as Record<string, unknown>;
    const prompt = typeof o.prompt === "string" ? o.prompt.trim() : "";
    if (!prompt) continue;
    const rawOptions = Array.isArray(o.options) ? o.options : [];
    const options: AskUserOption[] = rawOptions.flatMap((item) => {
      if (!item || typeof item !== "object") return [];
      const opt = item as Record<string, unknown>;
      if (typeof opt.label !== "string" || !opt.label.trim()) return [];
      return [
        {
          id: typeof opt.id === "string" ? opt.id : undefined,
          label: opt.label,
          text: typeof opt.text === "string" ? opt.text : undefined,
        },
      ];
    });
    if (options.length === 0) continue;
    questions.push({
      id: typeof o.id === "string" ? o.id : undefined,
      prompt,
      options,
    });
  }
  return questions;
}

/** Persist composer input as custom-only for the active question (options untouched). */
export function flushAskUserPrompt(
  pending: PendingAskUser,
  prompt: string,
): PendingAskUser {
  const q = pending.questions[pending.activeIndex];
  if (!q) return pending;
  const key = questionKey(q, pending.activeIndex);
  const prev = pending.answers[key] ?? {};
  return {
    ...pending,
    answers: {
      ...pending.answers,
      [key]: {
        ...prev,
        custom: prompt,
      },
    },
  };
}

/** Custom text shown in the composer for a question (never option fill-in). */
export function askUserCustomText(
  pending: PendingAskUser,
  index: number,
): string {
  const q = pending.questions[index];
  if (!q) return "";
  return pending.answers[questionKey(q, index)]?.custom ?? "";
}

/** Effective answer: custom if typed, otherwise selected option text. */
export function askUserAnswerText(
  pending: PendingAskUser,
  index: number,
): string {
  const q = pending.questions[index];
  if (!q) return "";
  const a = pending.answers[questionKey(q, index)];
  if (!a) return "";
  const custom = a.custom?.trim();
  if (custom) return custom;
  return a.optionText?.trim() ?? "";
}

export function firstUnansweredAskUserIndex(
  pending: PendingAskUser,
): number {
  for (let i = 0; i < pending.questions.length; i++) {
    if (!askUserAnswerText(pending, i).trim()) return i;
  }
  return -1;
}

export function formatAskUserReply(pending: PendingAskUser): string {
  const lines: string[] = ["[AskUser 答复]"];
  pending.questions.forEach((q, i) => {
    const answer = askUserAnswerText(pending, i).trim();
    lines.push(`${i + 1}. ${q.prompt}`);
    lines.push(`答：${answer}`);
  });
  return lines.join("\n");
}

export interface AskUserReplyItem {
  prompt: string;
  answer: string;
}

/** Structured per-question replies for tool_result / history card. */
export function formatAskUserItems(pending: PendingAskUser): AskUserReplyItem[] {
  return pending.questions.map((q, i) => ({
    prompt: q.prompt,
    answer: askUserAnswerText(pending, i).trim(),
  }));
}

/** Prefer the option label when the stored answer matches an option. */
export function displayAskUserAnswer(
  question: AskUserQuestion,
  answer: string,
): string {
  const trimmed = answer.trim();
  if (!trimmed) return "";
  for (const opt of question.options) {
    if (optionReplyText(opt).trim() === trimmed || opt.label.trim() === trimmed) {
      return opt.label;
    }
  }
  return trimmed;
}

/**
 * Resolve answered items from tool output (structured `items` preferred,
 * falls back to parsing the aggregated `answer` text).
 */
export function parseAskUserOutput(
  output: unknown,
  questions: AskUserQuestion[],
): AskUserReplyItem[] {
  const obj =
    output && typeof output === "object"
      ? (output as Record<string, unknown>)
      : null;
  if (obj && Array.isArray(obj.items)) {
    const items: AskUserReplyItem[] = [];
    for (const v of obj.items) {
      if (!v || typeof v !== "object") continue;
      const o = v as Record<string, unknown>;
      const prompt = typeof o.prompt === "string" ? o.prompt : "";
      const answer = typeof o.answer === "string" ? o.answer : "";
      if (prompt || answer) items.push({ prompt, answer });
    }
    if (items.length > 0) return items;
  }

  const raw =
    typeof obj?.answer === "string"
      ? obj.answer
      : typeof output === "string"
        ? output
        : "";
  if (!raw.trim()) {
    return questions.map((q) => ({ prompt: q.prompt, answer: "" }));
  }

  // Parse:
  // [AskUser 答复]
  // 1. <prompt>
  // 答：<answer>
  const items: AskUserReplyItem[] = [];
  const blocks = raw.split(/\n(?=\d+\.\s)/);
  for (const block of blocks) {
    const lines = block.split("\n").map((l) => l.trim()).filter(Boolean);
    if (lines.length === 0) continue;
    const head = lines[0].match(/^\d+\.\s*(.*)$/);
    if (!head) continue;
    const prompt = head[1];
    let answer = "";
    for (const line of lines.slice(1)) {
      const m = line.match(/^答[：:]\s*(.*)$/);
      if (m) answer = m[1];
    }
    items.push({ prompt, answer });
  }
  if (items.length > 0) return items;
  // Single free-form fallback
  if (questions.length === 1) {
    return [{ prompt: questions[0].prompt, answer: raw.trim() }];
  }
  return questions.map((q) => ({ prompt: q.prompt, answer: "" }));
}
