import {
  createContext,
  type ReactNode,
  useContext,
  useEffect,
  useState,
} from "react";

import { applyTheme, findTheme, THEME_PRESETS } from "./design-tokens";
import { useI18n } from "./i18n";

const STORAGE_KEY = "waveflow.theme";

type Preferences = {
  themeId: string;
  setThemeId: (id: string) => void;
};

const PreferencesContext = createContext<Preferences | null>(null);

export function PreferencesProvider({ children }: { children: ReactNode }) {
  const [themeId, setThemeId] = useState(() => {
    try {
      return findTheme(localStorage.getItem(STORAGE_KEY)).id;
    } catch {
      return findTheme(null).id;
    }
  });

  useEffect(() => {
    const theme = findTheme(themeId);
    applyTheme(theme);
    try {
      localStorage.setItem(STORAGE_KEY, theme.id);
    } catch {
      // Appearance remains usable when persistent storage is disabled.
    }
  }, [themeId]);

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
            {theme.id.replaceAll("-", " ")}
          </option>
        ))}
      </select>
    </label>
  );
}
