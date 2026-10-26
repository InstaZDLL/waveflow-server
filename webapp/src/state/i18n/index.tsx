import {
  createContext,
  type ReactNode,
  useCallback,
  useContext,
  useEffect,
  useState,
} from "react";

import { en } from "./en";
import { fr } from "./fr";

export type TranslationKey = keyof typeof en;
export type TranslationValue = string | { one: string; other: string };

export type Locale = "en" | "fr";

type I18n = {
  locale: Locale;
  setLocale: (locale: Locale) => void;
  t: (key: TranslationKey, values?: Record<string, string | number>) => string;
};

const I18nContext = createContext<I18n | null>(null);
const STORAGE_KEY = "waveflow.locale";
const pluralRules = {
  en: new Intl.PluralRules("en"),
  fr: new Intl.PluralRules("fr"),
};

export function translate(
  locale: Locale,
  key: TranslationKey,
  values: Record<string, string | number> = {},
): string {
  const candidate: TranslationValue = (locale === "fr" ? fr : en)[key];
  const count = values.count;
  let value =
    typeof candidate === "string"
      ? candidate
      : candidate[
          typeof count === "number" &&
          count === 1 &&
          pluralRules[locale].select(count) === "one"
            ? "one"
            : "other"
        ];
  for (const [name, replacement] of Object.entries(values)) {
    value = value.replaceAll(`{${name}}`, String(replacement));
  }
  return value;
}

export function I18nProvider({ children }: { children: ReactNode }) {
  const [locale, setLocale] = useState<Locale>(() => {
    try {
      const saved = localStorage.getItem(STORAGE_KEY);
      if (saved === "en" || saved === "fr") return saved;
    } catch {
      // Fall back to the browser language when storage is unavailable.
    }
    return navigator.language.toLowerCase().startsWith("fr") ? "fr" : "en";
  });

  useEffect(() => {
    document.documentElement.lang = locale;
    try {
      localStorage.setItem(STORAGE_KEY, locale);
    } catch {
      // Language switching remains available without persistence.
    }
  }, [locale]);

  const t: I18n["t"] = useCallback(
    (key, values = {}) => translate(locale, key, values),
    [locale],
  );

  return (
    <I18nContext.Provider value={{ locale, setLocale, t }}>
      {children}
    </I18nContext.Provider>
  );
}

export function useI18n(): I18n {
  const value = useContext(I18nContext);
  if (!value) throw new Error("useI18n requires I18nProvider");
  return value;
}

export function LanguagePicker() {
  const { locale, setLocale, t } = useI18n();
  return (
    <label className="language-picker">
      <span>{t("preferences.language")}</span>
      <select
        aria-label={t("preferences.language")}
        value={locale}
        onChange={(event) => setLocale(event.target.value as Locale)}
      >
        <option value="en">English</option>
        <option value="fr">Français</option>
      </select>
    </label>
  );
}
