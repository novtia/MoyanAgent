import type { AssistantBlock, MessageAbs, SessionWithMessagesAbs } from "../../types";
import { api } from "../../api/tauri";
import { useProject } from "../project";
import { useReader, syncPendingDiffsForSession } from "../reader";
import { useFileExplorer } from "../fileExplorer";
import {
  cancelStreamFlushRaf,
  streamingBuffers,
} from "./streaming";
import { useSession } from "./store";

export function isGenerationCancelled(e: unknown) {
  return String(e).includes("generation cancelled");
}

/** Bumped when the user stops, deletes, or resends — stale in-flight runs must not persist/reload. */
const generationEpochBySession = new Map<string, number>();
/** Tracks backend `generate_image` / `regenerate_image` invokes still settling after cancel. */
export const generationFlights = new Map<string, Promise<void>>();

export function getGenerationEpoch(sessionId: string) {
  return generationEpochBySession.get(sessionId) ?? 0;
}

export function bumpGenerationEpoch(sessionId: string) {
  const next = getGenerationEpoch(sessionId) + 1;
  generationEpochBySession.set(sessionId, next);
  cancelStreamFlushRaf(sessionId);
  streamingBuffers.delete(sessionId);
  const state = useSession.getState();
  if (state.activeId === sessionId && state.active) {
    useSession.setState({
      active: {
        ...state.active,
        messages: state.active.messages.filter((m) => !m.id.startsWith("tmp-assistant-")),
      },
    });
  }
  return next;
}

export function trackGenerationFlight(sessionId: string, run: Promise<unknown>): Promise<void> {
  const flight = run.then(
    () => {},
    () => {},
  ).finally(() => {
    if (generationFlights.get(sessionId) === flight) {
      generationFlights.delete(sessionId);
    }
  });
  generationFlights.set(sessionId, flight);
  return flight;
}

export async function waitForGenerationIdle(sessionId: string) {
  const pending = generationFlights.get(sessionId);
  if (pending) await pending;
}

/**
 * Extract partial streaming content for the in-flight assistant message
 * of a given session.
 */
export function extractPartialStreamContent(
  messages: MessageAbs[],
  sessionId: string,
): { text: string; thinking: string; blocks: AssistantBlock[] } {
  const tmp = messages.find(
    (m) => m.session_id === sessionId && m.id.startsWith("tmp-assistant-"),
  );
  const blocks: AssistantBlock[] = tmp?.params?.blocks ?? [];
  const text =
    tmp?.text?.trim() ??
    blocks
      .filter((b): b is { type: "text"; content: string } => b.type === "text")
      .map((b) => b.content)
      .join("")
      .trim();
  const thinkingFromParams =
    typeof tmp?.params?.thinking_content === "string"
      ? tmp.params.thinking_content.trim()
      : "";
  const thinking =
    thinkingFromParams ||
    blocks
      .filter(
        (b): b is { type: "thinking"; content: string } => b.type === "thinking",
      )
      .map((b) => b.content)
      .join("")
      .trim();
  return { text, thinking, blocks };
}

/** Persist in-flight stream content so a session reload does not wipe the UI. */
export async function persistPartialStreamIfAny(sessionId: string) {
  const partial = extractPartialStreamContent(
    useSession.getState().active?.messages ?? [],
    sessionId,
  );
  if (!partial.text && !partial.thinking && partial.blocks.length === 0) {
    return;
  }
  try {
    await api.saveCancelledMessage(
      sessionId,
      partial.text,
      partial.thinking,
      partial.blocks.length > 0 ? partial.blocks : null,
    );
  } catch (saveErr) {
    console.warn("Failed to save partial message", saveErr);
  }
}

export async function refreshReaderAfterFileRollback(sessionId: string) {
  await syncPendingDiffsForSession(sessionId);
  const reader = useReader.getState();
  for (const tab of reader.tabs) {
    try {
      const disk = await api.readProjectFile(sessionId, tab.path);
      reader.updateTabText(tab.path, disk.text, { dirty: false });
    } catch (e) {
      console.warn("refreshReaderAfterFileRollback: read failed", tab.path, e);
    }
  }
  useFileExplorer.getState().bumpTree();
}

/**
 * The parameters actually applied to a session's generation. For project
 * sessions the project shares its sampling params / prompt / history with all
 * its sessions (so the session's own values are overridden), while `thinking`
 * stays session-owned. Mirrors the backend `effective_session_params` so the
 * request log reflects what is really sent.
 */
export function effectiveSessionForLog(session: SessionWithMessagesAbs["session"]) {
  if (!session.project_id) return session;
  const proj = useProject
    .getState()
    .projects.find((p) => p.id === session.project_id);
  if (!proj) return session;
  return {
    ...session,
    system_prompt: proj.system_prompt,
    history_turns: proj.history_turns,
    context_window: proj.context_window ?? session.context_window,
    llm_params: {
      ...proj.llm_params,
      thinking_enabled: session.llm_params?.thinking_enabled ?? null,
      thinking_effort: session.llm_params?.thinking_effort ?? null,
    },
  };
}
