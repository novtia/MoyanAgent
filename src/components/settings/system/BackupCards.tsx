import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { api } from "../../../api/tauri";
import { useSettings } from "../../../store/settings";
import type {
  BackupListItem,
  BackupProgressEvent,
  BackupStatus,
} from "../../../types";
import { copyText } from "../../../utils/clipboard";
import { dialog } from "../../ui/Dialog";
import { toast } from "../../ui/Toast";
import { SettingsSelectDropdown } from "../SettingsSelectDropdown";
import { PathRow } from "./PathRow";

const SAVE_DEBOUNCE_MS = 500;
const STATUS_POLL_MS = 15_000;

function formatTime(ms: number | null | undefined): string {
  if (ms == null || !Number.isFinite(ms) || ms <= 0) return "—";
  try {
    return new Date(ms).toLocaleString();
  } catch {
    return "—";
  }
}

function formatBytes(n: number): string {
  if (!Number.isFinite(n) || n < 0) return "—";
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / (1024 * 1024)).toFixed(1)} MB`;
}

function progressPercent(event: BackupProgressEvent | null): number {
  if (!event) return 0;
  if (typeof event.percent === "number" && Number.isFinite(event.percent)) {
    return Math.max(0, Math.min(100, Math.round(event.percent)));
  }
  if (event.phase === "done") return 100;
  if (event.total > 0) {
    return Math.min(99, Math.round((event.current / event.total) * 100));
  }
  return event.phase === "preparing" ? 1 : 0;
}

export function BackupCards() {
  const { t } = useTranslation();
  const settings = useSettings((s) => s.settings);
  const update = useSettings((s) => s.update);
  const [status, setStatus] = useState<BackupStatus | null>(null);
  const [backups, setBackups] = useState<BackupListItem[]>([]);
  const [busy, setBusy] = useState(false);
  const [progress, setProgress] = useState<BackupProgressEvent | null>(null);
  const [showList, setShowList] = useState(false);
  const [copied, setCopied] = useState(false);
  const [configKeep, setConfigKeep] = useState(
    String(settings?.auto_backup_config_keep ?? 14),
  );
  const [chatKeep, setChatKeep] = useState(
    String(settings?.auto_backup_chat_keep ?? 48),
  );
  const configKeepTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const chatKeepTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

  const enabled = settings?.auto_backup_enabled ?? false;
  const interval = String(settings?.auto_backup_chat_interval_minutes ?? 30);
  const backupDir = status?.backup_dir || settings?.auto_backup_dir || "";

  const refresh = useCallback(async () => {
    try {
      const [st, list] = await Promise.all([
        api.getBackupStatus(),
        api.listBackups(),
      ]);
      setStatus(st);
      setBackups(list.slice(0, 20));
    } catch (e) {
      console.warn(e);
    }
  }, []);

  useEffect(() => {
    void refresh();
    const id = setInterval(() => void refresh(), STATUS_POLL_MS);
    return () => clearInterval(id);
  }, [refresh]);

  useEffect(() => {
    let unlisten: UnlistenFn | undefined;
    let cancelled = false;
    void listen<BackupProgressEvent>("backup://progress", (event) => {
      if (cancelled) return;
      setProgress(event.payload);
    }).then((fn) => {
      unlisten = fn;
    });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  useEffect(() => {
    setConfigKeep(String(settings?.auto_backup_config_keep ?? 14));
    setChatKeep(String(settings?.auto_backup_chat_keep ?? 48));
  }, [settings?.auto_backup_config_keep, settings?.auto_backup_chat_keep]);

  useEffect(
    () => () => {
      if (configKeepTimer.current) clearTimeout(configKeepTimer.current);
      if (chatKeepTimer.current) clearTimeout(chatKeepTimer.current);
    },
    [],
  );

  const copyDir = async () => {
    if (!backupDir) return;
    try {
      await copyText(backupDir);
      setCopied(true);
      setTimeout(() => setCopied(false), 1200);
    } catch (e) {
      console.warn(e);
    }
  };

  const runWithProgress = async (fn: () => Promise<void>) => {
    setBusy(true);
    setProgress({
      op: "backup",
      phase: "preparing",
      percent: 0,
      current: 0,
      total: 1,
      detail: "",
    });
    try {
      await fn();
    } finally {
      setBusy(false);
      setProgress(null);
    }
  };

  const onManualFull = async () => {
    const dest = await saveDialog({
      title: t("settings.system.backupSaveTitle"),
      defaultPath: `atelier-full-${new Date().toISOString().slice(0, 10)}.zip`,
      filters: [{ name: "ZIP", extensions: ["zip"] }],
    });
    if (!dest) return;
    await runWithProgress(async () => {
      try {
        const result = await api.createBackup("full", dest as string);
        toast.success(t("settings.system.backupSuccess"), {
          description: result.path,
        });
        await refresh();
      } catch (e) {
        toast.error(t("settings.system.backupFailed"), {
          description: String(e),
        });
      }
    });
  };

  const onRestore = async () => {
    const archive = await openDialog({
      multiple: false,
      title: t("settings.system.restorePickTitle"),
      filters: [{ name: "ZIP", extensions: ["zip"] }],
    });
    if (!archive) return;
    const ok = await dialog.confirm(t("settings.system.restoreConfirm"), {
      type: "danger",
      title: t("settings.system.restoreTitle"),
      confirmLabel: t("settings.system.restoreAction"),
    });
    if (!ok) return;
    await runWithProgress(async () => {
      try {
        const result = await api.restoreBackup(archive as string);
        const warning = result.warnings?.filter(Boolean).join(" · ");
        toast.success(t("settings.system.restoreSuccess"), {
          description: warning
            ? t("settings.system.restoreWarning", { warning })
            : t("settings.system.restoreRestartHint"),
          duration: 8000,
        });
        const reload = await dialog.confirm(
          t("settings.system.restoreRestartHint"),
          {
            title: t("settings.system.restoreSuccess"),
            confirmLabel: t("settings.system.restoreReloadNow"),
          },
        );
        if (reload) {
          window.location.reload();
        }
        await refresh();
      } catch (e) {
        toast.error(t("settings.system.restoreFailed"), {
          description: String(e),
        });
      }
    });
  };

  const onModuleBackup = async (module: "config" | "chat") => {
    await runWithProgress(async () => {
      try {
        const result = await api.createBackup(module);
        toast.success(t("settings.system.backupSuccess"), {
          description: result.path,
        });
        await refresh();
      } catch (e) {
        toast.error(t("settings.system.backupFailed"), {
          description: String(e),
        });
      }
    });
  };

  const onPickBackupDir = async () => {
    const dir = await openDialog({
      directory: true,
      multiple: false,
      title: t("settings.system.backupDirPickTitle"),
    });
    if (!dir) return;
    await update({ auto_backup_dir: dir as string });
    await refresh();
  };

  const onConfigKeepChange = (value: string) => {
    setConfigKeep(value);
    const n = Number.parseInt(value, 10);
    if (!Number.isFinite(n) || n < 1) return;
    if (configKeepTimer.current) clearTimeout(configKeepTimer.current);
    configKeepTimer.current = setTimeout(() => {
      void update({ auto_backup_config_keep: Math.min(n, 365) });
    }, SAVE_DEBOUNCE_MS);
  };

  const onChatKeepChange = (value: string) => {
    setChatKeep(value);
    const n = Number.parseInt(value, 10);
    if (!Number.isFinite(n) || n < 1) return;
    if (chatKeepTimer.current) clearTimeout(chatKeepTimer.current);
    chatKeepTimer.current = setTimeout(() => {
      void update({ auto_backup_chat_keep: Math.min(n, 1000) });
    }, SAVE_DEBOUNCE_MS);
  };

  const intervalOptions = [
    { value: "15", label: t("settings.system.backupInterval15") },
    { value: "30", label: t("settings.system.backupInterval30") },
    { value: "60", label: t("settings.system.backupInterval60") },
  ];

  const progressOp = progress?.op === "restore" ? "restore" : "backup";
  const phaseKey =
    progressOp === "restore"
      ? `settings.system.restoreProgressPhase.${progress?.phase || "preparing"}`
      : `settings.system.backupProgressPhase.${progress?.phase || "preparing"}`;
  const phaseLabel = t(phaseKey, {
    defaultValue: progress?.phase || "",
  });
  const pct = progressPercent(progress);

  return (
    <>
      <div className="settings-card">
        <div className="settings-block">
          <div className="settings-block-head">
            <div className="settings-row-title">
              {t("settings.system.backupTitle")}
            </div>
            <div className="settings-row-desc">
              {t("settings.system.backupDesc")}
            </div>
          </div>
        </div>

        <div className="settings-row">
          <div className="settings-row-main">
            <div className="settings-row-title">
              {t("settings.system.backupManualTitle")}
            </div>
            <div className="settings-row-desc">
              {t("settings.system.backupManualDesc", {
                full: formatTime(status?.last_full_at_ms),
                config: status?.last_config_slot || "—",
                chat: formatTime(status?.last_chat_at_ms),
              })}
            </div>
          </div>
          <div className="settings-row-control settings-row-control--actions">
            <button
              type="button"
              className="btn primary"
              disabled={busy || status?.busy}
              onClick={() => void onManualFull()}
            >
              {t("settings.system.backupNow")}
            </button>
            <button
              type="button"
              className="btn"
              disabled={busy || status?.busy}
              onClick={() => void onRestore()}
            >
              {t("settings.system.restoreNow")}
            </button>
          </div>
        </div>

        {busy && progress ? (
          <div className="settings-backup-progress" aria-live="polite">
            <div className="settings-backup-progress-head">
              <span>
                {progressOp === "restore"
                  ? t("settings.system.restoreProgressTitle")
                  : t("settings.system.backupProgressTitle")}
              </span>
              <span className="settings-backup-progress-pct">{pct}%</span>
            </div>
            <div
              className="settings-backup-progress-track"
              role="progressbar"
              aria-valuemin={0}
              aria-valuemax={100}
              aria-valuenow={pct}
            >
              <div
                className="settings-backup-progress-fill"
                style={{ width: `${pct}%` }}
              />
            </div>
            <div className="settings-backup-progress-detail">
              {phaseLabel}
              {progress.detail ? ` · ${progress.detail}` : ""}
            </div>
          </div>
        ) : null}
      </div>

      <div className="settings-card">
        <div className="settings-row">
          <div className="settings-row-main">
            <div className="settings-row-title">
              {t("settings.system.autoBackupTitle")}
            </div>
            <div className="settings-row-desc">
              {t("settings.system.autoBackupDesc")}
            </div>
          </div>
          <div className="settings-row-control">
            <button
              type="button"
              role="switch"
              aria-checked={enabled}
              aria-label={t("settings.system.autoBackupTitle")}
              className={`settings-toggle ${enabled ? "settings-toggle--on" : ""}`}
              onClick={() => void update({ auto_backup_enabled: !enabled })}
            >
              <span className="settings-toggle-thumb" />
            </button>
          </div>
        </div>

        <PathRow
          label={t("settings.system.backupDirLabel")}
          path={backupDir || undefined}
          copied={copied}
          onCopy={() => void copyDir()}
          onOpen={() => backupDir && api.openPath(backupDir).catch(console.warn)}
        />

        <div className="settings-row">
          <div className="settings-row-main">
            <div className="settings-row-title">
              {t("settings.system.backupDirChangeTitle")}
            </div>
            <div className="settings-row-desc">
              {t("settings.system.backupDirChangeDesc")}
            </div>
          </div>
          <div className="settings-row-control settings-row-control--actions">
            <button
              type="button"
              className="btn"
              onClick={() => void onPickBackupDir()}
            >
              {t("settings.system.backupDirPick")}
            </button>
            {settings?.auto_backup_dir ? (
              <button
                type="button"
                className="btn"
                onClick={() => void update({ auto_backup_dir: "" }).then(refresh)}
              >
                {t("settings.system.backupDirReset")}
              </button>
            ) : null}
          </div>
        </div>

        <div className="settings-row">
          <div className="settings-row-main">
            <div className="settings-row-title">
              {t("settings.system.chatIntervalTitle")}
            </div>
            <div className="settings-row-desc">
              {t("settings.system.chatIntervalDesc")}
            </div>
          </div>
          <div className="settings-row-control">
            <SettingsSelectDropdown
              value={interval}
              options={intervalOptions}
              ariaLabel={t("settings.system.chatIntervalTitle")}
              onChange={(v) =>
                void update({
                  auto_backup_chat_interval_minutes: Number.parseInt(v, 10),
                })
              }
            />
          </div>
        </div>

        <div className="settings-row">
          <div className="settings-row-main">
            <div className="settings-row-title">
              {t("settings.system.configKeepTitle")}
            </div>
            <div className="settings-row-desc">
              {t("settings.system.configKeepDesc")}
            </div>
          </div>
          <div className="settings-row-control">
            <input
              type="number"
              min={1}
              max={365}
              className="settings-number"
              value={configKeep}
              onChange={(e) => onConfigKeepChange(e.target.value)}
            />
          </div>
        </div>

        <div className="settings-row">
          <div className="settings-row-main">
            <div className="settings-row-title">
              {t("settings.system.chatKeepTitle")}
            </div>
            <div className="settings-row-desc">
              {t("settings.system.chatKeepDesc")}
            </div>
          </div>
          <div className="settings-row-control">
            <input
              type="number"
              min={1}
              max={1000}
              className="settings-number"
              value={chatKeep}
              onChange={(e) => onChatKeepChange(e.target.value)}
            />
          </div>
        </div>

        {status?.last_error ? (
          <div className="footnote" style={{ padding: "0 16px 12px" }}>
            {t("settings.system.backupLastError", { error: status.last_error })}
          </div>
        ) : null}

        <div className="settings-row">
          <div className="settings-row-main">
            <div className="settings-row-title">
              {t("settings.system.backupListTitle")}
            </div>
            <div className="settings-row-desc">
              {t("settings.system.backupListDesc")}
            </div>
          </div>
          <div className="settings-row-control settings-row-control--actions">
            <button
              type="button"
              className="btn"
              disabled={busy || status?.busy}
              onClick={() => void onModuleBackup("config")}
            >
              {t("settings.system.backupConfigNow")}
            </button>
            <button
              type="button"
              className="btn"
              disabled={busy || status?.busy}
              onClick={() => void onModuleBackup("chat")}
            >
              {t("settings.system.backupChatNow")}
            </button>
            <button
              type="button"
              className="btn"
              onClick={() => {
                setShowList((v) => !v);
                void refresh();
              }}
            >
              {showList
                ? t("settings.system.backupListHide")
                : t("settings.system.backupListShow")}
            </button>
          </div>
        </div>

        {showList ? (
          <div className="settings-backup-list">
            {backups.length === 0 ? (
              <div className="footnote">{t("settings.system.backupListEmpty")}</div>
            ) : (
              backups.map((item) => (
                <div key={item.path} className="settings-backup-item">
                  <div className="settings-backup-item-main">
                    <div className="settings-backup-item-title">
                      {t(`settings.system.backupModule.${item.module}`)} ·{" "}
                      {t(`settings.system.backupKind.${item.kind}`)}
                    </div>
                    <div className="settings-backup-item-meta">
                      {formatTime(item.created_at)} · {formatBytes(item.size_bytes)}
                    </div>
                    <code className="settings-backup-item-path" title={item.path}>
                      {item.path}
                    </code>
                  </div>
                  <button
                    type="button"
                    className="btn"
                    onClick={() => {
                      const parent = item.path.replace(/[\\/][^\\/]+$/, "");
                      api.openPath(parent || item.path).catch(console.warn);
                    }}
                  >
                    {t("settings.system.openInExplorer")}
                  </button>
                </div>
              ))
            )}
          </div>
        ) : null}
      </div>
    </>
  );
}
