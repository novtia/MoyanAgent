import { create } from "zustand";

import { api } from "../api/tauri";

export type RoleGender = "male" | "female";

/** Semen / fluid state under `nsfw.semen` (English keys). */
export interface RoleNsfwSemen {
  /** Male: semen texture / quality (text). */
  texture?: string;
  /** Female: external residue on body (text). */
  exterior?: string;
  /** Female: swallowed volume (ml). */
  swallowed?: number;
  /** Female: vaginal retention (ml). */
  vaginal?: number;
  /** Female: anal retention (ml). */
  anal?: number;
}

/** Structured NSFW block on a role card (English keys). */
export interface RoleNsfw {
  arousal?: number;
  wetness?: number;
  status?: string;
  sensitive_spots?: string[];
  semen?: RoleNsfwSemen;
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
  /** `"male"` | `"female"` — drives semen field rendering. */
  gender?: RoleGender;
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

/** Female semen volume keys (ml). */
export const SEMEN_ML_KEYS = ["swallowed", "vaginal", "anal"] as const;

const LEGACY_SEMEN_ML: Record<(typeof SEMEN_ML_KEYS)[number], string> = {
  swallowed: "吞精",
  vaginal: "阴道",
  anal: "肛门",
};

/** Read `nsfw.semen`, falling back to legacy `nsfw.精液`. */
export function resolveSemen(nsfw: RoleNsfw | undefined): RoleNsfwSemen | undefined {
  if (!nsfw) return undefined;
  const block = nsfw.semen ?? (nsfw as Record<string, unknown>)["精液"];
  if (block && typeof block === "object") return block as RoleNsfwSemen;
  return undefined;
}

/** Normalised gender from role (English keys only). */
export function resolveGender(role: Role): RoleGender | undefined {
  const g = role.gender;
  return g === "male" || g === "female" ? g : undefined;
}

/** Read a female semen ml field (new or legacy Chinese key). */
export function semenMl(
  semen: RoleNsfwSemen | undefined,
  key: (typeof SEMEN_ML_KEYS)[number],
): number | undefined {
  if (!semen) return undefined;
  const legacyKey = LEGACY_SEMEN_ML[key];
  const raw = semen[key] ?? (semen as Record<string, unknown>)[legacyKey];
  return typeof raw === "number" && Number.isFinite(raw) ? raw : undefined;
}

/** Read semen text: `texture` (male) or `exterior` (female), with legacy `外表`. */
export function semenText(
  semen: RoleNsfwSemen | undefined,
  kind: "texture" | "exterior",
): string | null {
  if (!semen) return null;
  const legacyExterior = (semen as Record<string, unknown>)["外表"];
  const raw =
    kind === "texture"
      ? semen.texture
      : semen.exterior ?? (typeof legacyExterior === "string" ? legacyExterior : undefined);
  if (typeof raw !== "string") return null;
  const trimmed = raw.trim();
  return trimmed || null;
}

/** NSFW scalar pairs [key, value] — English first, legacy Chinese fallback. */
export function nsfwScalars(nsfw: RoleNsfw): Array<[string, number]> {
  const pairs: Array<[string, number]> = [];
  const arousal =
    typeof nsfw.arousal === "number"
      ? nsfw.arousal
      : typeof (nsfw as Record<string, unknown>)["兴奋度"] === "number"
        ? ((nsfw as Record<string, unknown>)["兴奋度"] as number)
        : undefined;
  const wetness =
    typeof nsfw.wetness === "number"
      ? nsfw.wetness
      : typeof (nsfw as Record<string, unknown>)["湿润度"] === "number"
        ? ((nsfw as Record<string, unknown>)["湿润度"] as number)
        : undefined;
  if (typeof arousal === "number") pairs.push(["arousal", arousal]);
  if (typeof wetness === "number") pairs.push(["wetness", wetness]);
  return pairs;
}

export function nsfwStatus(nsfw: RoleNsfw): string | null {
  const raw = nsfw.status ?? (nsfw as Record<string, unknown>)["状态"];
  return typeof raw === "string" && raw.trim() ? raw.trim() : null;
}

export function nsfwSensitiveSpots(nsfw: RoleNsfw): string[] {
  const raw = nsfw.sensitive_spots ?? (nsfw as Record<string, unknown>)["敏感点"];
  return Array.isArray(raw) ? raw.map(String) : [];
}

interface RoleStateStore {
  orderBySession: Record<string, string[]>;
  rolesBySession: Record<string, Record<string, Role>>;

  applyOp: (sessionId: string, op: RoleStateOp) => void;
  setRoles: (sessionId: string, roles: Role[]) => void;
  loadLatest: (sessionId: string) => Promise<void>;
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
