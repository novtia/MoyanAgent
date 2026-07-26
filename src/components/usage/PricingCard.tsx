import { useTranslation } from "react-i18next";
import { EMPTY_MODEL_PRICE, useUsagePricing } from "../../store/usagePricing";
import type { ModelPrice } from "../../store/usagePricing";

interface PricingCardProps {
  models: string[];
}

type PriceField = keyof ModelPrice;

const FIELDS: PriceField[] = [
  "inputPer1M",
  "outputPer1M",
  "cacheReadPer1M",
  "cacheWritePer1M",
];

const FIELD_LABEL: Record<PriceField, string> = {
  inputPer1M: "usage.pricingInput",
  cacheReadPer1M: "usage.pricingCacheRead",
  cacheWritePer1M: "usage.pricingCacheWrite",
  outputPer1M: "usage.pricingOutput",
};

export function PricingCard({ models }: PricingCardProps) {
  const { t } = useTranslation();
  const enabled = useUsagePricing((s) => s.enabled);
  const prices = useUsagePricing((s) => s.prices);
  const currency = useUsagePricing((s) => s.currency);
  const setEnabled = useUsagePricing((s) => s.setEnabled);
  const setPrice = useUsagePricing((s) => s.setPrice);

  const list = models.length > 0 ? models : Object.keys(prices);

  return (
    <div className="usage-card">
      <div className="usage-card-head">
        <span className="t">{t("usage.pricingTitle")}</span>
        <span className="m">{t("usage.pricingMeta")}</span>
        <button
          type="button"
          className={`usage-toggle ${enabled ? "usage-toggle--on" : ""}`}
          role="switch"
          aria-checked={enabled}
          aria-label={t("usage.pricingTitle")}
          onClick={() => setEnabled(!enabled)}
        >
          <span className="usage-toggle-thumb" />
        </button>
      </div>
      {list.map((model, idx) => {
        const price = prices[model] ?? EMPTY_MODEL_PRICE;
        return (
          <div className="usage-set-row" key={model}>
            <div className="usage-set-main">
              <div className="usage-set-title">{model}</div>
              {idx === list.length - 1 ? (
                <div className="usage-set-desc">{t("usage.pricingUnset")}</div>
              ) : null}
            </div>
            <div className="usage-set-inputs">
              {FIELDS.map((field) => (
                <span className="usage-price-field" key={field}>
                  <label>{t(FIELD_LABEL[field])}</label>
                  <span className="box">
                    <span className="cur">{currency}</span>
                    <input
                      type="number"
                      min={0}
                      step="0.01"
                      value={price[field]}
                      disabled={!enabled}
                      onChange={(e) =>
                        setPrice(model, {
                          [field]: Number(e.target.value) || 0,
                        })
                      }
                    />
                    <span className="per">{t("usage.pricingPerMillion")}</span>
                  </span>
                </span>
              ))}
            </div>
          </div>
        );
      })}
    </div>
  );
}
