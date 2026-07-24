import type { ChainEntry, NodeOverrides } from "../../../../types";
import type { FNode } from "./types";

/** `true` when an override object actually carries at least one value. */
export function hasOverride(ov?: NodeOverrides | null): ov is NodeOverrides {
  return (
    !!ov &&
    (ov.system_prompt !== undefined ||
      ov.model !== undefined ||
      ov.tools !== undefined)
  );
}

export function entryType(e: ChainEntry): string {
  return typeof e === "string" ? e : e.agent_type;
}

export function entryOverrides(e: ChainEntry): NodeOverrides | undefined {
  return typeof e === "string" ? undefined : e.overrides;
}

/** Serialise a node back to a chain entry (bare string unless it has overrides). */
export function nodeToEntry(n: FNode): ChainEntry {
  return hasOverride(n.overrides) ? { agent_type: n.agentType, overrides: n.overrides } : n.agentType;
}

export function entryHasOverrides(e: ChainEntry): boolean {
  if (typeof e === "string") return false;
  return hasOverride(e.overrides);
}

export function chainEntryType(e: ChainEntry): string {
  return entryType(e);
}
