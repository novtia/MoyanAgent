import { useTranslation } from "react-i18next";
import {
  cacheHitRate,
  formatCacheHitPct,
} from "../../usage/format";
import { resolveMessageTokenUsage, tokenUsageFormatter } from "./utils";
import type { MessageTokenUsageData } from "./types";

export function MessageTokenUsage({ usage }: { usage?: MessageTokenUsageData | null }) {
  const { t } = useTranslation();
  if (!usage) return null;

  const turn = resolveMessageTokenUsage(usage);
  if (!turn) return null;

  const { prompt, completion, cacheRead, cacheWrite } = turn;
  const hasCache = cacheRead > 0 || cacheWrite > 0;
  const hitPct = cacheHitRate(prompt, cacheRead);
  const cache =
    !hasCache
      ? ""
      : hitPct != null
        ? t("message.tokenUsageCacheHit", {
            cache: tokenUsageFormatter.format(cacheRead),
            hit: formatCacheHitPct(hitPct),
          })
        : t("message.tokenUsageCache", {
            cache: tokenUsageFormatter.format(
              cacheRead > 0 ? cacheRead : cacheWrite,
            ),
          });

  const label =
    prompt > 0 && completion > 0
      ? t("message.tokenUsageTurn", {
          prompt: tokenUsageFormatter.format(prompt),
          completion: tokenUsageFormatter.format(completion),
          cache,
        })
      : completion > 0
        ? t("message.tokenUsageOutput", {
            completion: tokenUsageFormatter.format(completion),
            cache,
          })
        : t("message.tokenUsageInput", {
            prompt: tokenUsageFormatter.format(prompt > 0 ? prompt : 0),
            cache,
          });

  const title = hasCache
    ? t("usage.cacheSplit", {
        read: tokenUsageFormatter.format(cacheRead),
        write: tokenUsageFormatter.format(cacheWrite),
        hit: formatCacheHitPct(hitPct),
      })
    : undefined;

  return (
    <span className="msg-token-usage" title={title}>
      {label}
    </span>
  );
}
