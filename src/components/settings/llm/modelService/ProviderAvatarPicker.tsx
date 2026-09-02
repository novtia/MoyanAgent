import { type ChangeEvent } from "react";
import { toast } from "../../../ui";
import {
  isBundledBrandIcon,
  isCustomAvatarImage,
} from "../modelServices";
import { ProviderAvatarDisplay } from "../ProviderBrandIcon";

function readAvatarFile(file: File) {
  return new Promise<string>((resolve, reject) => {
    if (!file.type.startsWith("image/")) {
      reject(new Error("请选择图片文件。"));
      return;
    }
    const reader = new FileReader();
    reader.onload = () => resolve(String(reader.result ?? ""));
    reader.onerror = () => reject(new Error("头像图片读取失败。"));
    reader.readAsDataURL(file);
  });
}

export function ProviderAvatarPicker({
  name,
  avatar,
  onChange,
}: {
  name: string;
  avatar: string;
  onChange: (avatar: string) => void;
}) {
  const customUpload =
    !!avatar.trim() &&
    !isBundledBrandIcon(avatar) &&
    isCustomAvatarImage(avatar);
  const pickAvatar = async (event: ChangeEvent<HTMLInputElement>) => {
    const file = event.currentTarget.files?.[0];
    event.currentTarget.value = "";
    if (!file) return;
    try {
      onChange(await readAvatarFile(file));
    } catch (error) {
      toast.error(error instanceof Error ? error.message : "头像图片读取失败。");
    }
  };

  return (
    <div className="provider-avatar-control">
      <ProviderAvatarDisplay
        name={name}
        avatar={avatar}
        className="provider-avatar-preview-image"
      />
      <label className="btn provider-avatar-upload">
        上传图片
        <input type="file" accept="image/*" onChange={pickAvatar} />
      </label>
      {customUpload && (
        <button type="button" className="btn" onClick={() => onChange("")}>
          移除
        </button>
      )}
    </div>
  );
}
