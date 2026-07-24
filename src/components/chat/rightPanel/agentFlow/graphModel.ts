import type { ChainEntry, NodeOverrides } from "../../../../types";
import { GAP_X, MAIN, NODE_W } from "./constants";
import {
  entryOverrides,
  entryType,
  hasOverride,
  nodeToEntry,
} from "./chainUtils";
import type { FEdge, FNode, Graph } from "./types";

let idCounter = 0;
export function genId(prefix: string) {
  idCounter += 1;
  return `${prefix}-${Date.now().toString(36)}-${idCounter}`;
}

export function normalizeChain(chain: ChainEntry[]): ChainEntry[] {
  if (chain.length === 0) return [MAIN];
  return chain.some((e) => entryType(e) === MAIN) ? chain : [MAIN, ...chain];
}

export function buildGraphFromChain(chain: ChainEntry[]): Graph {
  const order = normalizeChain(chain);
  const nodes: FNode[] = order.map((entry, i) => {
    const overrides = entryOverrides(entry);
    return {
      id: genId("n"),
      agentType: entryType(entry),
      x: 48 + i * (NODE_W + GAP_X),
      y: 96,
      ...(hasOverride(overrides) ? { overrides } : {}),
    };
  });
  const edges: FEdge[] = [];
  for (let i = 0; i < nodes.length - 1; i++) {
    edges.push({ id: genId("e"), from: nodes[i].id, to: nodes[i + 1].id });
  }
  return { nodes, edges };
}

/** Apply per-node overrides from the persisted chain onto canvas nodes. */
export function mergeChainOverrides(graph: Graph, chain: ChainEntry[]): Graph {
  if (chain.length === 0) return graph;
  const overridesByType = new Map<string, NodeOverrides>();
  for (const entry of normalizeChain(chain)) {
    if (typeof entry === "string") continue;
    if (hasOverride(entry.overrides)) {
      overridesByType.set(entry.agent_type, entry.overrides);
    }
  }
  if (overridesByType.size === 0) return graph;
  let changed = false;
  const nodes = graph.nodes.map((n) => {
    const ov = overridesByType.get(n.agentType);
    if (!ov || JSON.stringify(n.overrides) === JSON.stringify(ov)) return n;
    changed = true;
    return { ...n, overrides: ov };
  });
  return changed ? { ...graph, nodes } : graph;
}

/**
 * Derive the linear execution chain from a free-form graph.
 *
 * The canvas now allows arbitrary wiring plus free-floating (unconnected)
 * nodes, so the chain is defined as the single path that runs *through the
 * main agent*: backtrack along incoming edges to the head of main's chain,
 * then walk forward along outgoing edges. Nodes not wired into that path
 * (floating nodes or separate components) are intentionally excluded from the
 * executed order — they just live on the canvas.
 */
export function deriveOrder(graph: Graph): ChainEntry[] {
  const { nodes, edges } = graph;
  const main = nodes.find((n) => n.agentType === MAIN);
  if (!main) return [MAIN];

  const inBy = new Map<string, string>();
  const outBy = new Map<string, string>();
  for (const e of edges) {
    inBy.set(e.to, e.from);
    outBy.set(e.from, e.to);
  }

  // Backtrack to the head of the chain that contains main.
  let head = main.id;
  const seenBack = new Set<string>([head]);
  while (inBy.has(head)) {
    const prev = inBy.get(head)!;
    if (seenBack.has(prev)) break; // cycle guard
    seenBack.add(prev);
    head = prev;
  }

  // Walk forward from the head collecting chain entries.
  const order: ChainEntry[] = [];
  const seen = new Set<string>();
  let cur: string | undefined = head;
  while (cur && !seen.has(cur)) {
    seen.add(cur);
    const node = nodes.find((n) => n.id === cur);
    // Disabled nodes stay wired but are skipped in the executed chain.
    if (node && !node.disabled) order.push(nodeToEntry(node));
    cur = outBy.get(cur);
  }
  if (!order.some((e) => entryType(e) === MAIN)) order.push(MAIN);
  return order;
}

/** The last node reachable from main by following outgoing edges. */
export function chainTailId(g: Graph): string | null {
  const main = g.nodes.find((n) => n.agentType === MAIN);
  if (!main) return null;
  const outBy = new Map(g.edges.map((e) => [e.from, e.to] as const));
  let cur = main.id;
  const seen = new Set<string>([cur]);
  while (outBy.has(cur)) {
    const next = outBy.get(cur)!;
    if (seen.has(next)) break;
    seen.add(next);
    cur = next;
  }
  return cur;
}

export function removeNodeFromGraph(g: Graph, id: string): Graph {
  return {
    nodes: g.nodes.filter((n) => n.id !== id),
    edges: g.edges.filter((e) => e.from !== id && e.to !== id),
  };
}
