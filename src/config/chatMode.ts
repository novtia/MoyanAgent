/** Composer / session mode → persisted `sessions.agent_type`. */
export type ComposerChatMode = "agent" | "anchored" | "plan" | "chat" | "director";

export const SESSION_AGENT_GENERAL = "general-purpose";
export const SESSION_AGENT_PLAN = "Plan";
/** Default main-session mode: normal chat (AskUser + web tools only). */
export const SESSION_AGENT_CHAT = "chat";
/** TRPG director: narrate + ConsultRoles (+ AskUser / RoleState). */
export const SESSION_AGENT_DIRECTOR = "trpg-director";
/** Agent capabilities, but the opening request advertises `Read` only. */
export const SESSION_AGENT_ANCHORED = "anchored";

export function agentTypeFromComposerMode(mode: ComposerChatMode): string {
  if (mode === "plan") return SESSION_AGENT_PLAN;
  if (mode === "agent") return SESSION_AGENT_GENERAL;
  if (mode === "anchored") return SESSION_AGENT_ANCHORED;
  if (mode === "director") return SESSION_AGENT_DIRECTOR;
  return SESSION_AGENT_CHAT;
}

export function composerModeFromAgentType(at: string | null | undefined): ComposerChatMode {
  if (at === SESSION_AGENT_PLAN) return "plan";
  if (at === SESSION_AGENT_GENERAL) return "agent";
  if (at === SESSION_AGENT_ANCHORED) return "anchored";
  if (at === SESSION_AGENT_DIRECTOR) return "director";
  return "chat";
}
