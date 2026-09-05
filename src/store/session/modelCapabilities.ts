import { useSettings } from "../settings";
import { useSession } from "./store";

/**
 * Look up a model's capabilities by provider + model id, falling back to a
 * search across all providers when the provider id is unknown (e.g. a session
 * whose provider hasn't been backfilled yet).
 */
export function modelCapabilities(
  providerId: string | null | undefined,
  modelId: string | null | undefined,
): string[] {
  const settings = useSettings.getState().settings;
  if (!settings || !modelId) return [];
  const byProvider = settings.model_services
    ?.find((p) => p.id === providerId)
    ?.models?.find((m) => m.id === modelId);
  if (byProvider) return withImpliedReasoning(modelId, byProvider.capabilities ?? []);
  for (const p of settings.model_services ?? []) {
    const m = p.models?.find((mm) => mm.id === modelId);
    if (m) return withImpliedReasoning(modelId, m.capabilities ?? []);
  }
  return [];
}

function withImpliedReasoning(modelId: string, caps: string[]): string[] {
  if (caps.includes("reasoning") || !geminiIdImpliesReasoning(modelId)) {
    return caps;
  }
  return [...caps, "reasoning"];
}

function geminiIdImpliesReasoning(modelId: string): boolean {
  const id = modelId.toLowerCase();
  return (
    !id.includes("image") &&
    !id.includes("imagen") &&
    !id.includes("veo") &&
    (id.includes("gemini-2.5") || id.includes("gemini-3"))
  );
}

/**
 * Capabilities of the currently active session's model (its own provider +
 * model), falling back to the global default when no session is active.
 */
export function activeModelCapabilities(): string[] {
  const settings = useSettings.getState().settings;
  if (!settings) return [];
  const active = useSession.getState().active;
  const providerId = active?.session.provider_id ?? settings.active_provider_id;
  const modelId = active?.session.model ?? settings.model;
  return modelCapabilities(providerId, modelId);
}
