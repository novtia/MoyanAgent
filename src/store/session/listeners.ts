import { listen } from "@tauri-apps/api/event";
import { useRoleState, type RoleStateOp } from "../roleState";
import { roleStateScopeForSession } from "./drafts";
import {
  appendDelta,
  applyStreamingToolCallDelta,
  applyToolEvent,
  cancellingSessions,
  scheduleStreamingMessageSync,
  streamingBuffers,
} from "./streaming";
import type {
  GenerationStatusPayload,
  GenerationStreamPayload,
  StreamBuffer,
  ToolEventPayload,
} from "./types";
import { useSession } from "./store";

let generationStreamListenerStarted = false;
let generationStatusListenerStarted = false;
let toolEventListenerStarted = false;
let roleStateResetListenerStarted = false;
let sessionTitleListenerStarted = false;

export function ensureGenerationStreamListener() {
  if (!generationStatusListenerStarted) {
    generationStatusListenerStarted = true;
    listen<GenerationStatusPayload>("gen://status", (event) => {
      const sessionId = event.payload?.session_id;
      const phase = event.payload?.phase;
      if (!sessionId || !phase) return;
      const state = useSession.getState();
      const next = { ...state.generationPhaseBySession };
      if (phase === "response") delete next[sessionId];
      else next[sessionId] = phase;
      useSession.setState({ generationPhaseBySession: next });
    }).catch((error) => {
      generationStatusListenerStarted = false;
      console.warn(error);
    });
  }

  if (!generationStreamListenerStarted) {
    generationStreamListenerStarted = true;
    listen<GenerationStreamPayload>("gen://stream", (event) => {
      const payload = event.payload;
      const sessionId = payload.session_id;
      if (sessionId && cancellingSessions.has(sessionId)) return;
      const textDelta = payload.text_delta ?? payload.delta ?? "";
      const thinkingDelta = payload.thinking_delta ?? "";
      const stage = payload.stage;
      const toolCallDelta = payload.tool_call_delta ?? null;
      if (
        !sessionId ||
        (!textDelta && !thinkingDelta && !stage && !toolCallDelta)
      )
        return;

      const requestId = payload.request_message_id || sessionId;
      // A buffer left over from a previous request (e.g. a generation that
      // finished while another session was active) must not leak into this
      // one — start from a clean slate when the requestId doesn't match.
      const existing = streamingBuffers.get(sessionId);
      const prev =
        existing && existing.requestId === requestId
          ? existing
          : { blocks: [], requestId };
      // Always work on a fresh array so React sees a new reference.
      const nextBlocks = prev.blocks.map((b) => ({ ...b }));
      if (stage) {
        nextBlocks.push({
          type: "agent_stage",
          agent_type: stage.agent_type,
          name: stage.name,
          index: stage.index,
        });
      }
      if (thinkingDelta) appendDelta(nextBlocks, "thinking", thinkingDelta);
      if (textDelta) appendDelta(nextBlocks, "text", textDelta);
      if (toolCallDelta)
        applyStreamingToolCallDelta(nextBlocks, sessionId, toolCallDelta);
      const next: StreamBuffer = { blocks: nextBlocks, requestId };
      streamingBuffers.set(sessionId, next);
      scheduleStreamingMessageSync(sessionId);
    }).catch((e) => {
      generationStreamListenerStarted = false;
      console.warn(e);
    });
  }

  if (!toolEventListenerStarted) {
    toolEventListenerStarted = true;
    listen<ToolEventPayload>("gen://tool", (event) => {
      const payload = event.payload;
      const sessionId = payload.session_id;
      if (!sessionId) return;
      if (cancellingSessions.has(sessionId)) return;
      const requestId = payload.request_message_id || sessionId;
      // Same stale-buffer guard as the gen://stream listener above.
      const existing = streamingBuffers.get(sessionId);
      const prev =
        existing && existing.requestId === requestId
          ? existing
          : { blocks: [], requestId };
      const nextBlocks = prev.blocks.map((b) => ({ ...b }));
      applyToolEvent(nextBlocks, payload, sessionId);
      // Incrementally drive the character state board off RoleState results.
      if (
        payload.type === "tool_result" &&
        payload.tool === "RoleState" &&
        !payload.is_error &&
        payload.output &&
        typeof payload.output === "object"
      ) {
        useRoleState
          .getState()
          .applyOp(roleStateScopeForSession(sessionId), payload.output as RoleStateOp);
      }
      const next: StreamBuffer = { blocks: nextBlocks, requestId };
      streamingBuffers.set(sessionId, next);
      scheduleStreamingMessageSync(sessionId);
    }).catch((e) => {
      toolEventListenerStarted = false;
      console.warn(e);
    });
  }

  if (!roleStateResetListenerStarted) {
    roleStateResetListenerStarted = true;
    listen<{ scope_id?: string; session_id?: string }>("role-state://reset", (event) => {
      const scopeId = event.payload?.scope_id;
      const sessionId = event.payload?.session_id;
      const state = useSession.getState();
      const activeScope = state.active
        ? roleStateScopeForSession(state.active.session.id)
        : null;
      if (scopeId && activeScope === scopeId && state.active) {
        void useRoleState
          .getState()
          .loadLatest(state.active.session.id, scopeId);
        return;
      }
      if (sessionId && state.activeId === sessionId) {
        void useRoleState
          .getState()
          .loadLatest(sessionId, roleStateScopeForSession(sessionId));
      }
    }).catch((e) => {
      roleStateResetListenerStarted = false;
      console.warn(e);
    });
  }

  if (!sessionTitleListenerStarted) {
    sessionTitleListenerStarted = true;
    listen<{ session_id: string; title: string }>("session://title", (event) => {
      const sessionId = event.payload?.session_id;
      const title = event.payload?.title;
      if (!sessionId || !title) return;
      const state = useSession.getState();
      useSession.setState({
        sessions: state.sessions.map((s) =>
          s.id === sessionId ? { ...s, title } : s,
        ),
      });
      if (state.activeId === sessionId && state.active) {
        const a = state.active;
        useSession.setState({
          active: { ...a, session: { ...a.session, title } },
        });
      }
    }).catch((e) => {
      sessionTitleListenerStarted = false;
      console.warn(e);
    });
  }
}
