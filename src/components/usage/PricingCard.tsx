import { useTranslation } from "react-i18next";
import { useUsagePricing } from "../../store/usagePricing";

interface PricingCardProps {
  models: string[];
}

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
        const price = prices[model] ?? { inputPer1M: 0, outputPer1M: 0 };
        return (
          <div className="usage-set-row" key={model}>
            <div className="usage-set-main">
              <div className="usage-set-title">{model}</div>
              {idx === list.length - 1 ? (
                <div className="usage-set-desc">{t("usage.pricingUnset")}</div>
              ) : null}
            </div>
            <div className="usage-set-inputs">
              <span className="usage-price-field">
                <label>{t("usage.pricingInput")}</label>
                <span className="box">
                  <span className="cur">{currency}</span>
                  <input
                    type="number"
                    min={0}
                    step="0.01"
                    value={price.inputPer1M}
                    disabled={!enabled}
                    onChange={(e) =>
                      setPrice(model, { inputPer1M: Number(e.target.value) || 0 })
                    }
                  />
                  <span className="per">/M</span>
                </span>
              </span>
              <span className="usage-price-field">
                <label>{t("usage.pricingOutput")}</label>
                <span className="box">
                  <span className="cur">{currency}</span>
                  <input
                    type="number"
                    min={0}
                    step="0.01"
                    value={price.outputPer1M}
                    disabled={!enabled}
                    onChange={(e) =>
                      setPrice(model, {
                        outputPer1M: Number(e.target.value) || 0,
                      })
                    }
                  />
                  <span className="per">/M</span>
                </span>
              </span>
            </div>
          </div>
        );
      })}
    </div>
  );
}
