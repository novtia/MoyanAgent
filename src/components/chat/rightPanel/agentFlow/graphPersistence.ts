import type { ChainEntry } from "../../../../types";
import { MAIN } from "./constants";
import { buildGraphFromChain, mergeChainOverrides } from "./graphModel";
import type { Graph } from "./types";

export function graphKey(sessionId: string) {
  return `atelier:agent-flow:${sessionId}`;
}

export function loadGraph(sessionId: string, chain: ChainEntry[]): Graph {
  let graph: Graph | null = null;
  try {
    const raw = window.localStorage.getItem(graphKey(sessionId));
    if (raw) {
      const parsed = JSON.parse(raw) as Graph;
      if (Array.isArray(parsed.nodes) && Array.isArray(parsed.edges)) {
        if (parsed.nodes.some((n) => n.agentType === MAIN)) {
          graph = parsed;
        }
      }
    }
  } catch {
    /* ignore */
  }
  return mergeChainOverrides(graph ?? buildGraphFromChain(chain), chain);
}
