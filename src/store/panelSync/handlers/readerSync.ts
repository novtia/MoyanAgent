import { api } from "../../../api/tauri";
import { cancelReaderSave } from "../../../components/chat/rightPanel/reader/hooks/useEditorSave";
import {
  inferFileType,
  syncPendingDiffsForPath,
  syncPendingDiffsForSession,
  useReader,
} from "../../reader";
import { registerPanelSyncHandler } from "../bus";
import type { SyncContext, WorkspaceEffect } from "../types";

async function refreshTabFromDisk(
  sessionId: string,
  path: string,
  opts: { openIfMissing: boolean },
) {
  cancelReaderSave(path);
  const reader = useReader.getState();
  const existing = reader.getTabByPath(path);
  if (!existing && !opts.openIfMissing) return;

  try {
    const disk = await api.readProjectFile(sessionId, path);
    if (!existing) {
      reader.openDoc({
        path,
        text: disk.text,
        fileType: inferFileType(path),
        encoding: disk.encoding,
        hadBom: disk.hadBom,
      });
    } else {
      reader.updateTabText(path, disk.text, { dirty: false });
    }
  } catch (e) {
    console.warn("readerSync: failed to refresh file from disk", path, e);
  }
}

async function handleReaderEffect(effect: WorkspaceEffect, ctx: SyncContext) {
  if (!ctx.isActiveView || !ctx.activeSessionId) return;
  const sessionId = ctx.activeSessionId;

  switch (effect.kind) {
    case "file_created": {
      if (!effect.cache_in_reader || !effect.path) return;
      await refreshTabFromDisk(sessionId, effect.path, { openIfMissing: true });
      return;
    }

    case "file_updated": {
      if (!effect.path) return;
      const openIfMissing = !!effect.ensure_in_reader || !!effect.pending_diff_id;
      await refreshTabFromDisk(sessionId, effect.path, { openIfMissing });
      if (effect.pending_diff_id) {
        await syncPendingDiffsForPath(sessionId, effect.path);
      }
      return;
    }

    case "file_deleted": {
      if (!effect.path) return;
      cancelReaderSave(effect.path);
      useReader.getState().closeByPaths([effect.path]);
      return;
    }

    case "file_renamed": {
      if (!effect.from || !effect.to) return;
      cancelReaderSave(effect.from);
      useReader.getState().remapPath(effect.from, effect.to);
      return;
    }

    case "workspace_rolled_back": {
      await syncPendingDiffsForSession(sessionId);
      const reader = useReader.getState();
      for (const tab of reader.tabs) {
        cancelReaderSave(tab.path);
        try {
          const disk = await api.readProjectFile(sessionId, tab.path);
          reader.updateTabText(tab.path, disk.text, { dirty: false });
        } catch (e) {
          console.warn("readerSync: rollback read failed", tab.path, e);
        }
      }
      return;
    }

    default:
      return;
  }
}

export function registerReaderSync(): () => void {
  return registerPanelSyncHandler(
    "reader",
    [
      "file_created",
      "file_updated",
      "file_deleted",
      "file_renamed",
      "workspace_rolled_back",
    ],
    handleReaderEffect,
  );
}
