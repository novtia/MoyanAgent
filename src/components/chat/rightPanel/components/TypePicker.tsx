import { useTranslation } from "react-i18next";
import type { PickerKind } from "../types";
import { FlowIcon, GalleryIcon, ReaderIcon, RoleStateIcon } from "./icons";

export interface TypePickerProps {
  tab: number;
  showReader: boolean;
  onPick: (kind: PickerKind) => void;
}

export function TypePicker({ tab, showReader, onPick }: TypePickerProps) {
  const { t } = useTranslation();
  return (
    <div className="right-panel-picker">
      <p className="right-panel-picker-title">{t("rightPanel.emptyTitle")}</p>
      <button
        type="button"
        className="right-panel-picker-card"
        tabIndex={tab}
        onClick={() => onPick("gallery")}
      >
        <span className="right-panel-picker-icon">
          <GalleryIcon />
        </span>
        <span className="right-panel-picker-text">
          <span className="right-panel-picker-name">{t("rightPanel.createGallery")}</span>
          <span className="right-panel-picker-desc">{t("rightPanel.createGalleryDesc")}</span>
        </span>
      </button>
      <button
        type="button"
        className="right-panel-picker-card"
        tabIndex={tab}
        onClick={() => onPick("agent-flow")}
      >
        <span className="right-panel-picker-icon">
          <FlowIcon />
        </span>
        <span className="right-panel-picker-text">
          <span className="right-panel-picker-name">{t("rightPanel.createAgentFlow")}</span>
          <span className="right-panel-picker-desc">{t("rightPanel.createAgentFlowDesc")}</span>
        </span>
      </button>
      <button
        type="button"
        className="right-panel-picker-card"
        tabIndex={tab}
        onClick={() => onPick("role-state")}
      >
        <span className="right-panel-picker-icon">
          <RoleStateIcon />
        </span>
        <span className="right-panel-picker-text">
          <span className="right-panel-picker-name">{t("rightPanel.createRoleState")}</span>
          <span className="right-panel-picker-desc">{t("rightPanel.createRoleStateDesc")}</span>
        </span>
      </button>
      {showReader && (
        <button
          type="button"
          className="right-panel-picker-card"
          tabIndex={tab}
          onClick={() => onPick("reader")}
        >
          <span className="right-panel-picker-icon">
            <ReaderIcon />
          </span>
          <span className="right-panel-picker-text">
            <span className="right-panel-picker-name">{t("rightPanel.createReader")}</span>
            <span className="right-panel-picker-desc">{t("rightPanel.createReaderDesc")}</span>
          </span>
        </button>
      )}
    </div>
  );
}
