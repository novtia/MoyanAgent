import { useEffect } from "react";
import { useReaderFind } from "../../../../../store/readerFind";

export function useReaderFindShortcuts(enabled: boolean) {
  const openFind = useReaderFind((s) => s.openFind);
  const close = useReaderFind((s) => s.close);
  const open = useReaderFind((s) => s.open);

  useEffect(() => {
    if (!enabled) return;
    const onKeyDown = (e: KeyboardEvent) => {
      const mod = e.ctrlKey || e.metaKey;
      if (!mod) return;
      const key = e.key.toLowerCase();
      if (key === "f" || key === "h") {
        e.preventDefault();
        openFind();
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [enabled, openFind]);

  useEffect(() => {
    if (!enabled || !open) return;
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        close();
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [enabled, open, close]);
}
