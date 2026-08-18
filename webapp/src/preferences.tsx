import {
  createContext,
  type ReactNode,
  useCallback,
  useContext,
  useEffect,
  useState,
} from "react";

import {
  applyTheme,
  findTheme,
  THEME_PRESETS,
  type ThemePreset,
} from "./design-tokens";
import { type TranslationKey, useI18n } from "./i18n";

const STORAGE_KEY = "waveflow.theme";

type Preferences = {
  themeId: string;
  setThemeId: (id: string) => void;
};

const PreferencesContext = createContext<Preferences | null>(null);

export function PreferencesProvider({ children }: { children: ReactNode }) {
  const [themeId, setStoredThemeId] = useState(() => {
    let theme: ThemePreset;
    try {
      theme = findTheme(localStorage.getItem(STORAGE_KEY));
    } catch {
      theme = findTheme(null);
    }
    applyTheme(theme);
    return theme.id;
  });

  useEffect(() => {
    try {
      localStorage.setItem(STORAGE_KEY, themeId);
    } catch {
      // Appearance remains usable when persistent storage is disabled.
    }
  }, [themeId]);

  const setThemeId = useCallback((id: string) => {
    const theme = findTheme(id);
    applyTheme(theme);
    setStoredThemeId(theme.id);
  }, []);

  return (
    <PreferencesContext.Provider value={{ themeId, setThemeId }}>
      {children}
    </PreferencesContext.Provider>
  );
}

export function ThemePicker() {
  const preferences = useContext(PreferencesContext);
  const { t } = useI18n();
  if (!preferences) throw new Error("ThemePicker requires PreferencesProvider");
  return (
    <label className="theme-picker">
      <span>{t("preferences.theme")}</span>
      <select
        aria-label={t("preferences.theme")}
        value={preferences.themeId}
        onChange={(event) => preferences.setThemeId(event.target.value)}
      >
        {THEME_PRESETS.map((theme) => (
          <option key={theme.id} value={theme.id}>
            {t(theme.labelKey as TranslationKey)}
          </option>
        ))}
      </select>
    </label>
  );
}
