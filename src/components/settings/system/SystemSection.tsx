import { copyText } from "../../../utils/clipboard";
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { api } from "../../../api/tauri";
import { useNotifySound } from "../../../store/notifySound";
import { useSettings } from "../../../store/settings";
import type { AppInfo } from "../types";
import { BackupCards } from "./BackupCards";
import { PathRow } from "./PathRow";

const SAVE_DEBOUNCE_MS = 500;

const PROXY_SCHEMES = new Set(["http:", "https:", "socks5:", "socks5h:"]);

function proxyUrlError(
  enabled: boolean,
  url: string,
  t: (key: string) => string,
): string | null {
  const trimmed = url.trim();
  if (!trimmed) {
    return enabled ? t("settings.system.proxyUrlRequired") : null;
  }
  try {
    const parsed = new URL(trimmed);
    if (!PROXY_SCHEMES.has(parsed.protocol) || !parsed.hostname) {
      return t("settings.system.proxyUrlInvalid");
    }
  } catch {
    return t("settings.system.proxyUrlInvalid");
  }
  return null;
}

export function SystemSection() {
  const { t } = useTranslation();
  const settings = useSettings((s) => s.settings);
  const update = useSettings((s) => s.update);
  const notifySoundEnabled = useNotifySound((s) => s.enabled);
  const setNotifySoundEnabled = useNotifySound((s) => s.setEnabled);
  const [info, setInfo] = useState<AppInfo | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [copied, setCopied] = useState<string | null>(null);
  const webSearchEnabled = settings?.web_search_enabled ?? true;
  const proxyEnabled = settings?.http_proxy_enabled ?? false;
  const [maxResults, setMaxResults] = useState(
    String(settings?.web_search_max_results ?? 5),
  );
  const [proxyUrl, setProxyUrl] = useState(settings?.http_proxy_url ?? "");
  const [proxyError, setProxyError] = useState<string | null>(null);
  const maxResultsTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const proxyUrlTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    setMaxResults(String(settings?.web_search_max_results ?? 5));
  }, [settings?.web_search_max_results]);

  useEffect(() => {
    setProxyUrl(settings?.http_proxy_url ?? "");
  }, [settings?.http_proxy_url]);

  useEffect(
    () => () => {
      if (maxResultsTimer.current) clearTimeout(maxResultsTimer.current);
      if (proxyUrlTimer.current) clearTimeout(proxyUrlTimer.current);
    },
    [],
  );

  const onMaxResultsChange = (value: string) => {
    setMaxResults(value);
    const n = Number.parseInt(value, 10);
    if (!Number.isFinite(n) || n < 1) return;
    if (maxResultsTimer.current) clearTimeout(maxResultsTimer.current);
    maxResultsTimer.current = setTimeout(() => {
      void update({ web_search_max_results: Math.min(n, 20) });
    }, SAVE_DEBOUNCE_MS);
  };

  const onProxyUrlChange = (value: string) => {
    setProxyUrl(value);
    const err = proxyUrlError(proxyEnabled, value, t);
    setProxyError(err);
    if (proxyUrlTimer.current) clearTimeout(proxyUrlTimer.current);
    if (err) return;
    proxyUrlTimer.current = setTimeout(() => {
      void update({ http_proxy_url: value.trim() }).catch((e) => {
        setProxyError(String(e));
      });
    }, SAVE_DEBOUNCE_MS);
  };

  const onProxyToggle = () => {
    const next = !proxyEnabled;
    const err = proxyUrlError(next, proxyUrl, t);
    setProxyError(err);
    if (err) return;
    void update({ http_proxy_enabled: next }).catch((e) => {
      setProxyError(String(e));
    });
  };

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
      await copyText(text);
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
        <div className="settings-row">
          <div className="settings-row-main">
            <div className="settings-row-title">
              {t("settings.search.enableTitle")}
            </div>
            <div className="settings-row-desc">
              {t("settings.search.enableDesc")}
            </div>
          </div>
          <div className="settings-row-control">
            <button
              type="button"
              role="switch"
              aria-checked={webSearchEnabled}
              aria-label={t("settings.search.enableTitle")}
              className={`settings-toggle ${webSearchEnabled ? "settings-toggle--on" : ""}`}
              onClick={() =>
                void update({ web_search_enabled: !webSearchEnabled })
              }
            >
              <span className="settings-toggle-thumb" />
            </button>
          </div>
        </div>

        <div className="settings-row">
          <div className="settings-row-main">
            <div className="settings-row-title">
              {t("settings.search.maxResultsTitle")}
            </div>
            <div className="settings-row-desc">
              {t("settings.search.maxResultsDesc")}
            </div>
          </div>
          <div className="settings-row-control">
            <input
              type="number"
              min={1}
              max={20}
              className="settings-number"
              value={maxResults}
              onChange={(e) => onMaxResultsChange(e.target.value)}
            />
          </div>
        </div>
      </div>

      <div className="settings-card">
        <div className="settings-row">
          <div className="settings-row-main">
            <div className="settings-row-title">
              {t("settings.system.notifySoundTitle")}
            </div>
            <div className="settings-row-desc">
              {t("settings.system.notifySoundDesc")}
            </div>
          </div>
          <div className="settings-row-control">
            <button
              type="button"
              role="switch"
              aria-checked={notifySoundEnabled}
              aria-label={t("settings.system.notifySoundTitle")}
              title={t("settings.system.notifySoundTitle")}
              className={`settings-toggle ${notifySoundEnabled ? "settings-toggle--on" : ""}`}
              onClick={() => setNotifySoundEnabled(!notifySoundEnabled)}
            >
              <span className="settings-toggle-thumb" />
            </button>
          </div>
        </div>
      </div>

      <div className="settings-card">
        <div className="settings-row">
          <div className="settings-row-main">
            <div className="settings-row-title">
              {t("settings.system.proxyTitle")}
            </div>
            <div className="settings-row-desc">
              {t("settings.system.proxyDesc")}
            </div>
          </div>
          <div className="settings-row-control">
            <button
              type="button"
              role="switch"
              aria-checked={proxyEnabled}
              aria-label={t("settings.system.proxyTitle")}
              title={t("settings.system.proxyTitle")}
              className={`settings-toggle ${proxyEnabled ? "settings-toggle--on" : ""}`}
              onClick={onProxyToggle}
            >
              <span className="settings-toggle-thumb" />
            </button>
          </div>
        </div>

        <div className="settings-row settings-row--stack">
          <div className="settings-row-main">
            <div className="settings-row-title">
              {t("settings.system.proxyUrlTitle")}
            </div>
            <div className="settings-row-desc">
              {t("settings.system.proxyUrlDesc")}
            </div>
          </div>
          <div className="row">
            <input
              type="text"
              value={proxyUrl}
              spellCheck={false}
              autoComplete="off"
              placeholder={t("settings.system.proxyUrlPlaceholder")}
              aria-invalid={proxyError ? true : undefined}
              aria-label={t("settings.system.proxyUrlTitle")}
              onChange={(e) => onProxyUrlChange(e.target.value)}
            />
            {proxyError && <div className="hint is-error">{proxyError}</div>}
          </div>
        </div>
      </div>

      <BackupCards />

      <div className="settings-card">
        <div className="settings-row">
          <div className="settings-row-main">
            <div className="settings-row-title">
              {t("settings.system.infoTitle")}
            </div>
            <div className="settings-row-desc">
              {t("settings.system.infoDesc")}
            </div>
          </div>
          <div className="settings-row-control">
            <span className="settings-version-pill" title={t("settings.system.version")}>
              {info?.version || "—"}
            </span>
          </div>
        </div>
      </div>

      <div className="settings-card">
        <div className="settings-block">
          <div className="settings-block-head">
            <div className="settings-row-title">
              {t("settings.system.dataDirTitle")}
            </div>
            <div className="settings-row-desc">
              {t("settings.system.dataDirDesc")}
            </div>
          </div>
          {error && (
            <div className="footnote" style={{ marginTop: 12 }}>
              {t("settings.system.readFailed", { error })}
            </div>
          )}
        </div>

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
    </>
  );
}
