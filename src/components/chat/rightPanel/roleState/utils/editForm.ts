import type { Role, RoleMeter } from "../../../../../store/roleState";
import type { AttrRow, MeterRow } from "../types";

export function clamp01to100(n: number): number {
  if (!Number.isFinite(n)) return 0;
  return Math.max(0, Math.min(100, n));
}

export function parseTags(tags: unknown): string[] {
  return Array.isArray(tags) ? tags.map(String).filter((t) => t.trim()) : [];
}

export function parseAttrs(attrs: Role["attributes"]): AttrRow[] {
  if (!attrs || typeof attrs !== "object") return [];
  return Object.entries(attrs)
    .filter(([, v]) => typeof v === "number")
    .map(([key, value]) => ({ key, value: value as number }));
}

export function parseMeters(meters: Role["meters"]): MeterRow[] {
  if (!meters || typeof meters !== "object") return [];
  return Object.entries(meters).map(([name, raw]) => {
    if (typeof raw === "number") return { name, value: raw, max: 100 };
    const m = raw as RoleMeter;
    return {
      name,
      value: Number(m?.value ?? 0),
      max: Number(m?.max ?? 100) || 100,
    };
  });
}

export function spotsToText(spots: string[]): string {
  return spots.join(", ");
}

export function textToSpots(text: string): string[] {
  return text
    .split(/[,，]/)
    .map((s) => s.trim())
    .filter(Boolean);
}
