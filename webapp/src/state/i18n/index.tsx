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
import { zhCN } from "./zh-CN";

export type TranslationKey = keyof typeof en;
export type TranslationValue = string | { one: string; other: string };

// Every language the client speaks, with the name the picker shows for it,
// written in that language. A new one is a directory of strings typed against
// the English domains, and a line here.
const LOCALES = {
  en: { name: "English", strings: en },
  fr: { name: "Français", strings: fr },
  "zh-CN": { name: "简体中文", strings: zhCN },
} satisfies Record<
  string,
  { name: string; strings: Record<TranslationKey, TranslationValue> }
>;

export type Locale = keyof typeof LOCALES;

function isLocale(value: string | null): value is Locale {
  return value !== null && Object.hasOwn(LOCALES, value);
}

/** The language to start in, from the one the browser prefers. */
export function localeFor(language: string): Locale {
  const tag = language.toLowerCase();
  if (tag.startsWith("fr")) return "fr";
  // Simplified Chinese only: Taiwan, Hong Kong and Macao read Traditional,
  // and are better served by English than by a script they do not use.
  if (/^zh(-(cn|sg|my|hans)(-|$)|$)/.test(tag)) return "zh-CN";
  return "en";
}

type I18n = {
  locale: Locale;
  setLocale: (locale: Locale) => void;
  t: (key: TranslationKey, values?: Record<string, string | number>) => string;
};

const I18nContext = createContext<I18n | null>(null);
const STORAGE_KEY = "waveflow.locale";

export function translate(
  locale: Locale,
  key: TranslationKey,
  values: Record<string, string | number> = {},
): string {
  const candidate: TranslationValue = LOCALES[locale].strings[key];
  // `one` is the form for exactly one, whatever a language's plural rules
  // call singular: French puts 0 there too, which would read "0 piste", and
  // Chinese puts nothing there, which would lose a form such as "mono".
  let value =
    typeof candidate === "string"
      ? candidate
      : candidate[values.count === 1 ? "one" : "other"];
  for (const [name, replacement] of Object.entries(values)) {
    value = value.replaceAll(`{${name}}`, String(replacement));
  }
  return value;
}

export function I18nProvider({ children }: { children: ReactNode }) {
  const [locale, setLocale] = useState<Locale>(() => {
    try {
      const saved = localStorage.getItem(STORAGE_KEY);
      if (isLocale(saved)) return saved;
    } catch {
      // Fall back to the browser language when storage is unavailable.
    }
    return localeFor(navigator.language);
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
        {Object.entries(LOCALES).map(([value, { name }]) => (
          <option key={value} value={value}>
            {name}
          </option>
        ))}
      </select>
    </label>
  );
}
