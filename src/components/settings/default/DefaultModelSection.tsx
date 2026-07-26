import { useMemo } from "react";
import { useTranslation } from "react-i18next";
import { useSettings } from "../../../store/settings";
import { normalizeProviders } from "../llm/modelServices";
import {
  QuickModelDropdown,
  type QuickModelOption,
} from "./QuickModelDropdown";

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

  const groups = useMemo(() => {
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
    return Array.from(map.entries()).map(([providerId, group]) => ({
      providerId,
      ...group,
    }));
  }, [options]);

  const selectedIndex = useMemo(() => {
    const providerId = settings?.quick_model_provider_id ?? "";
    const modelId = settings?.quick_model ?? "";
    if (!providerId || !modelId) return -1;
    return options.findIndex(
      (option) => option.providerId === providerId && option.modelId === modelId,
    );
  }, [options, settings?.quick_model_provider_id, settings?.quick_model]);

  const onChange = (index: number | null) => {
    if (index === null || index < 0) {
      void update({ quick_model_provider_id: "", quick_model: "" });
      return;
    }
    const option = options[index];
    if (!option) return;
    void update({
      quick_model_provider_id: option.providerId,
      quick_model: option.modelId,
    });
  };

  return (
    <div className="settings-card">
      <div className="settings-row">
        <div className="settings-row-main">
          <div className="settings-row-title">
            {t("settings.default.quickModelTitle")}
          </div>
          <div className="settings-row-desc">
            {t("settings.default.quickModelDesc")}
          </div>
          {options.length === 0 && (
            <div className="settings-row-desc">
              {t("settings.default.quickModelEmpty")}
            </div>
          )}
        </div>
        {options.length > 0 && (
          <div className="settings-row-control">
            <QuickModelDropdown
              groups={groups}
              options={options}
              selectedIndex={selectedIndex}
              noneLabel={t("settings.default.quickModelNone")}
              ariaLabel={t("settings.default.quickModelTitle")}
              onChange={onChange}
            />
          </div>
        )}
      </div>
    </div>
  );
}
