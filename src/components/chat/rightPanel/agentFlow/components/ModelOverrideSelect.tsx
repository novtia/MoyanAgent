import type { ModelProvider } from "../../../../../types";

export function ModelOverrideSelect({
  value,
  disabled,
  providers,
  t,
  onChange,
}: {
  value: string;
  disabled?: boolean;
  providers: ModelProvider[];
  t: (key: string) => string;
  onChange: (model: string) => void;
}) {
  const known = providers.some((p) => p.models.some((m) => m.id === value));
  return (
    <select
      className="agent-flow-select"
      value={value}
      disabled={disabled}
      onChange={(e) => onChange(e.target.value)}
    >
      <option value="">{t("agentFlow.formModelDefault")}</option>
      {value && !known && <option value={value}>{value}</option>}
      {providers.map((p) => (
        <optgroup key={p.id} label={p.name}>
          {p.models.map((m) => (
            <option key={`${p.id}:${m.id}`} value={m.id}>
              {m.name}
            </option>
          ))}
        </optgroup>
      ))}
    </select>
  );
}

