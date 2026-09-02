import type { MouseEvent as ReactMouseEvent } from "react";
import type { ModelProvider, ProviderSdkConfig } from "../../../../types";
import { providerSdkLabel } from "../modelServices";
import { ProviderAvatarDisplay } from "../ProviderBrandIcon";
import { PlusIcon, SearchIcon } from "./icons";
import { ProviderEnableSwitch } from "./ProviderEnableSwitch";

export function ProviderPane({
  providers,
  filteredProviders,
  selectedProviderId,
  providerSearch,
  sdkOptions,
  onSearchChange,
  onSelect,
  onContextMenu,
  onSetEnabled,
  onAdd,
}: {
  providers: ModelProvider[];
  filteredProviders: ModelProvider[];
  selectedProviderId?: string;
  providerSearch: string;
  sdkOptions: readonly ProviderSdkConfig[];
  onSearchChange: (value: string) => void;
  onSelect: (provider: ModelProvider) => void;
  onContextMenu: (event: ReactMouseEvent, provider: ModelProvider) => void;
  onSetEnabled: (providerId: string, enabled: boolean) => void;
  onAdd: () => void;
}) {
  return (
    <aside className="model-provider-pane">
      <div className="model-provider-search-wrap">
        <SearchIcon />
        <input
          type="search"
          value={providerSearch}
          placeholder="搜索模型平台..."
          onChange={(e) => onSearchChange(e.target.value)}
        />
      </div>
      <div className="model-provider-list">
        {filteredProviders.map((provider) => {
          const provOn = provider.enabled !== false;
          return (
            <div
              key={provider.id}
              className={`model-provider-item ${
                provider.id === selectedProviderId ? "active" : ""
              } ${!provOn ? "is-disabled" : ""}`}
              onContextMenu={(event) => onContextMenu(event, provider)}
            >
              <button
                type="button"
                className="model-provider-item-body"
                onClick={() => onSelect(provider)}
              >
                <ProviderAvatarDisplay
                  name={provider.name}
                  avatar={provider.avatar}
                  sdk={provider.sdk}
                />
                <span className="model-provider-name">
                  <span className="model-provider-name-text">{provider.name}</span>
                  <span className="model-provider-sdk">
                    {providerSdkLabel(provider.sdk, sdkOptions)}
                  </span>
                </span>
              </button>
              <ProviderEnableSwitch
                enabled={provOn}
                onChange={(next) => onSetEnabled(provider.id, next)}
                title={provOn ? "停用供应商" : "启用供应商"}
              />
            </div>
          );
        })}
        {providers.length === 0 && (
          <div className="model-provider-empty">还没有供应商</div>
        )}
      </div>
      <button type="button" className="btn model-provider-add" onClick={onAdd}>
        <PlusIcon />
        <span>添加供应商</span>
      </button>
    </aside>
  );
}
