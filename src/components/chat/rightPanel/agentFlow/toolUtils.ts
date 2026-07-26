import type { NodeOverrides } from "../../../../types";
import { MAIN } from "./constants";

export function resolveDefTools(
  defToolsRaw: string[],
  defAll: boolean,
  allTools: string[],
  disallowed: string[] = [],
): string[] {
  const deny = new Set(disallowed);
  const base = defAll
    ? allTools.filter((tn) => !deny.has(tn))
    : defToolsRaw.filter((tn) => allTools.includes(tn) && !deny.has(tn));
  return base;
}

export function resolveSelectedTools(
  ov: NodeOverrides | undefined,
  defTools: string[],
  allTools: string[],
): string[] {
  // Node-override tool semantics:
  //   undefined → inherit the agent's default tool set
  //   ["*"]     → all tools
  //   []        → NO tools (empty allow-list)
  //   [names]   → exactly those tools
  if (ov?.tools === undefined) return defTools;
  if (ov.tools.includes("*")) return [...allTools];
  return ov.tools.filter((tn) => allTools.includes(tn));
}

export function resolveDefinitionAgentType(agentType: string, sessionAgentType: string): string {
  return agentType === MAIN ? sessionAgentType : agentType;
}

export function toolDescription(
  t: (key: string, opts?: { defaultValue?: string }) => string,
  name: string,
) {
  return t(`agentFlow.toolDescriptions.${name}`, { defaultValue: name });
}
