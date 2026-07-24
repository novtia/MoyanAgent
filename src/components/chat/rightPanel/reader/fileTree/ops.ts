import { api } from "../../../../../api/tauri";
import { toast } from "../../../../ui/Toast";
import { useReader } from "../../../../../store/reader";
import { baseName, joinPath } from "../../../../../store/fileExplorer";

export function splitNameExt(name: string): { base: string; ext: string } {
  const dot = name.lastIndexOf(".");
  if (dot <= 0) return { base: name, ext: "" };
  return { base: name.slice(0, dot), ext: name.slice(dot) };
}

export async function toggleRule(
  sessionId: string,
  path: string,
  enabled: boolean,
  refresh: () => void,
) {
  try {
    await api.setProjectRuleEnabled(sessionId, path, enabled);
    refresh();
  } catch {
    /* ignore */
  }
}

export async function pasteInto(
  sessionId: string,
  clipboard: { mode: "copy" | "cut"; paths: string[] } | null,
  setClipboard: (c: { mode: "copy" | "cut"; paths: string[] } | null) => void,
  dir: string,
  refresh: () => void,
  t: (k: string) => string,
) {
  if (!clipboard || clipboard.paths.length === 0) return;
  try {
    const remaps: { from: string; to: string }[] = [];
    for (const from of clipboard.paths) {
      const target = joinPath(dir, baseName(from));
      if (clipboard.mode === "cut") {
        await api.renameProjectPath(sessionId, from, target);
        remaps.push({ from, to: target });
      } else {
        await api.copyProjectPath(sessionId, from, target);
      }
    }
    if (remaps.length > 0) useReader.getState().remapPaths(remaps);
    if (clipboard.mode === "cut") setClipboard(null);
    toast.success(t("fileExplorer.pasted"));
    refresh();
  } catch (err) {
    toast.error(t("fileExplorer.pasteFailed"), { description: String(err) });
  }
}
