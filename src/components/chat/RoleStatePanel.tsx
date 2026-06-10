import { useEffect } from "react";
import { useTranslation } from "react-i18next";

import { useSession } from "../../store/session";
import { useRoleState } from "../../store/roleState";
import { RoleStateCard } from "./RoleStateCard";

interface RoleStatePanelProps {
  open: boolean;
}

export function RoleStatePanel({ open }: RoleStatePanelProps) {
  const { t } = useTranslation();
  const sessionId = useSession((s) => s.active?.session.id ?? null);
  const loadLatest = useRoleState((s) => s.loadLatest);
  // Subscribe to the per-session maps so the panel re-renders on every
  // incremental op; `rolesOf` then derives the ordered, stable list.
  const rolesBySession = useRoleState((s) => s.rolesBySession);
  const orderBySession = useRoleState((s) => s.orderBySession);

  useEffect(() => {
    if (open && sessionId) void loadLatest(sessionId);
  }, [open, sessionId, loadLatest]);

  const map = sessionId ? rolesBySession[sessionId] : undefined;
  const order = sessionId ? orderBySession[sessionId] : undefined;
  const roles = map && order ? order.map((id) => map[id]).filter(Boolean) : [];

  if (!sessionId) {
    return (
      <div className="rs-panel rs-empty">
        <p className="rs-empty-text">{t("roleState.noSession")}</p>
      </div>
    );
  }

  if (roles.length === 0) {
    return (
      <div className="rs-panel rs-empty">
        <div className="rs-empty-art">🎭</div>
        <p className="rs-empty-text">{t("roleState.empty")}</p>
        <p className="rs-empty-hint">{t("roleState.emptyHint")}</p>
      </div>
    );
  }

  return (
    <div className="rs-panel">
      {roles.map((role) => (
        // Stable role id as key → each card is an independent instance that
        // only re-renders when its own role reference changes.
        <RoleStateCard key={role.id} role={role} />
      ))}
    </div>
  );
}
