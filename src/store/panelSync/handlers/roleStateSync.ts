import { useRoleState } from "../../roleState";
import { roleStateScopeForSession } from "../../session/drafts";
import { useSession } from "../../session/store";
import { registerPanelSyncHandler } from "../bus";
import type { SyncContext, WorkspaceEffect } from "../types";

function handleRoleStateEffect(effect: WorkspaceEffect, ctx: SyncContext) {
  switch (effect.kind) {
    case "role_state_op": {
      const scopeId =
        effect.scope_id || roleStateScopeForSession(ctx.ownerSessionId);
      useRoleState.getState().applyOp(scopeId, effect.op);
      return;
    }

    case "role_state_reset": {
      const state = useSession.getState();
      const activeScope = state.active
        ? roleStateScopeForSession(state.active.session.id)
        : null;
      if (effect.scope_id && activeScope === effect.scope_id && state.active) {
        void useRoleState
          .getState()
          .loadLatest(state.active.session.id, effect.scope_id);
        return;
      }
      if (ctx.isActiveView && ctx.activeSessionId) {
        void useRoleState
          .getState()
          .loadLatest(
            ctx.activeSessionId,
            roleStateScopeForSession(ctx.activeSessionId),
          );
      }
      return;
    }

    default:
      return;
  }
}

export function registerRoleStateSync(): () => void {
  return registerPanelSyncHandler(
    "role-state",
    ["role_state_op", "role_state_reset"],
    handleRoleStateEffect,
  );
}
