/** Composer / session mode → persisted `sessions.agent_type`. */
export type ComposerChatMode = "agent" | "plan" | "chat";

export const SESSION_AGENT_GENERAL = "general-purpose";
export const SESSION_AGENT_PLAN = "Plan";
/** Default main-session mode: normal chat (AskUser + web tools only). */
export const SESSION_AGENT_CHAT = "chat";

export function agentTypeFromComposerMode(mode: ComposerChatMode): string {
  if (mode === "plan") return SESSION_AGENT_PLAN;
  if (mode === "agent") return SESSION_AGENT_GENERAL;
  return SESSION_AGENT_CHAT;
}

export function composerModeFromAgentType(at: string | null | undefined): ComposerChatMode {
  if (at === SESSION_AGENT_PLAN) return "plan";
  if (at === SESSION_AGENT_GENERAL) return "agent";
  return "chat";
}
