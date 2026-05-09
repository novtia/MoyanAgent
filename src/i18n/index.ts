import i18n from "i18next";
import { initReactI18next } from "react-i18next";
import enUS from "./locales/en-US";
import zhCN from "./locales/zh-CN";

export const SUPPORTED_LANGUAGES = ["zh-CN", "en-US"] as const;
export type SupportedLanguage = (typeof SUPPORTED_LANGUAGES)[number];

const STORAGE_KEY = "atelier.language";
const DEFAULT_LANGUAGE: SupportedLanguage = "zh-CN";

function readStoredLanguage(): SupportedLanguage {
  try {
    const stored = window.localStorage.getItem(STORAGE_KEY);
    if (stored && (SUPPORTED_LANGUAGES as readonly string[]).includes(stored)) {
      return stored as SupportedLanguage;
    }
  } catch {
    // ignore storage access errors (e.g. SSR / privacy mode)
  }
  return DEFAULT_LANGUAGE;
}

void i18n.use(initReactI18next).init({
  resources: {
    "zh-CN": { translation: zhCN },
    "en-US": { translation: enUS },
  },
  lng: readStoredLanguage(),
  fallbackLng: DEFAULT_LANGUAGE,
  defaultNS: "translation",
  interpolation: {
    escapeValue: false,
  },
  returnNull: false,
});

export function setLanguage(lang: SupportedLanguage) {
  try {
    window.localStorage.setItem(STORAGE_KEY, lang);
  } catch {
    // ignore
  }
  void i18n.changeLanguage(lang);
}

export function getLanguage(): SupportedLanguage {
  const cur = (i18n.language || DEFAULT_LANGUAGE) as string;
  return (SUPPORTED_LANGUAGES as readonly string[]).includes(cur)
    ? (cur as SupportedLanguage)
    : DEFAULT_LANGUAGE;
}

export default i18n;
