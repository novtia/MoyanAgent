import { useCallback, useEffect } from "react";
import { createPortal } from "react-dom";
import { useTranslation } from "react-i18next";

import { useSession } from "../../../../store/session";
import { resolveRoleStateScope, useRoleState } from "../../../../store/roleState";
import { RoleStateCard } from "./RoleStateCard";
import { quoteRoleToComposer, roleToCitePayload } from "./citeRole";
import { useRoleListReorder } from "./hooks/useRoleListReorder";

interface RoleStatePanelProps {
  open: boolean;
}

function EmptyFolderIcon() {
  return (
    <span className="ti" aria-hidden>
      <svg
        width="28"
        height="28"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.6"
        strokeLinecap="round"
        strokeLinejoin="round"
      >
        <path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z" />
        <polyline points="14 2 14 8 20 8" />
        <line x1="12" y1="18" x2="12" y2="12" />
        <line x1="9" y1="15" x2="15" y2="15" />
      </svg>
    </span>
  );
}

export function RoleStatePanel({ open }: RoleStatePanelProps) {
  const { t } = useTranslation();
  const session = useSession((s) => s.active?.session ?? null);
  const sessionId = session?.id ?? null;
  const scopeId = session ? resolveRoleStateScope(session) : null;
  const loadLatest = useRoleState((s) => s.loadLatest);
  const reorderRoles = useRoleState((s) => s.reorderRoles);
  const rolesByScope = useRoleState((s) => s.rolesByScope);
  const orderByScope = useRoleState((s) => s.orderByScope);

  useEffect(() => {
    if (open && sessionId && scopeId) void loadLatest(sessionId, scopeId);
  }, [open, sessionId, scopeId, loadLatest]);

  const map = scopeId ? rolesByScope[scopeId] : undefined;
  const order = scopeId ? orderByScope[scopeId] : undefined;
  const roles = map && order ? order.map((id) => map[id]).filter(Boolean) : [];
  const ids = roles.map((r) => r.id);

  const onReorder = useCallback(
    (orderedIds: string[]) => {
      if (!sessionId || !scopeId) return;
      void reorderRoles(sessionId, scopeId, orderedIds).catch(() => {
        /* store already rolls back */
      });
    },
    [reorderRoles, sessionId, scopeId],
  );

  const onCite = useCallback(
    (id: string) => {
      if (!scopeId) return;
      const role = useRoleState.getState().rolesByScope[scopeId]?.[id];
      if (!role) return;
      quoteRoleToComposer(roleToCitePayload(role));
    },
    [scopeId, t],
  );

  const {
    listRef,
    itemStyle,
    floatingStyle,
    isDragging,
    draggingId,
    dropZone,
    onCardPointerDown,
  } = useRoleListReorder(ids, { onReorder, onCite });

  useEffect(() => {
    if (!isDragging) return;
    const prev = document.body.style.cursor;
    document.body.style.cursor = "grabbing";
    document.body.classList.add("is-arc-dragging");
    return () => {
      document.body.style.cursor = prev;
      document.body.classList.remove("is-arc-dragging");
    };
  }, [isDragging]);

  if (!sessionId) {
    return (
      <div className="arc-panel arc-empty">
        <div className="arc-empty-box">
          <EmptyFolderIcon />
          <p className="arc-empty-text">{t("roleState.noSession")}</p>
        </div>
      </div>
    );
  }

  if (roles.length === 0) {
    return (
      <div className="arc-panel arc-empty">
        <div className="arc-empty-box">
          <EmptyFolderIcon />
          <p className="arc-empty-text">{t("roleState.empty")}</p>
          <p className="arc-empty-hint">{t("roleState.emptyHint")}</p>
        </div>
      </div>
    );
  }

  const draggingRole = draggingId ? roles.find((r) => r.id === draggingId) : null;
  const floatStyle = draggingId ? floatingStyle(draggingId) : undefined;

  return (
    <div
      className={`arc-panel ${isDragging ? "is-reordering" : ""} ${
        dropZone === "list" ? "is-drop-target" : ""
      }`}
    >
      <div ref={listRef} className="arc-list">
        {roles.map((role) => {
          const isFloat = draggingId === role.id;
          return (
            <div
              key={role.id}
              data-role-id={role.id}
              className={`arc-list-item ${isFloat ? "is-dragging-slot" : ""}`}
              style={itemStyle(role.id)}
            >
              {!isFloat && (
                <RoleStateCard
                  role={role}
                  sessionId={sessionId}
                  scopeId={scopeId!}
                  onCardPointerDown={(e) => onCardPointerDown(role.id, e)}
                />
              )}
            </div>
          );
        })}
      </div>

      {draggingRole &&
        floatStyle &&
        createPortal(
          <div className="arc-float-layer" style={floatStyle}>
            <RoleStateCard
              role={draggingRole}
              sessionId={sessionId}
              scopeId={scopeId!}
              isDragging
            />
          </div>,
          document.body,
        )}
    </div>
  );
}
