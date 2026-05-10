import { useTranslation } from "react-i18next";
import type { AttachmentDraft } from "../types";
import { Composer } from "./Composer";

interface EmptyChatProps {
  onEditAttachment: (a: AttachmentDraft) => void;
  onOpenSettings: () => void;
  needsSetup: boolean;
}

export function EmptyChat({
  onEditAttachment,
  onOpenSettings,
  needsSetup,
}: EmptyChatProps) {
  const { t } = useTranslation();

  return (
    <div className="empty-chat" role="region" aria-labelledby="empty-chat-title">
      <div className="empty-chat-inner">
        <h1 id="empty-chat-title" className="empty-chat-title">
          {t("chat.heroTitle")}
        </h1>
        <Composer
          onEditAttachment={onEditAttachment}
          onOpenSettings={onOpenSettings}
          needsSetup={needsSetup}
        />
      </div>
    </div>
  );
}
