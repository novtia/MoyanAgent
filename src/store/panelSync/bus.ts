import { listen } from "@tauri-apps/api/event";
import { useSession } from "../session/store";
import type {
  PanelSyncHandler,
  SyncContext,
  WorkspaceEffectKind,
  WorkspaceSyncEvent,
} from "./types";

interface RegisteredHandler {
  id: string;
  kinds: ReadonlySet<WorkspaceEffectKind> | null;
  handler: PanelSyncHandler;
}

const handlers = new Map<string, RegisteredHandler>();
let listenerStarted = false;

/**
 * Register a right-panel subsystem handler on the unified workspace sync bus.
 * Passing `kinds = null` subscribes to all effect kinds.
 */
export function registerPanelSyncHandler(
  id: string,
  kinds: readonly WorkspaceEffectKind[] | null,
  handler: PanelSyncHandler,
): () => void {
  handlers.set(id, {
    id,
    kinds: kinds ? new Set(kinds) : null,
    handler,
  });
  return () => {
    const current = handlers.get(id);
    if (current?.handler === handler) {
      handlers.delete(id);
    }
  };
}

function buildSyncContext(event: WorkspaceSyncEvent): SyncContext {
  const sessionId = event.session_id;
  const ownerSessionId = event.owner_session_id || sessionId;
  const state = useSession.getState();
  const activeSessionId = state.activeId;

  const isActiveView =
    !!activeSessionId &&
    (sessionId === activeSessionId || ownerSessionId === activeSessionId);

  let isActiveProject = isActiveView;
  if (!isActiveProject && activeSessionId) {
    const eventProject =
      state.sessions.find((s) => s.id === ownerSessionId)?.project_id ??
      state.sessions.find((s) => s.id === sessionId)?.project_id ??
      null;
    const activeProject =
      state.active?.session.project_id ??
      state.sessions.find((s) => s.id === activeSessionId)?.project_id ??
      null;
    if (eventProject && activeProject && eventProject === activeProject) {
      isActiveProject = true;
    }
  }

  return {
    sessionId,
    ownerSessionId,
    activeSessionId,
    isActiveView,
    isActiveProject,
    source: event.source,
  };
}

/**
 * Dispatch a `WorkspaceSyncEvent` through all registered panel handlers.
 * Used both by the backend `workspace://sync` listener and by local frontend
 * operations (e.g. post-rollback refresh) so all right-panel sync flows through
 * one pipeline.
 */
export async function dispatchWorkspaceSync(event: WorkspaceSyncEvent): Promise<void> {
  if (!event.effects || event.effects.length === 0) return;
  const ctx = buildSyncContext(event);
  const snapshot = Array.from(handlers.values());

  for (const effect of event.effects) {
    const tasks: Array<void | Promise<void>> = [];
    for (const entry of snapshot) {
      if (entry.kinds && !entry.kinds.has(effect.kind)) continue;
      try {
        tasks.push(entry.handler(effect, ctx));
      } catch (e) {
        console.warn(`panelSync[${entry.id}] threw synchronously on ${effect.kind}`, e);
      }
    }
    if (tasks.length > 0) {
      await Promise.allSettled(tasks);
    }
  }
}

/** Idempotently start listening for backend `workspace://sync` events. */
export function ensureWorkspaceSyncListener(): void {
  if (listenerStarted) return;
  listenerStarted = true;
  listen<WorkspaceSyncEvent>("workspace://sync", (event) => {
    if (!event.payload) return;
    void dispatchWorkspaceSync(event.payload);
  }).catch((e) => {
    listenerStarted = false;
    console.warn("panelSync: failed to attach workspace://sync listener", e);
  });
}
