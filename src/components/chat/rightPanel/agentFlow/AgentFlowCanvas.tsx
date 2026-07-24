import {
  forwardRef,
  useCallback,
  useEffect,
  useImperativeHandle,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { useTranslation } from "react-i18next";
import { openContextMenu, type ContextMenuItem } from "../../../context-menu";
import type { ChainEntry, NodeOverrides } from "../../../../types";
import { CUSTOM_PREFIX, GAP_X, MAIN, NODE_H, NODE_W, ZOOM_MAX, ZOOM_MIN } from "./constants";
import {
  entryType,
  hasOverride,
  nodeToEntry,
} from "./chainUtils";
import {
  buildGraphFromChain,
  chainTailId,
  deriveOrder,
  genId,
  mergeChainOverrides,
  normalizeChain,
  removeNodeFromGraph,
} from "./graphModel";
import { graphKey, loadGraph } from "./graphPersistence";
import type {
  AddMode,
  Agent,
  AgentFlowCanvasHandle,
  AgentFlowCanvasProps,
  FEdge,
  FNode,
  Graph,
  NodeConfigTarget,
} from "./types";

export { MAIN } from "./constants";
export type { AgentFlowCanvasHandle, NodeConfigTarget } from "./types";

export const AgentFlowCanvas = forwardRef<AgentFlowCanvasHandle, AgentFlowCanvasProps>(
  function AgentFlowCanvas(
    {
      open,
      sessionId,
      scopeId,
      chain,
      agents,
      knownTypes,
      nameOf,
      onOrderChange,
      onRequestNewAgent,
      onEditAgent,
      onDeleteAgent,
      onEditNodeConfig,
    }: AgentFlowCanvasProps,
    ref,
  ) {
  const { t } = useTranslation();
  const containerRef = useRef<HTMLDivElement | null>(null);

  // Graph persistence + (re)initialisation is keyed by the flow scope (project
  // for project sessions, otherwise the session) so conversations under the
  // same project share a single flow instead of each caching its own.
  const storageScope = scopeId ?? sessionId;

  const [graph, setGraph] = useState<Graph>({ nodes: [], edges: [] });
  const [pan, setPan] = useState({ x: 0, y: 0 });
  const [zoom, setZoom] = useState(1);
  const [viewport, setViewport] = useState({ w: 0, h: 0 });
  const [needsFit, setNeedsFit] = useState(false);
  const [connecting, setConnecting] = useState<{
    fixedId: string;
    dir: "out" | "in";
    wx: number;
    wy: number;
  } | null>(null);
  // Node currently highlighted as the drop target while wiring a connection.
  const [linkTarget, setLinkTarget] = useState<string | null>(null);
  // Picker shown when a connection is dropped on empty canvas: lets the user
  // wire to an existing node or add a new agent at the drop point (`wx`/`wy`
  // are the drop position in world coordinates).
  const [connectMenu, setConnectMenu] = useState<{
    x: number;
    y: number;
    wx: number;
    wy: number;
    fixedId: string;
    dir: "out" | "in";
  } | null>(null);
  // Edge selected by a left-click; shows a delete affordance and is removable
  // with the Delete / Backspace key.
  const [selectedEdge, setSelectedEdge] = useState<string | null>(null);
  const [addMenu, setAddMenu] = useState<
    {
      x: number;
      y: number;
      mode: AddMode;
      edgeId?: string;
      targetId?: string;
      wx?: number;
      wy?: number;
    } | null
  >(null);
  const [nodeMenu, setNodeMenu] = useState<{ x: number; y: number; nodeId: string } | null>(null);

  const panRef = useRef(pan);
  const zoomRef = useRef(zoom);
  const graphRef = useRef(graph);
  panRef.current = pan;
  zoomRef.current = zoom;
  graphRef.current = graph;

  // Let the panel write per-node overrides back into the graph; a `null`
  // override clears the customisation for that node.
  useImperativeHandle(
    ref,
    () => ({
      applyNodeOverrides: (nodeId: string, overrides: NodeOverrides | null) => {
        setGraph((g) => ({
          ...g,
          nodes: g.nodes.map((n) =>
            n.id === nodeId
              ? { ...n, overrides: hasOverride(overrides) ? overrides : undefined }
              : n,
          ),
        }));
      },
      getNodeOverrides: (nodeId: string) =>
        graphRef.current.nodes.find((n) => n.id === nodeId)?.overrides,
    }),
    [],
  );

  const initedFor = useRef<string | null>(null);
  const lastPushedOrder = useRef<string>("");

  const toWorld = useCallback((clientX: number, clientY: number) => {
    const rect = containerRef.current?.getBoundingClientRect();
    const px = rect?.left ?? 0;
    const py = rect?.top ?? 0;
    return {
      x: (clientX - px - panRef.current.x) / zoomRef.current,
      y: (clientY - py - panRef.current.y) / zoomRef.current,
    };
  }, []);

  const fitView = useCallback((g: Graph) => {
    const el = containerRef.current;
    if (!el || g.nodes.length === 0) return;
    const minX = Math.min(...g.nodes.map((n) => n.x));
    const minY = Math.min(...g.nodes.map((n) => n.y));
    const maxX = Math.max(...g.nodes.map((n) => n.x + NODE_W));
    const maxY = Math.max(...g.nodes.map((n) => n.y + NODE_H));
    const w = el.clientWidth;
    const h = el.clientHeight;
    const pad = 40;
    const z = Math.max(
      ZOOM_MIN,
      Math.min(ZOOM_MAX, Math.min((w - pad * 2) / (maxX - minX || 1), (h - pad * 2) / (maxY - minY || 1), 1)),
    );
    setZoom(z);
    setPan({ x: (w - (maxX + minX) * z) / 2, y: (h - (maxY + minY) * z) / 2 });
  }, []);

  const chainKey = useMemo(() => JSON.stringify(chain), [chain]);

  // ── init graph per session ──────────────────────────────────────────────
  // The main node is always present (even without an active session) so the
  // canvas is never empty; persistence is gated on sessionId elsewhere.
  useEffect(() => {
    const key = storageScope ?? "__none__";
    if (initedFor.current === key) return;
    initedFor.current = key;
    const g = storageScope ? loadGraph(storageScope, chain) : buildGraphFromChain(chain);
    setGraph(g);
    lastPushedOrder.current = JSON.stringify(deriveOrder(g));
    setNeedsFit(true);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [storageScope]);

  // Keep per-node overrides in sync when agent_chain updates (e.g. after save).
  useEffect(() => {
    if (!initedFor.current || initedFor.current === "__none__") return;
    setGraph((g) => mergeChainOverrides(g, chain));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [chainKey]);

  // ── prune nodes referencing deleted custom agents ───────────────────────
  useEffect(() => {
    if (knownTypes.size === 0) return;
    setGraph((g) => {
      const invalid = g.nodes.filter(
        (n) =>
          n.agentType !== MAIN &&
          n.agentType.startsWith(CUSTOM_PREFIX) &&
          !knownTypes.has(n.agentType),
      );
      if (invalid.length === 0) return g;
      let next = g;
      for (const n of invalid) next = removeNodeFromGraph(next, n.id);
      return next;
    });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [knownTypes]);

  // ── persist graph + derive order ─────────────────────────────────────────
  useEffect(() => {
    if (!sessionId || !storageScope) return;
    try {
      window.localStorage.setItem(graphKey(storageScope), JSON.stringify(graph));
    } catch {
      /* ignore */
    }
    const order = deriveOrder(graph);
    const serialized = JSON.stringify(order);
    if (serialized !== lastPushedOrder.current) {
      lastPushedOrder.current = serialized;
      onOrderChange(order);
    }
  }, [graph, sessionId, storageScope, onOrderChange]);

  // ── viewport measurement ─────────────────────────────────────────────────
  useLayoutEffect(() => {
    const el = containerRef.current;
    if (!el) return;
    const measure = () => setViewport({ w: el.clientWidth, h: el.clientHeight });
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  // ── fit view once the viewport has a real size ───────────────────────────
  useEffect(() => {
    if (!needsFit) return;
    if (viewport.w === 0 || viewport.h === 0 || graph.nodes.length === 0) return;
    fitView(graph);
    setNeedsFit(false);
  }, [needsFit, viewport, graph, fitView]);

  // ── background pan ───────────────────────────────────────────────────────
  const onBgPointerDown = (e: React.PointerEvent) => {
    if (e.button !== 0) return;
    closeMenus();
    setSelectedEdge(null);
    const start = { x: e.clientX, y: e.clientY };
    const startPan = { ...panRef.current };
    const onMove = (ev: PointerEvent) => {
      setPan({ x: startPan.x + (ev.clientX - start.x), y: startPan.y + (ev.clientY - start.y) });
    };
    const onUp = () => {
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", onUp);
    };
    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onUp);
  };

  useEffect(() => {
    const el = containerRef.current;
    if (!el) return;
    const onWheel = (e: WheelEvent) => {
      e.preventDefault();
      const rect = el.getBoundingClientRect();
      const cx = e.clientX - rect.left;
      const cy = e.clientY - rect.top;
      const delta = -e.deltaY * 0.0015;
      const nz = Math.max(ZOOM_MIN, Math.min(ZOOM_MAX, zoomRef.current * (1 + delta)));
      const ratio = nz / zoomRef.current;
      setPan({
        x: cx - (cx - panRef.current.x) * ratio,
        y: cy - (cy - panRef.current.y) * ratio,
      });
      setZoom(nz);
    };
    el.addEventListener("wheel", onWheel, { passive: false });
    return () => el.removeEventListener("wheel", onWheel);
  }, []);

  // ── delete the selected edge with Delete / Backspace ─────────────────────
  useEffect(() => {
    if (!selectedEdge) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Delete" && e.key !== "Backspace") return;
      const ae = document.activeElement as HTMLElement | null;
      if (
        ae &&
        (ae.tagName === "INPUT" || ae.tagName === "TEXTAREA" || ae.isContentEditable)
      ) {
        return;
      }
      e.preventDefault();
      deleteEdge(selectedEdge);
      setSelectedEdge(null);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [selectedEdge]);

  // ── node drag ────────────────────────────────────────────────────────────
  const onNodePointerDown = (e: React.PointerEvent, nodeId: string) => {
    if (e.button !== 0) return;
    e.stopPropagation();
    closeMenus();
    const start = { x: e.clientX, y: e.clientY };
    const node = graphRef.current.nodes.find((n) => n.id === nodeId);
    if (!node) return;
    const origin = { x: node.x, y: node.y };
    let moved = false;
    const onMove = (ev: PointerEvent) => {
      const dx = (ev.clientX - start.x) / zoomRef.current;
      const dy = (ev.clientY - start.y) / zoomRef.current;
      if (Math.abs(dx) > 1 || Math.abs(dy) > 1) moved = true;
      setGraph((g) => ({
        ...g,
        nodes: g.nodes.map((n) =>
          n.id === nodeId ? { ...n, x: origin.x + dx, y: origin.y + dy } : n,
        ),
      }));
    };
    const onUp = () => {
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", onUp);
      void moved;
    };
    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onUp);
  };

  // ── connect (drag from a port) ───────────────────────────────────────────
  // `dir: "out"` drags forward from `fixedId`'s output toward a target's input
  // (`fixedId -> target`). `dir: "in"` drags backward from `fixedId`'s input
  // toward a source's output (`source -> fixedId`).
  const wouldCycle = (g: Graph, from: string, to: string) => {
    if (from === to) return true;
    const outBy = new Map(g.edges.map((e) => [e.from, e.to] as const));
    let cur: string | undefined = to;
    const guard = new Set<string>();
    while (cur && !guard.has(cur)) {
      if (cur === from) return true;
      guard.add(cur);
      cur = outBy.get(cur);
    }
    return false;
  };

  // The node whose (port-inclusive) hit box is under the cursor and to which a
  // valid, non-cycling connection can be made. Returns `null` otherwise.
  const findLinkTarget = (wx: number, wy: number, fixedId: string, dir: "out" | "in") => {
    const g = graphRef.current;
    const M = 16; // expand the hit box so dropping near the port still connects
    const hit = g.nodes.find(
      (n) =>
        n.id !== fixedId &&
        wx >= n.x - M &&
        wx <= n.x + NODE_W + M &&
        wy >= n.y - M &&
        wy <= n.y + NODE_H + M,
    );
    if (!hit) return null;
    const from = dir === "out" ? fixedId : hit.id;
    const to = dir === "out" ? hit.id : fixedId;
    if (wouldCycle(g, from, to)) return null;
    return hit.id;
  };

  const beginConnect = (e: React.PointerEvent, fixedId: string, dir: "out" | "in") => {
    if (e.button !== 0) return;
    e.stopPropagation();
    closeMenus();
    setSelectedEdge(null);
    const w0 = toWorld(e.clientX, e.clientY);
    setConnecting({ fixedId, dir, wx: w0.x, wy: w0.y });
    setLinkTarget(null);
    const onMove = (ev: PointerEvent) => {
      const w = toWorld(ev.clientX, ev.clientY);
      setConnecting((c) => (c ? { ...c, wx: w.x, wy: w.y } : c));
      setLinkTarget(findLinkTarget(w.x, w.y, fixedId, dir));
    };
    const onUp = (ev: PointerEvent) => {
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", onUp);
      const w = toWorld(ev.clientX, ev.clientY);
      const cand = findLinkTarget(w.x, w.y, fixedId, dir);
      if (cand) {
        // Dropped directly onto a node: wire it immediately.
        if (dir === "out") connect(fixedId, cand);
        else connect(cand, fixedId);
      } else {
        // Dropped on empty canvas: let the user pick a node to connect to.
        // Always shown — when no valid target exists (e.g. every other node
        // would form a cycle) the menu displays an explanatory empty state.
        const host = containerRef.current?.getBoundingClientRect();
        setConnectMenu({
          x: ev.clientX - (host?.left ?? 0),
          y: ev.clientY - (host?.top ?? 0),
          wx: w.x,
          wy: w.y,
          fixedId,
          dir,
        });
      }
      setConnecting(null);
      setLinkTarget(null);
    };
    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onUp);
  };

  // Nodes that can validly receive a connection from `fixedId` in `dir`
  // (excludes self and any target that would introduce a cycle).
  const validConnectTargets = (fixedId: string, dir: "out" | "in") => {
    const g = graphRef.current;
    return g.nodes.filter((n) => {
      if (n.id === fixedId) return false;
      const from = dir === "out" ? fixedId : n.id;
      const to = dir === "out" ? n.id : fixedId;
      return !wouldCycle(g, from, to);
    });
  };

  // Commit a connection chosen from the picker menu.
  const connectFromMenu = (targetId: string) => {
    if (!connectMenu) return;
    if (connectMenu.dir === "out") connect(connectMenu.fixedId, targetId);
    else connect(targetId, connectMenu.fixedId);
    setConnectMenu(null);
  };

  // Create a new agent node at the drop point and wire it in the drag
  // direction (out: fixed -> new, in: new -> fixed). Mirrors `connect`'s
  // replacement semantics so each side keeps at most one edge per direction.
  const addAndConnectFromMenu = (agentType: string) => {
    if (!connectMenu) return;
    const { fixedId, dir, wx, wy } = connectMenu;
    const node: FNode = {
      id: genId("n"),
      agentType,
      x: wx - NODE_W / 2,
      y: wy - NODE_H / 2,
    };
    setGraph((g) => {
      const from = dir === "out" ? fixedId : node.id;
      const to = dir === "out" ? node.id : fixedId;
      const edges = g.edges.filter((e) => e.from !== from && e.to !== to);
      edges.push({ id: genId("e"), from, to });
      return { nodes: [...g.nodes, node], edges };
    });
    setConnectMenu(null);
  };

  // Dragging an input port detaches its existing incoming edge (if any) and
  // re-grabs the dangling end so the user can rewire or drop it to disconnect.
  // With no incoming edge it starts a backward connection into this node.
  const onInPortPointerDown = (e: React.PointerEvent, nodeId: string) => {
    if (e.button !== 0) return;
    const inEdge = graphRef.current.edges.find((ed) => ed.to === nodeId);
    if (inEdge) {
      deleteEdge(inEdge.id);
      beginConnect(e, inEdge.from, "out");
    } else {
      beginConnect(e, nodeId, "in");
    }
  };

  const connect = (from: string, to: string) => {
    setGraph((g) => {
      if (wouldCycle(g, from, to)) return g;
      const edges = g.edges.filter((e) => e.from !== from && e.to !== to);
      edges.push({ id: genId("e"), from, to });
      return { ...g, edges };
    });
  };

  const addNode = (
    agentType: string,
    ctx: { mode: AddMode; edgeId?: string; targetId?: string; wx?: number; wy?: number },
  ) => {
    setGraph((g) => {
      const node: FNode = { id: genId("n"), agentType, x: 0, y: 0 };
      if (ctx.mode === "floating") {
        // A standalone node with no edges; the user wires it manually.
        node.x = ctx.wx ?? 48;
        node.y = ctx.wy ?? 96;
        return { nodes: [...g.nodes, node], edges: g.edges };
      }
      if (ctx.mode === "insert" && ctx.edgeId) {
        const edge = g.edges.find((e) => e.id === ctx.edgeId);
        if (edge) {
          const a = g.nodes.find((n) => n.id === edge.from);
          const b = g.nodes.find((n) => n.id === edge.to);
          node.x = a && b ? (a.x + b.x) / 2 : a ? a.x + NODE_W + GAP_X : 48;
          node.y = a && b ? (a.y + b.y) / 2 : a ? a.y : 96;
          const edges = g.edges.filter((e) => e.id !== ctx.edgeId);
          edges.push({ id: genId("e"), from: edge.from, to: node.id });
          edges.push({ id: genId("e"), from: node.id, to: edge.to });
          return { nodes: [...g.nodes, node], edges };
        }
      }
      if (ctx.mode === "insertBefore" && ctx.targetId) {
        const target = g.nodes.find((n) => n.id === ctx.targetId);
        if (target) {
          node.x = target.x - NODE_W - GAP_X;
          node.y = target.y;
          // Splice ahead of `target`, redirecting its existing predecessor (if
          // any) through the new node so the chain stays linear.
          const inEdge = g.edges.find((e) => e.to === target.id);
          const edges = g.edges.filter((e) => e.id !== inEdge?.id);
          if (inEdge) edges.push({ id: genId("e"), from: inEdge.from, to: node.id });
          edges.push({ id: genId("e"), from: node.id, to: target.id });
          return { nodes: [...g.nodes, node], edges };
        }
      }
      if (ctx.mode === "insertAfter" && ctx.targetId) {
        const target = g.nodes.find((n) => n.id === ctx.targetId);
        if (target) {
          node.x = target.x + NODE_W + GAP_X;
          node.y = target.y;
          const outEdge = g.edges.find((e) => e.from === target.id);
          const edges = g.edges.filter((e) => e.id !== outEdge?.id);
          edges.push({ id: genId("e"), from: target.id, to: node.id });
          if (outEdge) edges.push({ id: genId("e"), from: node.id, to: outEdge.to });
          return { nodes: [...g.nodes, node], edges };
        }
      }
      // append: chain after the main chain's tail (ignores floating nodes)
      const tailId = chainTailId(g);
      const tail = tailId ? g.nodes.find((n) => n.id === tailId) : undefined;
      if (tail) {
        node.x = tail.x + NODE_W + GAP_X;
        node.y = tail.y;
      } else {
        node.x = 48;
        node.y = 96;
      }
      const edges = [...g.edges];
      if (tail) edges.push({ id: genId("e"), from: tail.id, to: node.id });
      return { nodes: [...g.nodes, node], edges };
    });
    setNeedsFit(true);
  };

  const deleteEdge = (edgeId: string) => {
    setGraph((g) => ({ ...g, edges: g.edges.filter((e) => e.id !== edgeId) }));
  };

  function removeNodeFromGraph(g: Graph, nodeId: string): Graph {
    const inEdge = g.edges.find((e) => e.to === nodeId);
    const outEdge = g.edges.find((e) => e.from === nodeId);
    let edges = g.edges.filter((e) => e.from !== nodeId && e.to !== nodeId);
    // Reconnect the neighbors so the chain stays continuous.
    if (inEdge && outEdge) {
      edges = edges.filter((e) => !(e.from === inEdge.from && e.to === outEdge.to));
      edges.push({ id: genId("e"), from: inEdge.from, to: outEdge.to });
    }
    return { nodes: g.nodes.filter((n) => n.id !== nodeId), edges };
  }

  const removeNode = (nodeId: string) => {
    const n = graphRef.current.nodes.find((x) => x.id === nodeId);
    if (!n || n.agentType === MAIN) return;
    setGraph((g) => removeNodeFromGraph(g, nodeId));
  };

  // Toggle a node's enabled/disabled state. The main node is always enabled.
  const toggleNodeDisabled = (nodeId: string) => {
    setGraph((g) => ({
      ...g,
      nodes: g.nodes.map((n) =>
        n.id === nodeId && n.agentType !== MAIN ? { ...n, disabled: !n.disabled } : n,
      ),
    }));
  };

  const closeMenus = () => {
    setAddMenu(null);
    setNodeMenu(null);
    setConnectMenu(null);
  };

  // Translate a client point into afc-viewport-relative coords so the reused
  // `AddMenu` picker (rendered inside the viewport) lands under the cursor.
  const hostPoint = (clientX: number, clientY: number) => {
    const host = containerRef.current?.getBoundingClientRect();
    return { x: clientX - (host?.left ?? 0), y: clientY - (host?.top ?? 0) };
  };

  // ── right-click context menu: empty canvas ───────────────────────────────
  const onCanvasContextMenu = (e: React.MouseEvent) => {
    const p = hostPoint(e.clientX, e.clientY);
    const w = toWorld(e.clientX, e.clientY);
    closeMenus();
    const items: ContextMenuItem[] = [
      {
        id: "add",
        label: t("agentFlow.addAgentHere"),
        disabled: !sessionId,
        onSelect: () => {
          setNodeMenu(null);
          setAddMenu({ x: p.x, y: p.y, mode: "floating", wx: w.x, wy: w.y });
        },
      },
      {
        id: "append",
        label: t("agentFlow.appendAgent"),
        disabled: !sessionId,
        onSelect: () => {
          setNodeMenu(null);
          setAddMenu({ x: p.x, y: p.y, mode: "append" });
        },
      },
      {
        id: "new",
        label: t("agentFlow.newAgent"),
        onSelect: () => onRequestNewAgent(),
      },
      { type: "separator" },
      {
        id: "fit",
        label: t("agentFlow.fitView"),
        onSelect: () => fitView(graphRef.current),
      },
    ];
    openContextMenu(e, items, { menuId: "agent-flow-canvas" });
  };

  // ── right-click context menu: a connection ───────────────────────────────
  const onEdgeContextMenu = (e: React.MouseEvent, edgeId: string) => {
    const p = hostPoint(e.clientX, e.clientY);
    closeMenus();
    const items: ContextMenuItem[] = [
      {
        id: "insert",
        label: t("agentFlow.insertHere"),
        onSelect: () => {
          setNodeMenu(null);
          setAddMenu({ x: p.x, y: p.y, mode: "insert", edgeId });
        },
      },
      { type: "separator" },
      {
        id: "delete-edge",
        label: t("agentFlow.deleteEdge"),
        danger: true,
        onSelect: () => deleteEdge(edgeId),
      },
    ];
    openContextMenu(e, items, { menuId: "agent-flow-edge" });
  };

  // ── right-click context menu: a node ─────────────────────────────────────
  const onNodeContextMenu = (e: React.MouseEvent, node: FNode) => {
    const p = hostPoint(e.clientX, e.clientY);
    const isMain = node.agentType === MAIN;
    const isCustom = node.agentType.startsWith(CUSTOM_PREFIX);
    closeMenus();
    const items: ContextMenuItem[] = [
      {
        id: "insert-before",
        label: t("agentFlow.insertBefore"),
        onSelect: () => {
          setNodeMenu(null);
          setAddMenu({ x: p.x, y: p.y, mode: "insertBefore", targetId: node.id });
        },
      },
      {
        id: "insert-after",
        label: t("agentFlow.insertAfter"),
        onSelect: () => {
          setNodeMenu(null);
          setAddMenu({ x: p.x, y: p.y, mode: "insertAfter", targetId: node.id });
        },
      },
    ];
    items.push(
      { type: "separator" },
      {
        id: "edit-node-config",
        label: t("agentFlow.editAgent"),
        onSelect: () =>
          onEditNodeConfig({
            nodeId: node.id,
            agentType: node.agentType,
            overrides: node.overrides,
          }),
      },
    );
    if (isCustom) {
      items.push({
        id: "edit-global",
        label: t("agentFlow.editAgentGlobal"),
        onSelect: () => onEditAgent(node.agentType),
      });
    }
    if (!isMain) {
      items.push({
        id: "toggle-disabled",
        label: node.disabled ? t("agentFlow.enableNode") : t("agentFlow.disableNode"),
        onSelect: () => toggleNodeDisabled(node.id),
      });
    }
    if (!isMain) {
      items.push({
        id: "remove",
        label: t("agentFlow.removeNode"),
        onSelect: () => removeNode(node.id),
      });
    }
    if (isCustom) {
      items.push({
        id: "delete",
        label: t("agentFlow.deleteAgent"),
        danger: true,
        onSelect: () => onDeleteAgent(node.agentType),
      });
    }
    openContextMenu(e, items, { menuId: "agent-flow-node" });
  };

  // ── render geometry ──────────────────────────────────────────────────────
  const nodeById = useMemo(() => {
    const m = new Map<string, FNode>();
    for (const n of graph.nodes) m.set(n.id, n);
    return m;
  }, [graph.nodes]);

  const edgePaths = useMemo(() => {
    return graph.edges
      .map((e) => {
        const a = nodeById.get(e.from);
        const b = nodeById.get(e.to);
        if (!a || !b) return null;
        const x1 = a.x + NODE_W;
        const y1 = a.y + NODE_H / 2;
        const x2 = b.x;
        const y2 = b.y + NODE_H / 2;
        const dx = Math.max(40, Math.abs(x2 - x1) / 2);
        const d = `M ${x1} ${y1} C ${x1 + dx} ${y1}, ${x2 - dx} ${y2}, ${x2} ${y2}`;
        return { id: e.id, d, mx: (x1 + x2) / 2, my: (y1 + y2) / 2 };
      })
      .filter(Boolean) as { id: string; d: string; mx: number; my: number }[];
  }, [graph.edges, nodeById]);

  const minimap = useMemo(() => {
    if (graph.nodes.length === 0 || viewport.w === 0) return null;
    const MW = 132;
    const MH = 88;
    const minX = Math.min(...graph.nodes.map((n) => n.x)) - 30;
    const minY = Math.min(...graph.nodes.map((n) => n.y)) - 30;
    const maxX = Math.max(...graph.nodes.map((n) => n.x + NODE_W)) + 30;
    const maxY = Math.max(...graph.nodes.map((n) => n.y + NODE_H)) + 30;
    const worldW = maxX - minX || 1;
    const worldH = maxY - minY || 1;
    const s = Math.min(MW / worldW, MH / worldH);
    // visible world rect
    const vx = (-pan.x) / zoom;
    const vy = (-pan.y) / zoom;
    const vw = viewport.w / zoom;
    const vh = viewport.h / zoom;
    return {
      MW,
      MH,
      s,
      minX,
      minY,
      nodes: graph.nodes,
      view: { x: (vx - minX) * s, y: (vy - minY) * s, w: vw * s, h: vh * s },
    };
  }, [graph.nodes, viewport, pan, zoom]);

  const onMinimapClick = (e: React.MouseEvent) => {
    if (!minimap) return;
    const rect = (e.currentTarget as HTMLElement).getBoundingClientRect();
    const mx = e.clientX - rect.left;
    const my = e.clientY - rect.top;
    const worldX = mx / minimap.s + minimap.minX;
    const worldY = my / minimap.s + minimap.minY;
    setPan({ x: viewport.w / 2 - worldX * zoom, y: viewport.h / 2 - worldY * zoom });
  };

  const tab = open ? 0 : -1;

  return (
    <div className="afc">
      <div className="afc-toolbar">
        <button
          type="button"
          className="afc-tool-btn"
          tabIndex={tab}
          disabled={!sessionId}
          onClick={(e) => {
            const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
            const host = containerRef.current?.getBoundingClientRect();
            setNodeMenu(null);
            setAddMenu({
              x: r.left - (host?.left ?? 0),
              y: r.bottom - (host?.top ?? 0) + 4,
              mode: "append",
            });
          }}
        >
          <PlusIcon /> {t("agentFlow.addAgent")}
        </button>
        <button type="button" className="afc-tool-btn" tabIndex={tab} onClick={onRequestNewAgent}>
          {t("agentFlow.newAgent")}
        </button>
        <span className="afc-toolbar-spacer" />
        <button
          type="button"
          className="afc-tool-btn icon"
          title={t("agentFlow.fitView")}
          tabIndex={tab}
          onClick={() => fitView(graph)}
        >
          <FitIcon />
        </button>
      </div>

      <div
        ref={containerRef}
        className="afc-viewport"
        onPointerDown={onBgPointerDown}
        onContextMenu={onCanvasContextMenu}
      >
        <div
          className="afc-world"
          style={{ transform: `translate(${pan.x}px, ${pan.y}px) scale(${zoom})` }}
        >
          <svg className="afc-edges" style={{ position: "absolute", overflow: "visible" }}>
            <defs>
              <marker
                id="afc-arrow"
                viewBox="0 0 10 10"
                refX="9"
                refY="5"
                markerWidth="7"
                markerHeight="7"
                orient="auto-start-reverse"
              >
                <path d="M0 0 L10 5 L0 10 z" fill="var(--line-strong)" />
              </marker>
            </defs>
            {edgePaths.map((e) => (
              <g key={e.id}>
                <path
                  d={e.d}
                  fill="none"
                  stroke={selectedEdge === e.id ? "var(--blue-500)" : "var(--line-strong)"}
                  strokeWidth={selectedEdge === e.id ? 2.5 : 2}
                  markerEnd="url(#afc-arrow)"
                />
                {/* Wide transparent hit area: re-enable pointer events (the
                    parent SVG sets pointer-events:none) so edges are
                    clickable (select / delete) and right-clickable. */}
                <path
                  d={e.d}
                  fill="none"
                  stroke="transparent"
                  strokeWidth={16}
                  style={{ pointerEvents: "stroke", cursor: "pointer" }}
                  onPointerDown={(ev) => ev.stopPropagation()}
                  onClick={(ev) => {
                    ev.stopPropagation();
                    closeMenus();
                    setSelectedEdge(e.id);
                  }}
                  onContextMenu={(ev) => onEdgeContextMenu(ev, e.id)}
                />
              </g>
            ))}
            {connecting &&
              (() => {
                const a = nodeById.get(connecting.fixedId);
                if (!a) return null;
                let x1: number;
                let y1: number;
                let x2: number;
                let y2: number;
                if (connecting.dir === "out") {
                  x1 = a.x + NODE_W;
                  y1 = a.y + NODE_H / 2;
                  x2 = connecting.wx;
                  y2 = connecting.wy;
                } else {
                  x1 = connecting.wx;
                  y1 = connecting.wy;
                  x2 = a.x;
                  y2 = a.y + NODE_H / 2;
                }
                const dx = Math.max(40, Math.abs(x2 - x1) / 2);
                return (
                  <path
                    d={`M ${x1} ${y1} C ${x1 + dx} ${y1}, ${x2 - dx} ${y2}, ${x2} ${y2}`}
                    fill="none"
                    stroke="var(--blue-500)"
                    strokeWidth={2}
                    strokeDasharray="4 3"
                  />
                );
              })()}
          </svg>

          {edgePaths.map((e) => (
            <button
              key={`ins-${e.id}`}
              type="button"
              className="afc-insert"
              style={{ left: e.mx - 11, top: e.my - 11 }}
              title={t("agentFlow.insertHere")}
              tabIndex={tab}
              onPointerDown={(ev) => ev.stopPropagation()}
              onContextMenu={(ev) => onEdgeContextMenu(ev, e.id)}
              onClick={(ev) => {
                ev.stopPropagation();
                const host = containerRef.current?.getBoundingClientRect();
                setNodeMenu(null);
                setAddMenu({
                  x: ev.clientX - (host?.left ?? 0),
                  y: ev.clientY - (host?.top ?? 0),
                  mode: "insert",
                  edgeId: e.id,
                });
              }}
            >
              +
            </button>
          ))}

          {selectedEdge &&
            (() => {
              const e = edgePaths.find((p) => p.id === selectedEdge);
              if (!e) return null;
              return (
                <button
                  type="button"
                  className="afc-edge-delete"
                  style={{ left: e.mx - 11, top: e.my + 16 }}
                  title={t("agentFlow.deleteEdge")}
                  tabIndex={tab}
                  onPointerDown={(ev) => ev.stopPropagation()}
                  onClick={(ev) => {
                    ev.stopPropagation();
                    deleteEdge(selectedEdge);
                    setSelectedEdge(null);
                  }}
                >
                  ×
                </button>
              );
            })()}

          {graph.nodes.map((n) => {
            const isMain = n.agentType === MAIN;
            const isCustom = n.agentType.startsWith(CUSTOM_PREFIX);
            return (
              <div
                key={n.id}
                className={`afc-node ${isMain ? "is-main" : ""} ${
                  linkTarget === n.id ? "is-link-target" : ""
                } ${n.disabled ? "is-disabled" : ""}`}
                style={{ left: n.x, top: n.y, width: NODE_W, height: NODE_H }}
                onPointerDown={(e) => onNodePointerDown(e, n.id)}
                onContextMenu={(e) => onNodeContextMenu(e, n)}
              >
                <span
                  className="afc-node-port in"
                  title={t("agentFlow.connectHint")}
                  onPointerDown={(e) => onInPortPointerDown(e, n.id)}
                />
                <div className="afc-node-body">
                  <span className="afc-node-name" title={isMain ? undefined : n.agentType}>
                    {nameOf(n.agentType)}
                  </span>
                  <span className="afc-node-tag">
                    {isMain
                      ? t("agentFlow.defaultTag")
                      : isCustom
                        ? t("agentFlow.customGroup")
                        : t("agentFlow.builtinGroup")}
                    {hasOverride(n.overrides) && (
                      <span className="afc-node-badge" title={t("agentFlow.nodeCustomized")}>
                        {t("agentFlow.nodeCustomized")}
                      </span>
                    )}
                  </span>
                </div>
                {!isMain && (
                  <button
                    type="button"
                    className={`afc-node-power ${n.disabled ? "is-off" : "is-on"}`}
                    title={n.disabled ? t("agentFlow.enableNode") : t("agentFlow.disableNode")}
                    tabIndex={tab}
                    onPointerDown={(e) => e.stopPropagation()}
                    onClick={(e) => {
                      e.stopPropagation();
                      toggleNodeDisabled(n.id);
                    }}
                  >
                    <PowerIcon />
                  </button>
                )}
                <button
                  type="button"
                  className="afc-node-menu"
                  title={t("agentFlow.nodeMenu")}
                  tabIndex={tab}
                  onPointerDown={(e) => e.stopPropagation()}
                  onClick={(e) => {
                    e.stopPropagation();
                    const host = containerRef.current?.getBoundingClientRect();
                    const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
                    setAddMenu(null);
                    setNodeMenu({
                      x: r.left - (host?.left ?? 0),
                      y: r.bottom - (host?.top ?? 0) + 4,
                      nodeId: n.id,
                    });
                  }}
                >
                  <DotsIcon />
                </button>
                <span
                  className="afc-node-port out"
                  title={t("agentFlow.connectHint")}
                  onPointerDown={(e) => beginConnect(e, n.id, "out")}
                />
              </div>
            );
          })}
        </div>

        {!sessionId && <div className="afc-hint">{t("agentFlow.noSessionHint")}</div>}

        {minimap && (
          <div
            className="afc-minimap"
            style={{ width: minimap.MW, height: minimap.MH }}
            onPointerDown={(e) => e.stopPropagation()}
            onClick={onMinimapClick}
          >
            {minimap.nodes.map((n) => (
              <span
                key={`mm-${n.id}`}
                className={`afc-minimap-node ${n.agentType === MAIN ? "is-main" : ""}`}
                style={{
                  left: (n.x - minimap.minX) * minimap.s,
                  top: (n.y - minimap.minY) * minimap.s,
                  width: Math.max(3, NODE_W * minimap.s),
                  height: Math.max(3, NODE_H * minimap.s),
                }}
              />
            ))}
            <span
              className="afc-minimap-view"
              style={{
                left: minimap.view.x,
                top: minimap.view.y,
                width: minimap.view.w,
                height: minimap.view.h,
              }}
            />
          </div>
        )}

        {addMenu && (
          <AddMenu
            x={addMenu.x}
            y={addMenu.y}
            agents={agents}
            t={t}
            onPick={(agentType) => {
              addNode(agentType, {
                mode: addMenu.mode,
                edgeId: addMenu.edgeId,
                targetId: addMenu.targetId,
              });
              setAddMenu(null);
            }}
            onClose={() => setAddMenu(null)}
          />
        )}

        {nodeMenu &&
          (() => {
            const n = graph.nodes.find((x) => x.id === nodeMenu.nodeId);
            if (!n) return null;
            const isMain = n.agentType === MAIN;
            const isCustom = n.agentType.startsWith(CUSTOM_PREFIX);
            return (
              <div
                className="afc-menu"
                style={{ left: nodeMenu.x, top: nodeMenu.y }}
                onPointerDown={(e) => e.stopPropagation()}
              >
                <button
                  type="button"
                  className="afc-menu-item"
                  onClick={() => {
                    onEditNodeConfig({
                      nodeId: n.id,
                      agentType: n.agentType,
                      overrides: n.overrides,
                    });
                    setNodeMenu(null);
                  }}
                >
                  {t("agentFlow.editAgent")}
                </button>
                {isCustom && (
                  <button
                    type="button"
                    className="afc-menu-item"
                    onClick={() => {
                      onEditAgent(n.agentType);
                      setNodeMenu(null);
                    }}
                  >
                    {t("agentFlow.editAgentGlobal")}
                  </button>
                )}
                {!isMain && (
                  <>
                    <button
                      type="button"
                      className="afc-menu-item"
                      onClick={() => {
                        toggleNodeDisabled(n.id);
                        setNodeMenu(null);
                      }}
                    >
                      {n.disabled ? t("agentFlow.enableNode") : t("agentFlow.disableNode")}
                    </button>
                    <button
                      type="button"
                      className="afc-menu-item"
                      onClick={() => {
                        removeNode(n.id);
                        setNodeMenu(null);
                      }}
                    >
                      {t("agentFlow.removeNode")}
                    </button>
                  </>
                )}
                {isCustom && (
                  <button
                    type="button"
                    className="afc-menu-item danger"
                    onClick={() => {
                      onDeleteAgent(n.agentType);
                      setNodeMenu(null);
                    }}
                  >
                    {t("agentFlow.deleteAgent")}
                  </button>
                )}
              </div>
            );
          })()}

        {connectMenu &&
          (() => {
            const targets = validConnectTargets(connectMenu.fixedId, connectMenu.dir);
            return (
              <div
                className="afc-menu afc-add-menu"
                style={{ left: connectMenu.x, top: connectMenu.y }}
                onPointerDown={(e) => e.stopPropagation()}
              >
                {targets.length > 0 && (
                  <>
                    <div className="afc-menu-group">{t("agentFlow.connectTo")}</div>
                    {targets.map((n) => (
                      <button
                        key={n.id}
                        type="button"
                        className="afc-menu-item"
                        onClick={() => connectFromMenu(n.id)}
                      >
                        {nameOf(n.agentType)}
                      </button>
                    ))}
                  </>
                )}
                <div className="afc-menu-group">{t("agentFlow.connectAddAgent")}</div>
                {agents.length === 0 ? (
                  <div className="afc-menu-empty">{t("agentFlow.connectNoTargets")}</div>
                ) : (
                  agents.map((a) => (
                    <button
                      key={a.id}
                      type="button"
                      className="afc-menu-item"
                      onClick={() => addAndConnectFromMenu(a.id)}
                    >
                      {a.name}
                    </button>
                  ))
                )}
              </div>
            );
          })()}
      </div>
    </div>
  );
});

function AddMenu({
  x,
  y,
  agents,
  t,
  onPick,
  onClose,
}: {
  x: number;
  y: number;
  agents: Agent[];
  t: (k: string) => string;
  onPick: (agentType: string) => void;
  onClose: () => void;
}) {
  const builtins = agents.filter((a) => !a.custom);
  const customs = agents.filter((a) => a.custom);
  useEffect(() => {
    const onDoc = () => onClose();
    window.addEventListener("pointerdown", onDoc);
    return () => window.removeEventListener("pointerdown", onDoc);
  }, [onClose]);
  return (
    <div
      className="afc-menu afc-add-menu"
      style={{ left: x, top: y }}
      onPointerDown={(e) => e.stopPropagation()}
    >
      {builtins.length > 0 && <div className="afc-menu-group">{t("agentFlow.builtinGroup")}</div>}
      {builtins.map((a) => (
        <button key={a.id} type="button" className="afc-menu-item" onClick={() => onPick(a.id)}>
          {a.name}
        </button>
      ))}
      {customs.length > 0 && <div className="afc-menu-group">{t("agentFlow.customGroup")}</div>}
      {customs.map((a) => (
        <button key={a.id} type="button" className="afc-menu-item" onClick={() => onPick(a.id)}>
          {a.name}
        </button>
      ))}
    </div>
  );
}

function PlusIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round">
      <path d="M12 5v14M5 12h14" />
    </svg>
  );
}
function PowerIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round">
      <path d="M12 3v9" />
      <path d="M7.5 6.5a7 7 0 1 0 9 0" />
    </svg>
  );
}
function DotsIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="currentColor">
      <circle cx="5" cy="12" r="1.6" />
      <circle cx="12" cy="12" r="1.6" />
      <circle cx="19" cy="12" r="1.6" />
    </svg>
  );
}
function FitIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round">
      <path d="M4 9V5a1 1 0 0 1 1-1h4M20 9V5a1 1 0 0 0-1-1h-4M4 15v4a1 1 0 0 0 1 1h4M20 15v4a1 1 0 0 1-1 1h-4" />
    </svg>
  );
}
