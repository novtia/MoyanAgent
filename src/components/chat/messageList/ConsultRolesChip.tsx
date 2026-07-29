import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import type { AssistantBlock } from "../../../types";
import { ToolGlyph } from "./toolIcons";

type RoleChoice = {
  role_id?: string;
  name?: string;
  control?: string;
  action?: string | null;
  speech?: string | null;
  needs_ask_user?: boolean;
  error?: string | null;
  memory_path?: string | null;
  model?: string | null;
};

type ConsultOutput = {
  op?: string;
  compacted?: boolean;
  ask_user_required?: boolean;
  choices?: RoleChoice[];
  note?: string;
};

function parseOutput(raw: unknown): ConsultOutput | null {
  if (!raw || typeof raw !== "object") return null;
  return raw as ConsultOutput;
}

export function ConsultRolesChip({
  block,
}: {
  block: Extract<AssistantBlock, { type: "tool_use" }>;
}) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(true);
  const input = (block.input ?? {}) as {
    role_ids?: string[];
    question?: string;
    situation?: string;
  };
  const output = useMemo(() => parseOutput(block.output), [block.output]);
  const choices = output?.choices ?? [];
  const pending = block.status === "pending" || block.streaming;
  const errored = block.status === "error";
  const roleCount =
    choices.length ||
    (Array.isArray(input.role_ids) ? input.role_ids.length : 0);

  return (
    <div
      className={`consult-roles-card${pending ? " is-pending" : ""}${errored ? " is-error" : ""}`}
    >
      <button
        type="button"
        className="consult-roles-head"
        onClick={() => setOpen((v) => !v)}
        aria-expanded={open}
      >
        <span className="ti" aria-hidden>
          <ToolGlyph tool="ConsultRoles" />
        </span>
        <span className="consult-roles-title">{t("message.consultRolesTitle")}</span>
        <span className="consult-roles-meta">
          {pending
            ? t("message.consultRolesPending")
            : errored
              ? t("message.consultRolesError")
              : t("message.consultRolesCount", { count: roleCount })}
          {output?.compacted ? ` · ${t("message.consultRolesCompacted")}` : ""}
          {output?.ask_user_required
            ? ` · ${t("message.consultRolesNeedAskUser")}`
            : ""}
        </span>
        <span className="consult-roles-chevron" aria-hidden>
          {open ? "▾" : "▸"}
        </span>
      </button>

      {input.question ? (
        <div className="consult-roles-question">{input.question}</div>
      ) : null}

      {open && (
        <div className="consult-roles-list">
          {pending && choices.length === 0 ? (
            <div className="consult-roles-empty">{t("message.consultRolesWaiting")}</div>
          ) : null}
          {choices.map((c, i) => {
            const label = c.name || c.role_id || `#${i + 1}`;
            const control = c.control === "user" ? "user" : "ai";
            return (
              <div className="consult-roles-item" key={`${c.role_id ?? label}-${i}`}>
                <div className="consult-roles-item-head">
                  <b>{label}</b>
                  <span className={`consult-roles-control is-${control}`}>
                    {control === "user"
                      ? t("roleState.controlUser")
                      : t("roleState.controlAi")}
                  </span>
                  {c.needs_ask_user ? (
                    <span className="consult-roles-badge">
                      {t("message.consultRolesAskUserBadge")}
                    </span>
                  ) : null}
                </div>
                {c.error && c.error !== "pending" ? (
                  <div className="consult-roles-error">{c.error}</div>
                ) : null}
                {c.action ? (
                  <div className="consult-roles-line">
                    <span className="k">{t("message.consultRolesAction")}</span>
                    <span>{c.action}</span>
                  </div>
                ) : null}
                {c.speech ? (
                  <div className="consult-roles-line">
                    <span className="k">{t("message.consultRolesSpeech")}</span>
                    <span>{c.speech}</span>
                  </div>
                ) : null}
                {c.model ? (
                  <div className="consult-roles-line">
                    <span className="k">{t("roleState.roleModel")}</span>
                    <span>{c.model}</span>
                  </div>
                ) : null}
                {!c.action && !c.speech && !c.error && c.needs_ask_user ? (
                  <div className="consult-roles-hint">
                    {t("message.consultRolesUserHint")}
                  </div>
                ) : null}
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
}
