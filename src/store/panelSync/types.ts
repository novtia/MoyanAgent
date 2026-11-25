import type { RoleStateOp } from "../roleState";

/**
 * Mirror of Rust `WorkspaceEffect` (`src-tauri/src/ai/agent/tools/effects.rs`).
 *
 * Every backend mutation that affects right-panel state emits one or more of
 * these effects over the single `workspace://sync` event channel.
 */
export type WorkspaceEffect =
  | {
      kind: "file_created";
      path: string;
      cache_in_reader?: boolean;
    }
  | {
      kind: "file_updated";
      path: string;
      pending_diff_id?: string | null;
      ensure_in_reader?: boolean;
    }
  | {
      kind: "file_deleted";
      path: string;
    }
  | {
      kind: "file_renamed";
      from: string;
      to: string;
    }
  | {
      kind: "fs_tree_dirty";
    }
  | {
      kind: "role_state_op";
      scope_id: string;
      op: RoleStateOp;
    }
  | {
      kind: "role_state_reset";
      scope_id: string;
    }
  | {
      kind: "workspace_rolled_back";
      paths: string[];
    };

export type WorkspaceEffectKind = WorkspaceEffect["kind"];

export interface WorkspaceSyncEvent {
  session_id: string;
  /** Owning parent session id for temp subagent sessions (defaults to session_id). */
  owner_session_id?: string;
  /** Tool or host operation name that emitted the effects (for diagnostics). */
  source?: string;
  effects: WorkspaceEffect[];
}

export interface SyncContext {
  /** Session that emitted the event (may be a temp subagent child session). */
  sessionId: string;
  /** Owning user-visible session id (parent session for subagents). */
  ownerSessionId: string;
  /** Currently active session on screen, if any. */
  activeSessionId: string | null;
  /** True when the event targets the session currently on screen (or its child). */
  isActiveView: boolean;
  /** True when the event touches the project currently on screen. */
  isActiveProject: boolean;
  source?: string;
}

export type PanelSyncHandler = (
  effect: WorkspaceEffect,
  ctx: SyncContext,
) => void | Promise<void>;
