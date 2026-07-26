import { useEffect, useRef, useState } from "react";
import { CaretIcon, CheckIcon } from "../icons";

export interface QuickModelOption {
  providerId: string;
  providerName: string;
  modelId: string;
  modelName: string;
}

export interface QuickModelGroup {
  providerId: string;
  name: string;
  items: { option: QuickModelOption; index: number }[];
}

interface QuickModelDropdownProps {
  groups: QuickModelGroup[];
  options: QuickModelOption[];
  selectedIndex: number;
  noneLabel: string;
  ariaLabel: string;
  onChange: (index: number | null) => void;
}

export function QuickModelDropdown({
  groups,
  options,
  selectedIndex,
  noneLabel,
  ariaLabel,
  onChange,
}: QuickModelDropdownProps) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement | null>(null);

  const selectedLabel =
    selectedIndex >= 0 ? (options[selectedIndex]?.modelName ?? noneLabel) : noneLabel;

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

  const pick = (index: number | null) => {
    onChange(index);
    setOpen(false);
  };

  return (
    <div className="lang-dropdown lang-dropdown--settings" ref={ref}>
      <button
        type="button"
        className={`lang-dropdown-trigger ${open ? "active" : ""}`}
        onClick={() => setOpen((value) => !value)}
        aria-haspopup="listbox"
        aria-expanded={open}
        aria-label={ariaLabel}
      >
        <span className="lang-dropdown-trigger-text">{selectedLabel}</span>
        <CaretIcon />
      </button>
      {open && (
        <div
          className="lang-dropdown-menu lang-dropdown-menu--scroll"
          role="listbox"
          aria-label={ariaLabel}
        >
          <button
            type="button"
            role="option"
            aria-selected={selectedIndex < 0}
            className={`lang-dropdown-item ${selectedIndex < 0 ? "active" : ""}`}
            onClick={() => pick(null)}
          >
            <span className="lang-dropdown-item-text">{noneLabel}</span>
            {selectedIndex < 0 && <CheckIcon />}
          </button>
          {groups.map((group) => (
            <div key={group.providerId} className="lang-dropdown-group">
              <div className="lang-dropdown-group-label">{group.name}</div>
              {group.items.map(({ option, index }) => {
                const active = index === selectedIndex;
                return (
                  <button
                    key={`${option.providerId}:${option.modelId}`}
                    type="button"
                    role="option"
                    aria-selected={active}
                    className={`lang-dropdown-item ${active ? "active" : ""}`}
                    onClick={() => pick(index)}
                  >
                    <span className="lang-dropdown-item-text">{option.modelName}</span>
                    {active && <CheckIcon />}
                  </button>
                );
              })}
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
