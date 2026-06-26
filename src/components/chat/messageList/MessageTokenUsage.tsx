import { useTranslation } from "react-i18next";
import { resolveMessageTokenUsage, tokenUsageFormatter } from "./utils";
import type { MessageTokenUsageData } from "./types";

export function MessageTokenUsage({ usage }: { usage?: MessageTokenUsageData | null }) {
  const { t } = useTranslation();
  if (!usage) return null;

  const turn = resolveMessageTokenUsage(usage);
  if (!turn) return null;

  const { prompt, completion } = turn;
  const label =
    prompt > 0 && completion > 0
      ? t("message.tokenUsageTurn", {
          prompt: tokenUsageFormatter.format(prompt),
          completion: tokenUsageFormatter.format(completion),
        })
      : completion > 0
        ? t("message.tokenUsageOutput", {
            completion: tokenUsageFormatter.format(completion),
          })
        : t("message.tokenUsageInput", {
            prompt: tokenUsageFormatter.format(prompt),
          });

  return <span className="msg-token-usage">{label}</span>;
}
