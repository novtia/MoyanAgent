import type { ChainEntry, NodeOverrides } from "../../../../types";
import { MAIN } from "./constants";

export interface FNode {
  id: string;
  agentType: string;
  x: number;
  y: number;
  /** Per-node config override for this chain position (not the global agent). */
  overrides?: NodeOverrides;
  /**
   * When `true` the node is kept on the canvas (and in the wiring) but skipped
   * when deriving the executed chain. The main node can never be disabled.
   */
  disabled?: boolean;
}

export interface FEdge {
  id: string;
  from: string;
  to: string;
}

export interface Graph {
  nodes: FNode[];
  edges: FEdge[];
}

export interface Agent {
  id: string;
  name: string;
  custom: boolean;
}

/**
 * How a freshly added node is wired into the graph:
 * - `append`       — chained after the main chain's tail node.
 * - `insert`       — spliced into an existing edge (`edgeId`).
 * - `insertBefore` — spliced in front of `targetId` (enables adding a node
 *   ahead of the main agent, which otherwise has no predecessor slot).
 * - `insertAfter`  — spliced right after `targetId`.
 * - `floating`     — dropped at (`wx`,`wy`) with no edges; the user wires it
 *   up manually (a node with both ends empty).
 */
export type AddMode = "append" | "insert" | "insertBefore" | "insertAfter" | "floating";

/** Info handed to the panel when the user opens the per-node config editor. */
export interface NodeConfigTarget {
  nodeId: string;
  agentType: string;
  overrides?: NodeOverrides;
}

/** Imperative handle the panel uses to write per-node overrides back. */
export interface AgentFlowCanvasHandle {
  applyNodeOverrides: (nodeId: string, overrides: NodeOverrides | null) => void;
  getNodeOverrides: (nodeId: string) => NodeOverrides | undefined;
}

export interface AgentFlowCanvasProps {
  open: boolean;
  sessionId: string | null;
  /**
   * Storage scope for the graph (layout + chain). Sessions in a project share
   * one flow, so this is the project id when in a project, otherwise the
   * session id. Falls back to `sessionId` when omitted.
   */
  scopeId?: string | null;
  chain: ChainEntry[];
  agents: Agent[];
  knownTypes: Set<string>;
  nameOf: (agentType: string) => string;
  onOrderChange: (order: ChainEntry[]) => void;
  onRequestNewAgent: () => void;
  onEditAgent: (agentType: string) => void;
  onDeleteAgent: (agentType: string) => void;
  /** Open the per-node config editor for a chain node. */
  onEditNodeConfig: (target: NodeConfigTarget) => void;
}

export interface FormState {
  mode: "closed" | "new" | "edit" | "edit-node";
  /** Chain canvas node id when mode is `edit-node`. */
  nodeId?: string;
  agentType: string;
  name: string;
  whenToUse: string;
  systemPrompt: string;
  model: string;
  tools: string[];
  loading: boolean;
}

export type FormSection = "basic" | "prompt" | "model" | "tools";

export { MAIN };
