import { useTranslation } from "react-i18next";
import { nowStamp } from "./utils";

export function DevelopingRow() {
  const { t } = useTranslation();
  return (
    <div className="msg assistant">
      <div className="msg-col">
        <div className="bubble">
          <span className="stamp">{t("message.stampGenerating", { time: nowStamp(Date.now()) })}</span>
          <div className="developing">
            <span className="dots">
              <span className="dot" />
              <span className="dot" />
              <span className="dot" />
            </span>
            <span>{t("message.generatingText")}</span>
          </div>
        </div>
      </div>
    </div>
  );
}
