import { useEffect, useState } from "react";
import type { ModelProvider, ProviderSdkConfig } from "../../../../types";
import {
  getProviderSdkConfig,
  isKnownProviderSdk,
  normalizeProviderSdk,
} from "../modelServices";
import { ProviderAvatarDisplay } from "../ProviderBrandIcon";
import { ProviderAvatarPicker } from "./ProviderAvatarPicker";

interface EditProviderModalProps {
  sdkOptions: readonly ProviderSdkConfig[];
  provider: ModelProvider;
  onClose: () => void;
  onSave: (draft: { name: string; sdk: string; avatar: string }) => void | Promise<void>;
}

export function EditProviderModal({ sdkOptions, provider, onClose, onSave }: EditProviderModalProps) {
  const [sdk, setSdk] = useState(() => normalizeProviderSdk(provider.sdk));
  const [name, setName] = useState(() => provider.name);
  const [avatar, setAvatar] = useState(() => provider.avatar ?? "");

  useEffect(() => {
    setSdk(normalizeProviderSdk(provider.sdk));
    setName(provider.name);
    setAvatar(provider.avatar ?? "");
  }, [provider.id, provider.sdk, provider.name, provider.avatar]);

  const sdkConfig = getProviderSdkConfig(sdk, sdkOptions);
  const canSubmit = !!name.trim() && isKnownProviderSdk(sdk, sdkOptions);

  const changeSdk = (nextSdk: string) => {
    setSdk(getProviderSdkConfig(nextSdk, sdkOptions).id);
  };

  const submit = () => {
    if (!canSubmit) return;
    void onSave({
      name: name.trim(),
      sdk,
      avatar: avatar.trim(),
    });
  };

  return (
    <div className="modal-backdrop" role="presentation" onMouseDown={onClose}>
      <div
        className="modal model-settings-modal add-provider-modal"
        onMouseDown={(e) => e.stopPropagation()}
      >
        <div className="modal-head">
          <h3>编辑供应商</h3>
          <button type="button" className="close" onClick={onClose}>
            关闭
          </button>
        </div>
        <div className="modal-body">
          <div className="model-settings-form">
            <div className="provider-avatar-preview">
              <ProviderAvatarDisplay
                name={name.trim() || sdkConfig.defaultName}
                avatar={avatar}
                className="provider-avatar-preview-image"
              />
              <div>
                <strong>{name.trim() || sdkConfig.defaultName}</strong>
                <em>{sdkConfig.label}</em>
              </div>
            </div>
            <div className="row">
              <label className="field-label">
                <span className="required-star">*</span> 供应商名称
              </label>
              <input
                type="text"
                value={name}
                autoFocus
                onChange={(e) => setName(e.target.value)}
              />
            </div>
            <div className="row">
              <label className="field-label">
                <span className="required-star">*</span> 类型（SDK）
              </label>
              <select value={sdk} onChange={(e) => changeSdk(e.target.value)}>
                {!isKnownProviderSdk(sdk, sdkOptions) && (
                  <option value={sdk}>{sdk}（未注册）</option>
                )}
                {sdkOptions.map((option) => (
                  <option key={option.id} value={option.id}>
                    {option.label}
                  </option>
                ))}
              </select>
              <div className="hint">{sdkConfig.description}</div>
            </div>
            <div className="row">
              <label className="field-label">供应商头像</label>
              <ProviderAvatarPicker
                name={name.trim() || sdkConfig.defaultName}
                avatar={avatar}
                onChange={setAvatar}
              />
            </div>
          </div>
        </div>
        <div className="modal-foot">
          <button type="button" className="btn" onClick={onClose}>
            取消
          </button>
          <button
            type="button"
            className="btn primary"
            disabled={!canSubmit}
            onClick={submit}
          >
            保存
          </button>
        </div>
      </div>
    </div>
  );
}
