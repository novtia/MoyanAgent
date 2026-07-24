import { createContext, useContext } from "react";

export interface TreeCtx {
  sessionId: string;
  refreshNonce: number;
  activePath: string | null;
  onOpenFile: (path: string) => void;
  refresh: () => void;
  expand: (dir: string) => void;
  newFile: (dir: string) => void;
  newFolder: (dir: string) => void;
}

export const TreeContext = createContext<TreeCtx | null>(null);

export function useTree(): TreeCtx {
  const ctx = useContext(TreeContext);
  if (!ctx) throw new Error("TreeContext missing");
  return ctx;
}
