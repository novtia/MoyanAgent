import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  SUPPORTED_LANGUAGES,
  type SupportedLanguage,
} from "../../../i18n";
import { CaretIcon, CheckIcon, GlobeIcon } from "../icons";

const LANGUAGE_OPTION_KEYS: Record<
  SupportedLanguage,
  "settings.appearance.languageOptionZhCN" | "settings.appearance.languageOptionEnUS"
> = {
  "zh-CN": "settings.appearance.languageOptionZhCN",
  "en-US": "settings.appearance.languageOptionEnUS",
};

interface LanguageDropdownProps {
  current: SupportedLanguage;
  onChange: (lang: SupportedLanguage) => void;
  ariaLabel: string;
}

export function LanguageDropdown({
  current,
  onChange,
  ariaLabel,
}: LanguageDropdownProps) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    if (!open) return;
    const onDoc = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) setOpen(false);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setOpen(false);
    };
    window.addEventListener("mousedown", onDoc);
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("mousedown", onDoc);
      window.removeEventListener("keydown", onKey);
    };
  }, [open]);

  return (
    <div className="lang-dropdown" ref={ref}>
      <button
        type="button"
        className={`lang-dropdown-trigger ${open ? "active" : ""}`}
        onClick={() => setOpen((v) => !v)}
        aria-haspopup="listbox"
        aria-expanded={open}
        aria-label={ariaLabel}
      >
        <GlobeIcon />
        <span className="lang-dropdown-trigger-text">
          {t(LANGUAGE_OPTION_KEYS[current])}
        </span>
        <CaretIcon />
      </button>
      {open && (
        <div className="lang-dropdown-menu" role="listbox">
          {SUPPORTED_LANGUAGES.map((lang) => {
            const active = lang === current;
            return (
              <button
                key={lang}
                type="button"
                role="option"
                aria-selected={active}
                className={`lang-dropdown-item ${active ? "active" : ""}`}
                onClick={() => {
                  onChange(lang);
                  setOpen(false);
                }}
              >
                <span className="lang-dropdown-item-text">
                  {t(LANGUAGE_OPTION_KEYS[lang])}
                </span>
                {active && <CheckIcon />}
              </button>
            );
          })}
        </div>
      )}
    </div>
  );
}
