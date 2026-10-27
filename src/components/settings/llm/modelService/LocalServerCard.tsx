import { useCallback, useEffect, useState } from "react";
import { api } from "../../../../api/tauri";
import { toast } from "../../../ui/Toast";

export function LocalServerCard({
  providerId,
  serverExe,
  modelPath,
  onServerExe,
  onModelPath,
  onBeforeStart,
}: {
  providerId: string;
  serverExe: string;
  modelPath: string;
  onServerExe: (value: string) => void;
  onModelPath: (value: string) => void;
  onBeforeStart?: () => void | Promise<void>;
}) {
  const [healthy, setHealthy] = useState(false);
  const [owned, setOwned] = useState(false);
  const [busy, setBusy] = useState<"start" | "stop" | null>(null);

  const refresh = useCallback(async () => {
    try {
      const status = await api.localLlamaServerStatus(providerId);
      setHealthy(status.healthy);
      setOwned(status.owned);
    } catch {
      setHealthy(false);
      setOwned(false);
    }
  }, [providerId]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const start = async () => {
    setBusy("start");
    try {
      await onBeforeStart?.();
      const status = await api.startLocalLlamaServer(providerId);
      setHealthy(status.healthy);
      setOwned(status.owned);
      if (status.healthy && !status.owned) {
        toast.success("本地服务已在运行");
      } else if (status.healthy) {
        toast.success("本地服务已启动");
      }
    } catch (err) {
      toast.error("启动本地服务失败", { description: String(err) });
      await refresh();
    } finally {
      setBusy(null);
    }
  };

  const stop = async () => {
    setBusy("stop");
    try {
      const status = await api.stopLocalLlamaServer(providerId);
      setHealthy(status.healthy);
      setOwned(status.owned);
      toast.success("已停止本地服务");
    } catch (err) {
      toast.error("无法停止", { description: String(err) });
    } finally {
      setBusy(null);
    }
  };

  const statusText = busy === "start"
    ? "正在启动…"
    : busy === "stop"
      ? "正在停止…"
      : healthy
        ? "运行中"
        : "未运行";

  return (
    <div className="model-modal-section">
      <div className="model-modal-section-title">本地服务</div>
      <div className="row">
        <label className="field-label">服务程序</label>
        <input
          type="text"
          value={serverExe}
          spellCheck={false}
          placeholder="llama-server.exe"
          onChange={(e) => onServerExe(e.target.value)}
        />
      </div>
      <div className="row">
        <label className="field-label">模型文件</label>
        <input
          type="text"
          value={modelPath}
          spellCheck={false}
          placeholder="模型 .gguf 路径"
          onChange={(e) => onModelPath(e.target.value)}
        />
        <div className="hint">启动日志写到模型文件所在目录。</div>
      </div>
      <div className="local-server-actions">
        <span className={`local-server-status${healthy ? " is-on" : ""}`}>{statusText}</span>
        <button
          type="button"
          className="btn primary"
          disabled={busy != null || healthy}
          onClick={() => void start()}
        >
          启动本地服务
        </button>
        <button
          type="button"
          className="btn"
          disabled={busy != null || (!healthy && !owned)}
          onClick={() => void stop()}
        >
          停止
        </button>
      </div>
    </div>
  );
}
