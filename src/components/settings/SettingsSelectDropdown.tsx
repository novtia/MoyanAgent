import { useEffect, useRef, useState } from "react";
import { CaretIcon, CheckIcon } from "./icons";

export interface SettingsSelectOption {
  value: string;
  label: string;
}

interface SettingsSelectDropdownProps {
  value: string;
  options: SettingsSelectOption[];
  ariaLabel: string;
  onChange: (value: string) => void;
  className?: string;
}

export function SettingsSelectDropdown({
  value,
  options,
  ariaLabel,
  onChange,
  className = "",
}: SettingsSelectDropdownProps) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement | null>(null);
  const selected = options.find((option) => option.value === value);
  const selectedLabel = selected?.label ?? options[0]?.label ?? "";

  useEffect(() => {
    if (!open) return;
    const onDoc = (event: MouseEvent) => {
      if (ref.current && !ref.current.contains(event.target as Node)) setOpen(false);
    };
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") setOpen(false);
    };
    window.addEventListener("mousedown", onDoc);
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("mousedown", onDoc);
      window.removeEventListener("keydown", onKey);
    };
  }, [open]);

  return (
    <div
      className={`lang-dropdown lang-dropdown--settings ${className}`.trim()}
      ref={ref}
    >
      <button
        type="button"
        className={`lang-dropdown-trigger ${open ? "active" : ""}`}
        onClick={() => setOpen((v) => !v)}
        aria-haspopup="listbox"
        aria-expanded={open}
        aria-label={ariaLabel}
      >
        <span className="lang-dropdown-trigger-text">{selectedLabel}</span>
        <CaretIcon />
      </button>
      {open && (
        <div className="lang-dropdown-menu" role="listbox" aria-label={ariaLabel}>
          {options.map((option) => {
            const active = option.value === value;
            return (
              <button
                key={option.value}
                type="button"
                role="option"
                aria-selected={active}
                className={`lang-dropdown-item ${active ? "active" : ""}`}
                onClick={() => {
                  onChange(option.value);
                  setOpen(false);
                }}
              >
                <span className="lang-dropdown-item-text">{option.label}</span>
                {active && <CheckIcon />}
              </button>
            );
          })}
        </div>
      )}
    </div>
  );
}
