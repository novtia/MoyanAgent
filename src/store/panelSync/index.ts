import { ensureWorkspaceSyncListener } from "./bus";
import { registerFileExplorerSync } from "./handlers/fileExplorerSync";
import { registerReaderSync } from "./handlers/readerSync";
import { registerRoleStateSync } from "./handlers/roleStateSync";

let initialized = false;

/**
 * Bootstrap the right-panel synchronization system: registers all built-in
 * panel handlers and attaches the single `workspace://sync` Tauri listener.
 */
export function initPanelSync(): void {
  if (initialized) return;
  initialized = true;
  registerReaderSync();
  registerFileExplorerSync();
  registerRoleStateSync();
  ensureWorkspaceSyncListener();
}

export {
  dispatchWorkspaceSync,
  ensureWorkspaceSyncListener,
  registerPanelSyncHandler,
} from "./bus";
export type {
  PanelSyncHandler,
  SyncContext,
  WorkspaceEffect,
  WorkspaceEffectKind,
  WorkspaceSyncEvent,
} from "./types";
