import { useTranslation } from "react-i18next";
import { ToolGlyph } from "../../chat/messageList/toolIcons";
import {
  isCustomToolDescription,
  modelToolDescription,
  toolDescription,
} from "../../chat/rightPanel/agentFlow/toolUtils";
import { ProviderEnableSwitch } from "../llm/modelService/ProviderEnableSwitch";
import type { AgentToolSpec } from "../../../types";

function listToolDescription(
  t: (key: string, opts?: { defaultValue?: string }) => string,
  tool: AgentToolSpec,
  descriptions: Record<string, string>,
): string {
  if (isCustomToolDescription(tool, descriptions)) {
    return modelToolDescription(tool, descriptions).replace(/\s+/g, " ").trim();
  }
  const localized = toolDescription(t, tool.name);
  return localized === tool.name ? tool.description : localized;
}

function SearchIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.7" aria-hidden>
      <circle cx="11" cy="11" r="7" />
      <line x1="21" y1="21" x2="16.65" y2="16.65" />
    </svg>
  );
}

export function ToolsPane({
  tools,
  filteredTools,
  selectedName,
  disabledTools,
  forcedTools,
  descriptions,
  search,
  onSearchChange,
  onSelect,
  onSetEnabled,
}: {
  tools: AgentToolSpec[];
  filteredTools: AgentToolSpec[];
  selectedName?: string;
  disabledTools: string[];
  forcedTools: string[];
  descriptions: Record<string, string>;
  search: string;
  onSearchChange: (value: string) => void;
  onSelect: (tool: AgentToolSpec) => void;
  onSetEnabled: (name: string, enabled: boolean) => void;
}) {
  const { t } = useTranslation();
  const disabled = new Set(disabledTools);
  const forced = new Set(forcedTools);

  return (
    <aside className="model-provider-pane">
      <div className="model-provider-search-wrap">
        <SearchIcon />
        <input
          type="search"
          value={search}
          placeholder={t("settings.tools.searchPlaceholder")}
          onChange={(e) => onSearchChange(e.target.value)}
        />
      </div>
      <div className="model-provider-list">
        {filteredTools.map((tool) => {
          const enabled = !disabled.has(tool.name);
          return (
            <div
              key={tool.name}
              className={`model-provider-item ${
                tool.name === selectedName ? "active" : ""
              } ${!enabled ? "is-disabled" : ""}`}
            >
              <button
                type="button"
                className="model-provider-item-body settings-tool-item-body"
                onClick={() => onSelect(tool)}
              >
                <span className="model-provider-avatar settings-tool-avatar">
                  <ToolGlyph tool={tool.name} />
                </span>
                <span className="model-provider-name settings-tool-name">
                  <span className="model-provider-name-text">
                    {tool.name}
                    {forced.has(tool.name) && (
                      <span className="settings-tool-force-tag">
                        {t("settings.tools.forceBadge")}
                      </span>
                    )}
                  </span>
                  <span className="settings-tool-list-desc">
                    {listToolDescription(t, tool, descriptions)}
                  </span>
                </span>
              </button>
              <ProviderEnableSwitch
                enabled={enabled}
                onChange={(next) => onSetEnabled(tool.name, next)}
                title={
                  enabled
                    ? t("settings.tools.disableTool")
                    : t("settings.tools.enableTool")
                }
              />
            </div>
          );
        })}
        {tools.length === 0 && (
          <div className="model-provider-empty">{t("settings.tools.empty")}</div>
        )}
        {tools.length > 0 && filteredTools.length === 0 && (
          <div className="model-provider-empty">
            {t("settings.tools.searchEmpty")}
          </div>
        )}
      </div>
    </aside>
  );
}
