import type { CSSProperties, PointerEvent as ReactPointerEvent } from "react";
import type { Role } from "../../../../store/roleState";

export type AttrRow = { key: string; value: number };
export type MeterRow = { name: string; value: number; max: number };

export interface RoleStateEditModalProps {
  role: Role;
  sessionId: string;
  scopeId: string;
  onClose: () => void;
}

export interface RoleStateCardProps {
  role: Role;
  sessionId: string;
  scopeId: string;
  style?: CSSProperties;
  isDragging?: boolean;
  /** Unified card drag (reorder / cite). */
  onCardPointerDown?: (e: ReactPointerEvent) => void;
}

/** A radar dimension: label, raw value, and the max used to normalise it. */
export type RadarDatum = [string, number, number?];
