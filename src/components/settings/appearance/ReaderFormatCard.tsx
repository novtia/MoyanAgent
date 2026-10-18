import { useTranslation } from "react-i18next";
import { useSettings } from "../../../store/settings";

function FormatToggle({
  on,
  label,
  onToggle,
}: {
  on: boolean;
  label: string;
  onToggle: () => void;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={on}
      aria-label={label}
      className={`settings-toggle ${on ? "settings-toggle--on" : ""}`}
      onClick={onToggle}
    >
      <span className="settings-toggle-thumb" />
    </button>
  );
}

/** Reader toolbar options: first-line indent and blank lines between paragraphs. */
export function ReaderFormatCard() {
  const { t } = useTranslation();
  const settings = useSettings((s) => s.settings);
  const update = useSettings((s) => s.update);
  const firstLine = settings?.reader_first_line_indent ?? true;
  const spacing = settings?.reader_paragraph_spacing ?? true;

  return (
    <div className="settings-card">
      <div className="settings-block">
        <div className="settings-block-head">
          <div className="settings-row-title">{t("settings.appearance.readerFormatTitle")}</div>
          <div className="settings-row-desc">{t("settings.appearance.readerFormatDesc")}</div>
        </div>
      </div>
      <div className="settings-row">
        <div className="settings-row-main">
          <div className="settings-row-title">{t("settings.appearance.firstLineIndentTitle")}</div>
          <div className="settings-row-desc">{t("settings.appearance.firstLineIndentDesc")}</div>
        </div>
        <div className="settings-row-control">
          <FormatToggle
            on={firstLine}
            label={t("settings.appearance.firstLineIndentTitle")}
            onToggle={() => void update({ reader_first_line_indent: !firstLine })}
          />
        </div>
      </div>
      <div className="settings-row">
        <div className="settings-row-main">
          <div className="settings-row-title">
            {t("settings.appearance.paragraphSpacingTitle")}
          </div>
          <div className="settings-row-desc">{t("settings.appearance.paragraphSpacingDesc")}</div>
        </div>
        <div className="settings-row-control">
          <FormatToggle
            on={spacing}
            label={t("settings.appearance.paragraphSpacingTitle")}
            onToggle={() => void update({ reader_paragraph_spacing: !spacing })}
          />
        </div>
      </div>
    </div>
  );
}
