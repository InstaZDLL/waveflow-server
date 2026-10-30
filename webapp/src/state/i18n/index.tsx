import {
  createContext,
  type ReactNode,
  useCallback,
  useContext,
  useEffect,
  useRef,
  useState,
} from "react";

import { en } from "./en";

export type TranslationKey = keyof typeof en;
export type TranslationValue = string | { one: string; other: string };
type Strings = Record<TranslationKey, TranslationValue>;

// Every language the client speaks, with the name the picker shows for it,
// written in that language, and how to fetch its strings. English is bundled;
// every other language is a chunk of its own, fetched when a reader chooses it
// or the browser asks for it, so nobody downloads the languages they do not
// read. A new one is a directory of strings typed against the English domains,
// and a line here.
const LOCALES = {
  en: { name: "English", load: async () => en },
  fr: { name: "Français", load: async () => (await import("./fr")).fr },
  de: { name: "Deutsch", load: async () => (await import("./de")).de },
  es: { name: "Español", load: async () => (await import("./es")).es },
  it: { name: "Italiano", load: async () => (await import("./it")).it },
  ja: { name: "日本語", load: async () => (await import("./ja")).ja },
  ko: { name: "한국어", load: async () => (await import("./ko")).ko },
  "zh-CN": {
    name: "简体中文",
    load: async () => (await import("./zh-CN")).zhCN,
  },
  "zh-TW": {
    name: "繁體中文",
    load: async () => (await import("./zh-TW")).zhTW,
  },
} satisfies Record<string, { name: string; load: () => Promise<Strings> }>;

export type Locale = keyof typeof LOCALES;

export const locales = Object.keys(LOCALES) as Locale[];

// The strings of every language fetched so far. English is always among them:
// it is what `translate` answers in for a language whose strings have not
// arrived.
const loaded = new Map<Locale, Strings>([["en", en]]);
const loading = new Map<Locale, Promise<void>>();

/**
 * Fetches a language's strings, once; `translate` answers in it from then on.
 * A fetch that fails is forgotten, so the next attempt fetches again.
 */
export function loadLocale(locale: Locale): Promise<void> {
  if (loaded.has(locale)) return Promise.resolve();
  let pending = loading.get(locale);
  if (!pending) {
    pending = LOCALES[locale].load().then(
      (strings) => {
        loaded.set(locale, strings);
      },
      (error: unknown) => {
        loading.delete(locale);
        throw error;
      },
    );
    loading.set(locale, pending);
  }
  return pending;
}

function isLocale(value: string | null): value is Locale {
  return value !== null && Object.hasOwn(LOCALES, value);
}

/** The language to start in, from the one the browser prefers. */
export function localeFor(language: string): Locale {
  const [primary, ...subtags] = language.toLowerCase().split("-");
  if (primary === "zh") {
    // The script decides when the tag names one; otherwise the region does,
    // Taiwan, Hong Kong and Macao reading Traditional and the rest Simplified.
    if (subtags.includes("hant")) return "zh-TW";
    if (subtags.includes("hans")) return "zh-CN";
    return subtags.some((subtag) => ["tw", "hk", "mo"].includes(subtag))
      ? "zh-TW"
      : "zh-CN";
  }
  return isLocale(primary) ? primary : "en";
}

type I18n = {
  locale: Locale;
  setLocale: (locale: Locale) => void;
  t: (key: TranslationKey, values?: Record<string, string | number>) => string;
};

const I18nContext = createContext<I18n | null>(null);
const STORAGE_KEY = "waveflow.locale";

/** A string in `locale`, or in English while that language's are on the way. */
export function translate(
  locale: Locale,
  key: TranslationKey,
  values: Record<string, string | number> = {},
): string {
  const candidate: TranslationValue = (loaded.get(locale) ?? en)[key];
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

function startingLocale(): Locale {
  try {
    const saved = localStorage.getItem(STORAGE_KEY);
    if (isLocale(saved)) return saved;
  } catch {
    // Fall back to the browser language when storage is unavailable.
  }
  return localeFor(navigator.language);
}

export function I18nProvider({ children }: { children: ReactNode }) {
  const [locale, setCurrent] = useState(startingLocale);
  // Nothing renders until the starting language's strings arrive: a moment of
  // nothing reads better than a flash of English replaced by the reader's own.
  // A fetch that fails renders all the same, in English.
  const [ready, setReady] = useState(() => loaded.has(locale));
  const wanted = useRef(locale);

  useEffect(() => {
    if (ready) return;
    const done = () => setReady(true);
    loadLocale(locale).then(done, done);
  }, [ready, locale]);

  // The language on screen stays until the chosen one's strings arrive, so a
  // switch never passes through English, and only the last choice applies.
  const setLocale = useCallback((next: Locale) => {
    wanted.current = next;
    if (loaded.has(next)) {
      setCurrent(next);
      return;
    }
    loadLocale(next).then(
      () => {
        if (wanted.current === next) setCurrent(next);
      },
      () => {
        // Offline before this language was ever fetched: keep the one shown.
      },
    );
  }, []);

  useEffect(() => {
    if (!ready) return;
    document.documentElement.lang = loaded.has(locale) ? locale : "en";
    try {
      localStorage.setItem(STORAGE_KEY, locale);
    } catch {
      // Language switching remains available without persistence.
    }
  }, [ready, locale]);

  const t: I18n["t"] = useCallback(
    (key, values = {}) => translate(locale, key, values),
    [locale],
  );

  return (
    <I18nContext.Provider value={{ locale, setLocale, t }}>
      {ready ? children : null}
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
