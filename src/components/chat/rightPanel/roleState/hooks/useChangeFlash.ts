import { useEffect, useRef, useState } from "react";

/** Track which scalar keys changed since the previous render so we can flash
 * just the affected rows (smooth incremental updates per the spec). */
export function useChangedKeys(snapshot: Record<string, number>): Set<string> {
  const prevRef = useRef<Record<string, number>>(snapshot);
  const [changed, setChanged] = useState<Set<string>>(new Set());

  useEffect(() => {
    const prev = prevRef.current;
    const next = new Set<string>();
    for (const [k, v] of Object.entries(snapshot)) {
      if (prev[k] !== v) next.add(k);
    }
    prevRef.current = snapshot;
    if (next.size === 0) return;
    setChanged(next);
    const t = window.setTimeout(() => setChanged(new Set()), 900);
    return () => window.clearTimeout(t);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [JSON.stringify(snapshot)]);

  return changed;
}

/** Flash when a string field changes (e.g. `nsfw.semen.exterior`). */
export function useChangedString(value: string | null | undefined): boolean {
  const prevRef = useRef(value);
  const [changed, setChanged] = useState(false);

  useEffect(() => {
    if (prevRef.current !== value) {
      prevRef.current = value;
      setChanged(true);
      const t = window.setTimeout(() => setChanged(false), 900);
      return () => window.clearTimeout(t);
    }
  }, [value]);

  return changed;
}
