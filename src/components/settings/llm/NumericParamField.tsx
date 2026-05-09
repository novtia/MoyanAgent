import { useEffect, useState } from "react";
import type { NumericFieldDef } from "./modelParams";

interface NumericParamFieldProps {
  def: NumericFieldDef;
  value: number | null;
  onCommit: (next: number | null) => void;
  invalidLabel: string;
  label: string;
  hint: string;
}

export function NumericParamField({
  def,
  value,
  onCommit,
  invalidLabel,
  label,
  hint,
}: NumericParamFieldProps) {
  const [draft, setDraft] = useState<string>(value == null ? "" : String(value));
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    setDraft(value == null ? "" : String(value));
    setError(null);
  }, [value]);

  const commit = () => {
    const trimmed = draft.trim();
    if (trimmed === "") {
      setError(null);
      if (value !== null) onCommit(null);
      return;
    }
    const parsed = def.integer ? Number.parseInt(trimmed, 10) : Number(trimmed);
    if (!Number.isFinite(parsed)) {
      setError(invalidLabel);
      return;
    }
    if (def.min !== undefined && parsed < def.min) {
      setError(invalidLabel);
      return;
    }
    if (def.max !== undefined && parsed > def.max) {
      setError(invalidLabel);
      return;
    }
    setError(null);
    if (parsed !== value) onCommit(parsed);
  };

  return (
    <div className="settings-param-field">
      <label className="field-label">{label}</label>
      <input
        type="number"
        className="field-input field-input--mono"
        inputMode={def.integer ? "numeric" : "decimal"}
        step={def.step}
        min={def.min}
        max={def.max}
        value={draft}
        placeholder={def.placeholder}
        onChange={(e) => setDraft(e.target.value)}
        onBlur={commit}
        onKeyDown={(e) => {
          if (e.key === "Enter") {
            e.preventDefault();
            commit();
          }
        }}
      />
      <div className={`hint ${error ? "is-error" : ""}`}>{error ?? hint}</div>
    </div>
  );
}
