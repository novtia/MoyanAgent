import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { api } from "../../../api/tauri";
import type { NovelAiSettings, NovelAiStatus } from "../../../types";
import { ProviderEnableSwitch } from "../llm/modelService/ProviderEnableSwitch";
import { SettingsSelectDropdown } from "../SettingsSelectDropdown";
import {
  NAI_MODELS,
  NAI_SAMPLERS,
  NAI_SCHEDULES,
  NAI_SIZE_PRESETS,
  mergeNovelAi,
  parseSizePreset,
  sizePresetValue,
} from "./novelAi";

const SAVE_DEBOUNCE_MS = 500;

export function NovelAiConfigFields({
  value,
  onChange,
}: {
  value: NovelAiSettings | undefined;
  onChange: (next: NovelAiSettings) => void;
}) {
  const { t } = useTranslation();
  const current = mergeNovelAi(value, {});
  const [apiKey, setApiKey] = useState(current.api_key);
  const [showKey, setShowKey] = useState(false);
  const [forceCustomSize, setForceCustomSize] = useState(false);
  const [status, setStatus] = useState<NovelAiStatus | null>(null);
  const [statusBusy, setStatusBusy] = useState(false);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const latest = useRef(current);
  latest.current = { ...current, api_key: apiKey };

  useEffect(() => {
    setApiKey(current.api_key);
  }, [current.api_key]);

  useEffect(() => {
    return () => {
      if (timer.current) clearTimeout(timer.current);
    };
  }, []);

  const commit = (patch: Partial<NovelAiSettings>) => {
    onChange(mergeNovelAi(latest.current, patch));
  };

  const scheduleKeySave = (next: string) => {
    if (timer.current) clearTimeout(timer.current);
    timer.current = setTimeout(() => {
      onChange(mergeNovelAi(latest.current, { api_key: next.trim() }));
    }, SAVE_DEBOUNCE_MS);
  };

  const sizeValue = forceCustomSize
    ? "custom"
    : sizePresetValue(current.width, current.height);
  const sizeOptions = [
    ...NAI_SIZE_PRESETS.map((p) => ({
      value: `${p.width}x${p.height}`,
      label: t(`settings.tools.novelai.size.${p.key}`, {
        w: p.width,
        h: p.height,
      }),
    })),
    {
      value: "custom",
      label: t("settings.tools.novelai.size.custom"),
    },
  ];

  const testConnection = () => {
    setStatusBusy(true);
    setStatus(null);
    api
      .novelaiStatus(apiKey.trim() || current.api_key)
      .then(setStatus)
      .catch((e) =>
        setStatus({
          configured: Boolean((apiKey || current.api_key).trim()),
          hint: "",
          subscription: null,
          error: String(e),
        }),
      )
      .finally(() => setStatusBusy(false));
  };

  const sub = status?.subscription;
  const statusText = statusBusy
    ? t("settings.tools.novelai.statusChecking")
    : status?.error
      ? status.error
      : sub
        ? t("settings.tools.novelai.statusOk", {
            tier: sub.tierName,
            anlas: sub.anlas,
          })
        : status && !status.configured
          ? t("settings.tools.novelai.statusEmpty")
          : null;

  return (
    <div className="model-provider-config">
      <span className="field-label">{t("settings.tools.novelai.title")}</span>
      <p className="hint">{t("settings.tools.novelai.desc")}</p>

      <div className="model-provider-fields settings-nai-fields">
        <div className="row">
          <label className="field-label">
            {t("settings.tools.novelai.tokenLabel")}
          </label>
          <div className="input-affix">
            <input
              type={showKey ? "text" : "password"}
              value={apiKey}
              spellCheck={false}
              autoComplete="off"
              placeholder={t("settings.tools.novelai.tokenPlaceholder")}
              onChange={(e) => {
                const next = e.target.value;
                setApiKey(next);
                scheduleKeySave(next);
              }}
            />
            <button
              type="button"
              className="affix-btn"
              onClick={() => setShowKey((v) => !v)}
            >
              {showKey
                ? t("settings.llm.keyHide")
                : t("settings.llm.keyShow")}
            </button>
          </div>
          <p className="hint">{t("settings.tools.novelai.tokenHint")}</p>
        </div>

        <div className="settings-nai-test-row">
          <button
            type="button"
            className="appearance-reset-btn"
            onClick={testConnection}
            disabled={statusBusy}
          >
            {t("settings.tools.novelai.test")}
          </button>
          {statusText && (
            <span
              className={`hint settings-nai-status${status?.error ? " is-error" : ""}`}
            >
              {statusText}
            </span>
          )}
        </div>

        <div className="row">
          <label className="field-label">
            {t("settings.tools.novelai.modelLabel")}
          </label>
          <SettingsSelectDropdown
            value={current.model}
            ariaLabel={t("settings.tools.novelai.modelLabel")}
            options={NAI_MODELS.map((m) => ({
              value: m.value,
              label: t(`settings.tools.novelai.model.${m.labelKey}`),
            }))}
            onChange={(model) => commit({ model })}
          />
        </div>

        <div className="row">
          <label className="field-label">
            {t("settings.tools.novelai.sizeLabel")}
          </label>
          <SettingsSelectDropdown
            value={sizeValue}
            ariaLabel={t("settings.tools.novelai.sizeLabel")}
            options={sizeOptions}
            onChange={(v) => {
              if (v === "custom") {
                setForceCustomSize(true);
                return;
              }
              setForceCustomSize(false);
              const parsed = parseSizePreset(v);
              if (parsed) commit(parsed);
            }}
          />
        </div>

        {sizeValue === "custom" && (
          <div className="settings-nai-grid">
            <div className="row">
              <label className="field-label">
                {t("settings.tools.novelai.widthLabel")}
              </label>
              <input
                type="number"
                min={64}
                max={2048}
                step={64}
                value={current.width}
                onChange={(e) => {
                  const n = Number.parseInt(e.target.value, 10);
                  if (Number.isFinite(n)) commit({ width: n });
                }}
              />
            </div>
            <div className="row">
              <label className="field-label">
                {t("settings.tools.novelai.heightLabel")}
              </label>
              <input
                type="number"
                min={64}
                max={2048}
                step={64}
                value={current.height}
                onChange={(e) => {
                  const n = Number.parseInt(e.target.value, 10);
                  if (Number.isFinite(n)) commit({ height: n });
                }}
              />
            </div>
          </div>
        )}

        <div className="settings-nai-grid">
          <div className="row">
            <label className="field-label">
              {t("settings.tools.novelai.samplerLabel")}
            </label>
            <SettingsSelectDropdown
              value={current.sampler}
              ariaLabel={t("settings.tools.novelai.samplerLabel")}
              options={NAI_SAMPLERS.map((s) => ({ value: s, label: s }))}
              onChange={(sampler) => commit({ sampler })}
            />
          </div>
          <div className="row">
            <label className="field-label">
              {t("settings.tools.novelai.scheduleLabel")}
            </label>
            <SettingsSelectDropdown
              value={
                current.noise_schedule === "native"
                  ? "karras"
                  : current.noise_schedule
              }
              ariaLabel={t("settings.tools.novelai.scheduleLabel")}
              options={NAI_SCHEDULES.map((s) => ({ value: s, label: s }))}
              onChange={(noise_schedule) => commit({ noise_schedule })}
            />
          </div>
        </div>

        <div className="settings-nai-grid">
          <div className="row">
            <label className="field-label">
              {t("settings.tools.novelai.stepsLabel")}
            </label>
            <input
              type="number"
              min={1}
              max={50}
              value={current.steps}
              onChange={(e) => {
                const n = Number.parseInt(e.target.value, 10);
                if (Number.isFinite(n)) commit({ steps: n });
              }}
            />
          </div>
          <div className="row">
            <label className="field-label">
              {t("settings.tools.novelai.scaleLabel")}
            </label>
            <input
              type="number"
              min={0}
              max={30}
              step={0.5}
              value={current.scale}
              onChange={(e) => {
                const n = Number.parseFloat(e.target.value);
                if (Number.isFinite(n)) commit({ scale: n });
              }}
            />
          </div>
        </div>

        <div className="row">
          <label className="field-label">
            {t("settings.tools.novelai.cfgRescaleLabel")}
          </label>
          <input
            type="number"
            min={0}
            max={1}
            step={0.05}
            value={current.cfg_rescale}
            onChange={(e) => {
              const n = Number.parseFloat(e.target.value);
              if (Number.isFinite(n)) commit({ cfg_rescale: n });
            }}
          />
        </div>

        <div className="settings-nai-grid">
          <div className="row">
            <label className="field-label">
              {t("settings.tools.novelai.qualityLabel")}
            </label>
            <SettingsSelectDropdown
              value={current.quality}
              ariaLabel={t("settings.tools.novelai.qualityLabel")}
              options={["off", "official", "gallery"].map((q) => ({
                value: q,
                label: t(`settings.tools.novelai.quality.${q}`),
              }))}
              onChange={(quality) => commit({ quality })}
            />
          </div>
          <div className="row">
            <label className="field-label">
              {t("settings.tools.novelai.v5ModeLabel")}
            </label>
            <SettingsSelectDropdown
              value={current.v5_mode}
              ariaLabel={t("settings.tools.novelai.v5ModeLabel")}
              options={["anime", "furry"].map((m) => ({
                value: m,
                label: t(`settings.tools.novelai.v5Mode.${m}`),
              }))}
              onChange={(v5_mode) => commit({ v5_mode })}
            />
          </div>
        </div>

        <div className="row">
          <label className="field-label">
            {t("settings.tools.novelai.ucPresetLabel")}
          </label>
          <SettingsSelectDropdown
            value={current.uc_preset}
            ariaLabel={t("settings.tools.novelai.ucPresetLabel")}
            options={["heavy", "comic", "none", "custom"].map((p) => ({
              value: p,
              label: t(`settings.tools.novelai.ucPreset.${p}`),
            }))}
            onChange={(uc_preset) => commit({ uc_preset })}
          />
        </div>

        {current.uc_preset === "custom" && (
          <div className="row">
            <label className="field-label">
              {t("settings.tools.novelai.ucLabel")}
            </label>
            <textarea
              className="field-input field-input--lg"
              rows={4}
              spellCheck={false}
              value={current.uc}
              placeholder={t("settings.tools.novelai.ucPlaceholder")}
              onChange={(e) => commit({ uc: e.target.value })}
            />
          </div>
        )}

        <div className="row">
          <label className="field-label">
            {t("settings.tools.novelai.artistsLabel")}
          </label>
          <textarea
            className="field-input field-input--lg settings-nai-artists"
            rows={4}
            spellCheck={false}
            value={current.artists ?? ""}
            placeholder={t("settings.tools.novelai.artistsPlaceholder")}
            onChange={(e) => commit({ artists: e.target.value })}
          />
          <p className="hint">{t("settings.tools.novelai.artistsHint")}</p>
        </div>

        <div className="model-provider-switch-row">
          <div className="model-provider-switch-head">
            <span className="field-label">
              {t("settings.tools.novelai.alphaTitle")}
            </span>
            <ProviderEnableSwitch
              enabled={current.straight_alpha}
              onChange={(straight_alpha) => commit({ straight_alpha })}
            />
          </div>
          <p className="hint">{t("settings.tools.novelai.alphaDesc")}</p>
        </div>
      </div>
    </div>
  );
}
