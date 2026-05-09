import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { api } from "../../../api/tauri";
import type { AppInfo } from "../types";
import { PathRow } from "./PathRow";

export function SystemSection() {
  const { t } = useTranslation();
  const [info, setInfo] = useState<AppInfo | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [copied, setCopied] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    api
      .getAppInfo()
      .then((result) => {
        if (!cancelled) setInfo(result);
      })
      .catch((e) => {
        if (!cancelled) setError(String(e));
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const copy = async (text: string, key: string) => {
    try {
      await writeText(text);
      setCopied(key);
      setTimeout(() => {
        setCopied((current) => (current === key ? null : current));
      }, 1200);
    } catch (e) {
      console.warn(e);
    }
  };

  const open = (path: string) => {
    api.openPath(path).catch(console.warn);
  };

  return (
    <>
      <div className="settings-card">
        <div className="settings-card-title">{t("settings.system.infoTitle")}</div>
        <div className="settings-card-desc">{t("settings.system.infoDesc")}</div>

        <div className="settings-info-list">
          <div className="settings-info-row">
            <span className="settings-info-label">{t("settings.system.version")}</span>
            <span className="settings-info-value">{info?.version || "—"}</span>
          </div>
        </div>
      </div>

      <div className="settings-card">
        <div className="settings-card-title">{t("settings.system.dataDirTitle")}</div>
        <div className="settings-card-desc">{t("settings.system.dataDirDesc")}</div>

        {error && <div className="footnote">{t("settings.system.readFailed", { error })}</div>}

        <div className="settings-info-list">
          <PathRow
            label={t("settings.system.appDataLabel")}
            path={info?.data_dir}
            copied={copied === "data_dir"}
            onCopy={() => info && copy(info.data_dir, "data_dir")}
            onOpen={() => info && open(info.data_dir)}
          />
          <PathRow
            label={t("settings.system.databaseLabel")}
            path={info?.db_path}
            copied={copied === "db_path"}
            onCopy={() => info && copy(info.db_path, "db_path")}
          />
          <PathRow
            label={t("settings.system.sessionsLabel")}
            path={info?.sessions_dir}
            copied={copied === "sessions_dir"}
            onCopy={() => info && copy(info.sessions_dir, "sessions_dir")}
            onOpen={() => info && open(info.sessions_dir)}
          />
        </div>
      </div>
    </>
  );
}
