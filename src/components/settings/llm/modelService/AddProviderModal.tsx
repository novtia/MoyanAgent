import { useState } from "react";
import type { ProviderSdkConfig } from "../../../../types";
import {
  DEFAULT_PROVIDER_SDK,
  getProviderSdkConfig,
  isKnownProviderSdk,
} from "../modelServices";
import { ProviderAvatarDisplay } from "../ProviderBrandIcon";
import { ProviderAvatarPicker } from "./ProviderAvatarPicker";

interface AddProviderModalProps {
  sdkOptions: readonly ProviderSdkConfig[];
  onClose: () => void;
  onAdd: (provider: { name: string; sdk: string; avatar: string }) => void | Promise<void>;
}

export function AddProviderModal({ sdkOptions, onClose, onAdd }: AddProviderModalProps) {
  const initialConfig = getProviderSdkConfig(DEFAULT_PROVIDER_SDK, sdkOptions);
  const [sdk, setSdk] = useState(initialConfig.id);
  const [name, setName] = useState(initialConfig.defaultName);
  const [avatar, setAvatar] = useState("");
  const [nameTouched, setNameTouched] = useState(false);

  const sdkConfig = getProviderSdkConfig(sdk, sdkOptions);
  const canSubmit = !!name.trim() && isKnownProviderSdk(sdk, sdkOptions);

  const changeSdk = (nextSdk: string) => {
    const nextConfig = getProviderSdkConfig(nextSdk, sdkOptions);
    setSdk(nextConfig.id);
    if (!nameTouched) setName(nextConfig.defaultName);
  };

  const submit = () => {
    if (!canSubmit) return;
    void onAdd({
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
          <h3>添加供应商</h3>
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
                onChange={(e) => {
                  setNameTouched(true);
                  setName(e.target.value);
                }}
              />
            </div>
            <div className="row">
              <label className="field-label">
                <span className="required-star">*</span> 类型（SDK）
              </label>
              <select value={sdk} onChange={(e) => changeSdk(e.target.value)}>
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
            添加
          </button>
        </div>
      </div>
    </div>
  );
}
