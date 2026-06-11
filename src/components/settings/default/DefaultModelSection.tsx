import { useMemo } from "react";
import { useTranslation } from "react-i18next";
import { useSettings } from "../../../store/settings";
import { normalizeProviders } from "../llm/modelServices";

interface QuickModelOption {
  providerId: string;
  providerName: string;
  modelId: string;
  modelName: string;
}

export function DefaultModelSection() {
  const { t } = useTranslation();
  const settings = useSettings((s) => s.settings);
  const update = useSettings((s) => s.update);

  const providers = useMemo(
    () => normalizeProviders(settings?.model_services ?? []),
    [settings?.model_services],
  );

  const options = useMemo<QuickModelOption[]>(
    () =>
      providers
        .filter((provider) => provider.enabled !== false)
        .flatMap((provider) =>
          provider.models.map((model) => ({
            providerId: provider.id,
            providerName: provider.name,
            modelId: model.id,
            modelName: model.name || model.id,
          })),
        ),
    [providers],
  );

  const grouped = useMemo(() => {
    const map = new Map<
      string,
      { name: string; items: { option: QuickModelOption; index: number }[] }
    >();
    options.forEach((option, index) => {
      const entry = map.get(option.providerId) ?? {
        name: option.providerName,
        items: [],
      };
      entry.items.push({ option, index });
      map.set(option.providerId, entry);
    });
    return Array.from(map.entries());
  }, [options]);

  const selectedIndex = useMemo(() => {
    const providerId = settings?.quick_model_provider_id ?? "";
    const modelId = settings?.quick_model ?? "";
    if (!providerId || !modelId) return -1;
    return options.findIndex(
      (option) => option.providerId === providerId && option.modelId === modelId,
    );
  }, [options, settings?.quick_model_provider_id, settings?.quick_model]);

  const onChange = (value: string) => {
    if (value === "") {
      void update({ quick_model_provider_id: "", quick_model: "" });
      return;
    }
    const index = Number(value);
    const option = options[index];
    if (!option) return;
    void update({
      quick_model_provider_id: option.providerId,
      quick_model: option.modelId,
    });
  };

  return (
    <div className="settings-card">
      <div className="settings-card-head">
        <div>
          <div className="settings-card-title">
            {t("settings.default.quickModelTitle")}
          </div>
          <div className="settings-card-desc">
            {t("settings.default.quickModelDesc")}
          </div>
        </div>
        {options.length > 0 ? (
          <select
            className="settings-select"
            aria-label={t("settings.default.quickModelTitle")}
            value={selectedIndex >= 0 ? String(selectedIndex) : ""}
            onChange={(event) => onChange(event.target.value)}
          >
            <option value="">{t("settings.default.quickModelNone")}</option>
            {grouped.map(([providerId, group]) => (
              <optgroup key={providerId} label={group.name}>
                {group.items.map(({ option, index }) => (
                  <option
                    key={`${option.providerId}:${option.modelId}`}
                    value={String(index)}
                  >
                    {option.modelName}
                  </option>
                ))}
              </optgroup>
            ))}
          </select>
        ) : (
          <div className="settings-card-desc">
            {t("settings.default.quickModelEmpty")}
          </div>
        )}
      </div>
    </div>
  );
}
