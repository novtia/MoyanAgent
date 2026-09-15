import { createContext, useContext, useMemo, type ReactNode } from "react";

export interface PanelTabScope {
  tabId: string;
  /** True when this tab is the one the user is looking at. */
  isActive: boolean;
  /** Session the tab belongs to; panes never read another session's state. */
  sessionId: string | null;
}

const PanelTabCtx = createContext<PanelTabScope | null>(null);

export function PanelTabProvider({
  tabId,
  isActive,
  sessionId,
  children,
}: PanelTabScope & { children: ReactNode }) {
  const value = useMemo(
    () => ({ tabId, isActive, sessionId }),
    [tabId, isActive, sessionId],
  );
  return <PanelTabCtx.Provider value={value}>{children}</PanelTabCtx.Provider>;
}

/** Scope of the tab pane the component is rendered in. */
export function usePanelTab(): PanelTabScope {
  const ctx = useContext(PanelTabCtx);
  if (!ctx) throw new Error("usePanelTab must be used inside a PanelTabProvider");
  return ctx;
}
