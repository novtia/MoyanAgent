import { create } from "zustand";

import { api } from "../api/tauri";

/** Semen / fluid state under `nsfw.精液`. */
export interface RoleNsfwSemen {
  /** 外表精液 — short text (部位 + 大致量感), NOT a number */
  外表?: string;
  /** 吞精量 (ml) */
  吞精?: number;
  /** 阴道精液量 (ml) */
  阴道?: number;
  /** 肛门精液量 (ml) */
  肛门?: number;
}

/** Structured NSFW block on a role card. */
export interface RoleNsfw {
  兴奋度?: number;
  湿润度?: number;
  状态?: string;
  敏感点?: string[];
  精液?: RoleNsfwSemen;
  [key: string]: unknown;
}

/** One named meter rendered as a bar (e.g. 体力 80/100). */
export interface RoleMeter {
  value: number;
  max?: number;
}

/** A character on the state board. Free-form by design; only `id` is required.
 * The recommended numeric-first shape mirrors the backend `RoleState` tool. */
export interface Role {
  id: string;
  name?: string;
  location?: string;
  mood?: string;
  outfit?: string;
  /** 0-100 scalar attributes → radar polygon. */
  attributes?: Record<string, number>;
  /** { value, max } gauges → bars. */
  meters?: Record<string, RoleMeter | number>;
  /** Short status chips. */
  tags?: string[];
  /** Explicit body / arousal state — see [`RoleNsfw`]. */
  nsfw?: RoleNsfw;
  // Any additional model-authored fields pass through untouched.
  [key: string]: unknown;
}

/** Shape of a `RoleState` tool result emitted over `gen://tool`. */
export interface RoleStateOp {
  op: "get" | "create" | "update" | "delete";
  id?: string;
  role?: Role;
  roles?: Role[];
  removed?: boolean;
}

interface RoleStateStore {
  /** sessionId → ordered list of role ids (insertion order). */
  orderBySession: Record<string, string[]>;
  /** sessionId → roleId → role. */
  rolesBySession: Record<string, Record<string, Role>>;

  /** Apply one incremental tool result for a session. */
  applyOp: (sessionId: string, op: RoleStateOp) => void;
  /** Replace a session's board wholesale (load / reset). */
  setRoles: (sessionId: string, roles: Role[]) => void;
  /** Re-fetch the persisted board for a session from the backend. */
  loadLatest: (sessionId: string) => Promise<void>;
  /** Ordered roles for a session (stable references where unchanged). */
  rolesOf: (sessionId: string | null | undefined) => Role[];
}

function dedupeOrder(order: string[]): string[] {
  return Array.from(new Set(order));
}

export const useRoleState = create<RoleStateStore>((set, get) => ({
  orderBySession: {},
  rolesBySession: {},

  setRoles: (sessionId, roles) => {
    const map: Record<string, Role> = {};
    const order: string[] = [];
    for (const r of roles) {
      if (!r || typeof r.id !== "string") continue;
      map[r.id] = r;
      order.push(r.id);
    }
    set((s) => ({
      rolesBySession: { ...s.rolesBySession, [sessionId]: map },
      orderBySession: { ...s.orderBySession, [sessionId]: dedupeOrder(order) },
    }));
  },

  applyOp: (sessionId, op) => {
    if (op.op === "get") {
      if (Array.isArray(op.roles)) get().setRoles(sessionId, op.roles);
      return;
    }
    set((s) => {
      const map = { ...(s.rolesBySession[sessionId] ?? {}) };
      let order = [...(s.orderBySession[sessionId] ?? [])];

      if (op.op === "create" && op.role && typeof op.role.id === "string") {
        map[op.role.id] = op.role;
        if (!order.includes(op.role.id)) order.push(op.role.id);
      } else if (op.op === "update" && op.role && typeof op.role.id === "string") {
        // Backend returns the full updated role; replacing the reference is
        // what triggers the single affected card to re-render.
        map[op.role.id] = op.role;
        if (!order.includes(op.role.id)) order.push(op.role.id);
      } else if (op.op === "delete" && typeof op.id === "string") {
        delete map[op.id];
        order = order.filter((id) => id !== op.id);
      }

      return {
        rolesBySession: { ...s.rolesBySession, [sessionId]: map },
        orderBySession: { ...s.orderBySession, [sessionId]: order },
      };
    });
  },

  loadLatest: async (sessionId) => {
    try {
      const roles = await api.getRoleStates(sessionId);
      get().setRoles(sessionId, Array.isArray(roles) ? roles : []);
    } catch (e) {
      console.warn("[roleState] loadLatest failed", e);
    }
  },

  rolesOf: (sessionId) => {
    if (!sessionId) return [];
    const map = get().rolesBySession[sessionId];
    const order = get().orderBySession[sessionId];
    if (!map || !order) return [];
    return order.map((id) => map[id]).filter(Boolean) as Role[];
  },
}));
