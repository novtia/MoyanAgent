export interface NumericFieldDef {
  key: "temperature" | "top_p" | "max_tokens" | "frequency_penalty" | "presence_penalty";
  labelKey: string;
  hintKey: string;
  step: string;
  min?: number;
  max?: number;
  integer?: boolean;
  placeholder: string;
}

export const NUMERIC_FIELDS: NumericFieldDef[] = [
  {
    key: "temperature",
    labelKey: "settings.llm.temperature",
    hintKey: "settings.llm.temperatureHint",
    step: "0.1",
    min: 0,
    max: 2,
    placeholder: "0.7",
  },
  {
    key: "top_p",
    labelKey: "settings.llm.topP",
    hintKey: "settings.llm.topPHint",
    step: "0.05",
    min: 0,
    max: 1,
    placeholder: "0.9",
  },
  {
    key: "max_tokens",
    labelKey: "settings.llm.maxTokens",
    hintKey: "settings.llm.maxTokensHint",
    step: "1",
    min: 1,
    integer: true,
    placeholder: "2048",
  },
  {
    key: "frequency_penalty",
    labelKey: "settings.llm.frequencyPenalty",
    hintKey: "settings.llm.frequencyPenaltyHint",
    step: "0.1",
    min: -2,
    max: 2,
    placeholder: "0",
  },
  {
    key: "presence_penalty",
    labelKey: "settings.llm.presencePenalty",
    hintKey: "settings.llm.presencePenaltyHint",
    step: "0.1",
    min: -2,
    max: 2,
    placeholder: "0",
  },
];
