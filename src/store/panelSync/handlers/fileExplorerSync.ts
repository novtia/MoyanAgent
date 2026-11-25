import { useFileExplorer } from "../../fileExplorer";
import { registerPanelSyncHandler } from "../bus";
import type { SyncContext, WorkspaceEffect } from "../types";

function handleFileExplorerEffect(effect: WorkspaceEffect, ctx: SyncContext) {
  if (!ctx.isActiveProject) return;

  switch (effect.kind) {
    case "file_renamed":
      if (effect.from && effect.to) {
        useFileExplorer.getState().noteRename(effect.from, effect.to);
      }
      useFileExplorer.getState().bumpTree();
      return;

    case "file_created":
    case "file_updated":
    case "file_deleted":
    case "fs_tree_dirty":
    case "workspace_rolled_back":
      useFileExplorer.getState().bumpTree();
      return;

    default:
      return;
  }
}

export function registerFileExplorerSync(): () => void {
  return registerPanelSyncHandler(
    "file-explorer",
    [
      "file_created",
      "file_updated",
      "file_deleted",
      "file_renamed",
      "fs_tree_dirty",
      "workspace_rolled_back",
    ],
    handleFileExplorerEffect,
  );
}
