import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { useTranslation } from "react-i18next";

export const MAIN = "__main__";
const CUSTOM_PREFIX = "custom:";

const NODE_W = 168;
const NODE_H = 62;
const ZOOM_MIN = 0.4;
const ZOOM_MAX = 1.8;
const GAP_X = 90;

interface FNode {
  id: string;
  agentType: string;
  x: number;
  y: number;
}
interface FEdge {
  id: string;
  from: string;
  to: string;
}
interface Graph {
  nodes: FNode[];
  edges: FEdge[];
}

interface Agent {
  id: string;
  name: string;
  custom: boolean;
}

interface AgentFlowCanvasProps {
  open: boolean;
  sessionId: string | null;
  chain: string[];
  agents: Agent[];
  knownTypes: Set<string>;
  nameOf: (agentType: string) => string;
  onOrderChange: (order: string[]) => void;
  onRequestNewAgent: () => void;
  onEditAgent: (agentType: string) => void;
  onDeleteAgent: (agentType: string) => void;
}

let idCounter = 0;
function genId(prefix: string) {
  idCounter += 1;
  return `${prefix}-${Date.now().toString(36)}-${idCounter}`;
}

function graphKey(sessionId: string) {
  return `atelier:agent-flow:${sessionId}`;
}

function normalizeChain(chain: string[]): string[] {
  if (chain.length === 0) return [MAIN];
  return chain.includes(MAIN) ? chain : [MAIN, ...chain];
}

function buildGraphFromChain(chain: string[]): Graph {
  const order = normalizeChain(chain);
  const nodes: FNode[] = order.map((agentType, i) => ({
    id: genId("n"),
    agentType,
    x: 48 + i * (NODE_W + GAP_X),
    y: 96,
  }));
  const edges: FEdge[] = [];
  for (let i = 0; i < nodes.length - 1; i++) {
    edges.push({ id: genId("e"), from: nodes[i].id, to: nodes[i + 1].id });
  }
  return { nodes, edges };
}

function loadGraph(sessionId: string, chain: string[]): Graph {
  try {
    const raw = window.localStorage.getItem(graphKey(sessionId));
    if (raw) {
      const parsed = JSON.parse(raw) as Graph;
      if (Array.isArray(parsed.nodes) && Array.isArray(parsed.edges)) {
        if (parsed.nodes.some((n) => n.agentType === MAIN)) {
          return parsed;
        }
      }
    }
  } catch {
    /* ignore */
  }
  return buildGraphFromChain(chain);
}

/** Walk edges from the head node (no incoming) to produce the linear order. */
function deriveOrder(graph: Graph): string[] {
  const { nodes, edges } = graph;
  if (nodes.length === 0) return [MAIN];
  const incoming = new Set(edges.map((e) => e.to));
  const outBy = new Map<string, string>();
  for (const e of edges) outBy.set(e.from, e.to);

  let head = nodes.find((n) => !incoming.has(n.id)) ?? nodes[0];
  const visited: string[] = [];
  const seen = new Set<string>();
  let cur: FNode | undefined = head;
  while (cur && !seen.has(cur.id)) {
    seen.add(cur.id);
    visited.push(cur.agentType);
    const nextId = outBy.get(cur.id);
    cur = nextId ? nodes.find((n) => n.id === nextId) : undefined;
  }
  // Guarantee the main agent always runs even if it was left unconnected.
  if (!visited.includes(MAIN)) visited.push(MAIN);
  return visited;
}

export function AgentFlowCanvas({
  open,
  sessionId,
  chain,
  agents,
  knownTypes,
  nameOf,
  onOrderChange,
  onRequestNewAgent,
  onEditAgent,
  onDeleteAgent,
}: AgentFlowCanvasProps) {
  const { t } = useTranslation();
  const containerRef = useRef<HTMLDivElement | null>(null);

  const [graph, setGraph] = useState<Graph>({ nodes: [], edges: [] });
  const [pan, setPan] = useState({ x: 0, y: 0 });
  const [zoom, setZoom] = useState(1);
  const [viewport, setViewport] = useState({ w: 0, h: 0 });
  const [needsFit, setNeedsFit] = useState(false);
  const [connecting, setConnecting] = useState<{ from: string; wx: number; wy: number } | null>(
    null,
  );
  const [addMenu, setAddMenu] = useState<
    { x: number; y: number; mode: "append" | "insert"; edgeId?: string } | null
  >(null);
  const [nodeMenu, setNodeMenu] = useState<{ x: number; y: number; nodeId: string } | null>(null);

  const panRef = useRef(pan);
  const zoomRef = useRef(zoom);
  const graphRef = useRef(graph);
  panRef.current = pan;
  zoomRef.current = zoom;
  graphRef.current = graph;

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

  // ── init graph per session ──────────────────────────────────────────────
  // The main node is always present (even without an active session) so the
  // canvas is never empty; persistence is gated on sessionId elsewhere.
  useEffect(() => {
    const key = sessionId ?? "__none__";
    if (initedFor.current === key) return;
    initedFor.current = key;
    const g = sessionId ? loadGraph(sessionId, chain) : buildGraphFromChain(chain);
    setGraph(g);
    lastPushedOrder.current = JSON.stringify(deriveOrder(g));
    setNeedsFit(true);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [sessionId]);

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
    if (!sessionId) return;
    try {
      window.localStorage.setItem(graphKey(sessionId), JSON.stringify(graph));
    } catch {
      /* ignore */
    }
    const order = deriveOrder(graph);
    const serialized = JSON.stringify(order);
    if (serialized !== lastPushedOrder.current) {
      lastPushedOrder.current = serialized;
      onOrderChange(order);
    }
  }, [graph, sessionId, onOrderChange]);

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

  // ── connect (drag from output port) ──────────────────────────────────────
  const onPortPointerDown = (e: React.PointerEvent, fromId: string) => {
    if (e.button !== 0) return;
    e.stopPropagation();
    closeMenus();
    const w0 = toWorld(e.clientX, e.clientY);
    setConnecting({ from: fromId, wx: w0.x, wy: w0.y });
    const onMove = (ev: PointerEvent) => {
      const w = toWorld(ev.clientX, ev.clientY);
      setConnecting((c) => (c ? { ...c, wx: w.x, wy: w.y } : c));
    };
    const onUp = (ev: PointerEvent) => {
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", onUp);
      const w = toWorld(ev.clientX, ev.clientY);
      const target = graphRef.current.nodes.find(
        (n) => n.id !== fromId && w.x >= n.x && w.x <= n.x + NODE_W && w.y >= n.y && w.y <= n.y + NODE_H,
      );
      if (target) connect(fromId, target.id);
      setConnecting(null);
    };
    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onUp);
  };

  const connect = (from: string, to: string) => {
    setGraph((g) => {
      if (from === to) return g;
      // Reject connections that would create a cycle.
      const outBy = new Map(g.edges.map((e) => [e.from, e.to] as const));
      let cur: string | undefined = to;
      const guard = new Set<string>();
      while (cur && !guard.has(cur)) {
        if (cur === from) return g; // would cycle
        guard.add(cur);
        cur = outBy.get(cur);
      }
      const edges = g.edges.filter((e) => e.from !== from && e.to !== to);
      edges.push({ id: genId("e"), from, to });
      return { ...g, edges };
    });
  };

  const addNode = (agentType: string, ctx: { mode: "append" | "insert"; edgeId?: string }) => {
    setGraph((g) => {
      const node: FNode = { id: genId("n"), agentType, x: 0, y: 0 };
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
      // append: place to the right of the tail node and connect from it
      const tail = g.nodes.find((n) => !g.edges.some((e) => e.from === n.id)) ?? g.nodes[g.nodes.length - 1];
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

  const closeMenus = () => {
    setAddMenu(null);
    setNodeMenu(null);
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
              <path
                key={e.id}
                d={e.d}
                fill="none"
                stroke="var(--line-strong)"
                strokeWidth={2}
                markerEnd="url(#afc-arrow)"
              />
            ))}
            {connecting &&
              (() => {
                const a = nodeById.get(connecting.from);
                if (!a) return null;
                const x1 = a.x + NODE_W;
                const y1 = a.y + NODE_H / 2;
                const dx = Math.max(40, Math.abs(connecting.wx - x1) / 2);
                return (
                  <path
                    d={`M ${x1} ${y1} C ${x1 + dx} ${y1}, ${connecting.wx - dx} ${connecting.wy}, ${connecting.wx} ${connecting.wy}`}
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

          {graph.nodes.map((n) => {
            const isMain = n.agentType === MAIN;
            const isCustom = n.agentType.startsWith(CUSTOM_PREFIX);
            return (
              <div
                key={n.id}
                className={`afc-node ${isMain ? "is-main" : ""}`}
                style={{ left: n.x, top: n.y, width: NODE_W, height: NODE_H }}
                onPointerDown={(e) => onNodePointerDown(e, n.id)}
              >
                <span className="afc-node-port in" aria-hidden />
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
                  </span>
                </div>
                {!isMain && (
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
                )}
                <span
                  className="afc-node-port out"
                  title={t("agentFlow.connectHint")}
                  onPointerDown={(e) => onPortPointerDown(e, n.id)}
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
              addNode(agentType, { mode: addMenu.mode, edgeId: addMenu.edgeId });
              setAddMenu(null);
            }}
            onClose={() => setAddMenu(null)}
          />
        )}

        {nodeMenu &&
          (() => {
            const n = graph.nodes.find((x) => x.id === nodeMenu.nodeId);
            if (!n) return null;
            const isCustom = n.agentType.startsWith(CUSTOM_PREFIX);
            return (
              <div
                className="afc-menu"
                style={{ left: nodeMenu.x, top: nodeMenu.y }}
                onPointerDown={(e) => e.stopPropagation()}
              >
                {isCustom && (
                  <button
                    type="button"
                    className="afc-menu-item"
                    onClick={() => {
                      onEditAgent(n.agentType);
                      setNodeMenu(null);
                    }}
                  >
                    {t("agentFlow.editAgent")}
                  </button>
                )}
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
      </div>
    </div>
  );
}

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
