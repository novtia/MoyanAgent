/** Composer / session mode → persisted `sessions.agent_type`. */
export type ComposerChatMode = "agent" | "plan" | "chat" | "director";

export const SESSION_AGENT_GENERAL = "general-purpose";
export const SESSION_AGENT_PLAN = "Plan";
/** Default main-session mode: normal chat (AskUser + web tools only). */
export const SESSION_AGENT_CHAT = "chat";
/** TRPG director: narrate + ConsultRoles (+ AskUser / RoleState). */
export const SESSION_AGENT_DIRECTOR = "trpg-director";

export function agentTypeFromComposerMode(mode: ComposerChatMode): string {
  if (mode === "plan") return SESSION_AGENT_PLAN;
  if (mode === "agent") return SESSION_AGENT_GENERAL;
  if (mode === "director") return SESSION_AGENT_DIRECTOR;
  return SESSION_AGENT_CHAT;
}

export function composerModeFromAgentType(at: string | null | undefined): ComposerChatMode {
  if (at === SESSION_AGENT_PLAN) return "plan";
  if (at === SESSION_AGENT_GENERAL) return "agent";
  if (at === SESSION_AGENT_DIRECTOR) return "director";
  return "chat";
}
