import { useEffect } from "react";
import { useTranslation } from "react-i18next";

import { useSession } from "../../store/session";
import { resolveRoleStateScope, useRoleState } from "../../store/roleState";
import { RoleStateCard } from "./RoleStateCard";

interface RoleStatePanelProps {
  open: boolean;
}

export function RoleStatePanel({ open }: RoleStatePanelProps) {
  const { t } = useTranslation();
  const session = useSession((s) => s.active?.session ?? null);
  const sessionId = session?.id ?? null;
  const scopeId = session ? resolveRoleStateScope(session) : null;
  const loadLatest = useRoleState((s) => s.loadLatest);
  const rolesByScope = useRoleState((s) => s.rolesByScope);
  const orderByScope = useRoleState((s) => s.orderByScope);

  useEffect(() => {
    if (open && sessionId && scopeId) void loadLatest(sessionId, scopeId);
  }, [open, sessionId, scopeId, loadLatest]);

  const map = scopeId ? rolesByScope[scopeId] : undefined;
  const order = scopeId ? orderByScope[scopeId] : undefined;
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
        <RoleStateCard key={role.id} role={role} sessionId={sessionId} scopeId={scopeId!} />
      ))}
    </div>
  );
}
