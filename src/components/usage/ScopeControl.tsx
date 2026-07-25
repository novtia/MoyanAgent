import { useTranslation } from "react-i18next";
import type { UsageScope } from "./format";

const SCOPES: UsageScope[] = ["today", "7d", "30d", "all"];

const SCOPE_KEYS: Record<UsageScope, string> = {
  today: "usage.scopeToday",
  "7d": "usage.scope7d",
  "30d": "usage.scope30d",
  all: "usage.scopeAll",
};

interface ScopeControlProps {
  value: UsageScope;
  onChange: (scope: UsageScope) => void;
}

export function ScopeControl({ value, onChange }: ScopeControlProps) {
  const { t } = useTranslation();
  return (
    <div className="usage-scope" role="tablist" aria-label={t("usage.title")}>
      {SCOPES.map((scope) => (
        <button
          key={scope}
          type="button"
          role="tab"
          aria-selected={value === scope}
          className={value === scope ? "on" : undefined}
          onClick={() => onChange(scope)}
        >
          {t(SCOPE_KEYS[scope])}
        </button>
      ))}
    </div>
  );
}
