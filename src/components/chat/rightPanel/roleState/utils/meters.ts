import type { RoleMeter } from "../../../../../store/roleState";

/** Clamp any model-authored number into a sane 0-100 percentage. */
export function pct(value: number, max = 100): number {
  if (!Number.isFinite(value) || !Number.isFinite(max) || max <= 0) return 0;
  return Math.max(0, Math.min(100, (value / max) * 100));
}

export function asMeter(v: RoleMeter | number): RoleMeter {
  if (typeof v === "number") return { value: v, max: 100 };
  return { value: Number(v?.value ?? 0), max: Number(v?.max ?? 100) || 100 };
}
