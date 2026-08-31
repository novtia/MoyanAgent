import { useMemo } from "react";
import { useTranslation } from "react-i18next";
import { estimateTextTokens } from "../../utils/estimateTokens";

const nf = new Intl.NumberFormat();

/** Live token estimate for a system-prompt textarea. */
export function PromptTokenMeta({ text }: { text: string }) {
  const { t } = useTranslation();
  const tokens = useMemo(() => estimateTextTokens(text), [text]);
  return (
    <span className="prompt-token-meta" title={t("common.promptTokensHint")}>
      {t("common.promptTokens", { count: nf.format(tokens) })}
    </span>
  );
}
