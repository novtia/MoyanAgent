import { useTranslation } from "react-i18next";
import { ChatFontControls } from "./ChatFontControls";

export function ChatFontPanel() {
  const { t } = useTranslation();

  return (
    <div
      className="chat-font-panel"
      role="dialog"
      aria-label={t("chat.fontSettings")}
    >
      <ChatFontControls />
    </div>
  );
}
