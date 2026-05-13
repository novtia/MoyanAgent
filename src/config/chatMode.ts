/** Composer UI mode → persisted `sessions.agent_type`. */
export type ComposerChatMode = "agent" | "plan";

export const SESSION_AGENT_GENERAL = "general-purpose";
export const SESSION_AGENT_PLAN = "Plan";

export function agentTypeFromComposerMode(mode: ComposerChatMode): string {
  return mode === "plan" ? SESSION_AGENT_PLAN : SESSION_AGENT_GENERAL;
}

export function composerModeFromAgentType(at: string | null | undefined): ComposerChatMode {
  return at === SESSION_AGENT_PLAN ? "plan" : "agent";
}
