import { useTranslation } from "react-i18next";
import { CheckIcon, CopyIcon, FolderOpenIcon } from "../icons";

interface PathRowProps {
  label: string;
  path: string | undefined;
  copied?: boolean;
  onCopy: () => void;
  onOpen?: () => void;
}

export function PathRow({
  label,
  path,
  copied,
  onCopy,
  onOpen,
}: PathRowProps) {
  const { t } = useTranslation();

  return (
    <div className="settings-info-row path">
      <span className="settings-info-label">{label}</span>
      <code className="settings-info-path" title={path || ""}>
        {path || "—"}
      </code>
      <div className="settings-info-actions">
        <button
          type="button"
          className="settings-icon-btn"
          onClick={onCopy}
          disabled={!path}
          title={copied ? t("settings.system.copied") : t("settings.system.copyPath")}
        >
          {copied ? <CheckIcon /> : <CopyIcon />}
        </button>
        {onOpen && (
          <button
            type="button"
            className="settings-icon-btn"
            onClick={onOpen}
            disabled={!path}
            title={t("settings.system.openInExplorer")}
          >
            <FolderOpenIcon />
          </button>
        )}
      </div>
    </div>
  );
}
