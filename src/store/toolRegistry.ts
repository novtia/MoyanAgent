import { api } from "../api/tauri";
import type { AgentToolSpec } from "../types";

/**
 * Frontend mirror of the backend `ToolPool` registration metadata.
 *
 * Synced once at startup via `list_agent_tool_specs` so the UI never needs to
 * hardcode lists like `STREAMING_INPUT_TOOLS` when new tools are registered on
 * the backend.
 */
const specsByName = new Map<string, AgentToolSpec>();
let initStarted = false;
let initPromise: Promise<void> | null = null;

/**
 * Fallback map used only during the brief window before `list_agent_tool_specs`
 * resolves on cold start. Once backend specs arrive, backend registration is
 * authoritative.
 */
const BOOTSTRAP_STREAMING_FIELDS: Record<string, string[]> = {
  CreateDoc: ["title", "doc_type", "content"],
  Edit: ["path", "old_string", "new_string"],
  Write: ["path", "content"],
};

export function initToolRegistry(): Promise<void> {
  if (initStarted && initPromise) return initPromise;
  initStarted = true;
  initPromise = api
    .listAgentToolSpecs()
    .then((specs) => {
      specsByName.clear();
      for (const spec of specs) {
        specsByName.set(spec.name, spec);
      }
    })
    .catch((e) => {
      initStarted = false;
      console.warn("toolRegistry: failed to sync backend tool specs", e);
    });
  return initPromise;
}

/**
 * Streaming input field names declared by `toolName` on the backend, or `null`
 * if the tool does not stream partial arguments into tool cards.
 */
export function getStreamingInputFields(toolName: string): string[] | null {
  const spec = specsByName.get(toolName);
  if (spec) {
    const fields = spec.uiMeta?.streamingInputFields;
    return fields && fields.length > 0 ? fields : null;
  }
  if (specsByName.size === 0) {
    return BOOTSTRAP_STREAMING_FIELDS[toolName] ?? null;
  }
  return null;
}

export function getRegisteredToolSpec(toolName: string): AgentToolSpec | undefined {
  return specsByName.get(toolName);
}
