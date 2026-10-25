import {
  createContext,
  type ReactNode,
  useCallback,
  useContext,
  useEffect,
  useId,
  useState,
} from "react";
import {
  clampPreamp,
  PREAMP_MAX_DB,
  PREAMP_MIN_DB,
  parseReplayGainPreference,
  REPLAY_GAIN_MODES,
  type ReplayGainMode,
  type ReplayGainPreference,
} from "../lib/replay-gain";
import {
  applyTheme,
  findTheme,
  THEME_PRESETS,
  type ThemePreset,
} from "../styles/design-tokens";
import { type TranslationKey, useI18n } from "./i18n";

const STORAGE_KEY = "waveflow.theme";
/**
 * Per browser, like the volume and the theme: how loud a device should be is
 * not a property of the account (RFC-022 decision 1).
 */
const REPLAY_GAIN_KEY = "waveflow.replayGain";

type Preferences = {
  themeId: string;
  setThemeId: (id: string) => void;
  replayGain: ReplayGainPreference;
  setReplayGain: (preference: ReplayGainPreference) => void;
};

const PreferencesContext = createContext<Preferences | null>(null);

function readStoredReplayGain(): ReplayGainPreference {
  try {
    return parseReplayGainPreference(localStorage.getItem(REPLAY_GAIN_KEY));
  } catch {
    // Blocked site data: the default, which is off.
    return parseReplayGainPreference(null);
  }
}

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
  const [replayGain, setReplayGainState] = useState(readStoredReplayGain);

  useEffect(() => {
    try {
      localStorage.setItem(STORAGE_KEY, themeId);
    } catch {
      // Appearance remains usable when persistent storage is disabled.
    }
  }, [themeId]);

  useEffect(() => {
    try {
      localStorage.setItem(REPLAY_GAIN_KEY, JSON.stringify(replayGain));
    } catch {
      // The level still applies for this session without persistence.
    }
  }, [replayGain]);

  const setThemeId = useCallback((id: string) => {
    const theme = findTheme(id);
    applyTheme(theme);
    setStoredThemeId(theme.id);
  }, []);

  const setReplayGain = useCallback((preference: ReplayGainPreference) => {
    setReplayGainState({
      mode: preference.mode,
      preampDb: clampPreamp(preference.preampDb),
    });
  }, []);

  return (
    <PreferencesContext.Provider
      value={{ themeId, setThemeId, replayGain, setReplayGain }}
    >
      {children}
    </PreferencesContext.Provider>
  );
}

/** The ReplayGain preference, for the player that applies it. */
export function useReplayGainPreference(): ReplayGainPreference {
  const preferences = useContext(PreferencesContext);
  if (!preferences)
    throw new Error("useReplayGainPreference requires PreferencesProvider");
  return preferences.replayGain;
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

const MODE_LABELS: Record<ReplayGainMode, TranslationKey> = {
  off: "replayGain.off",
  track: "replayGain.track",
  album: "replayGain.album",
};

/** A preamp as a reader writes it: signed, in decibels. */
function formatPreamp(decibels: number): string {
  return `${decibels > 0 ? "+" : decibels < 0 ? "−" : "±"}${Math.abs(decibels)} dB`;
}

/**
 * ReplayGain: whether, and by which measurement, tracks are levelled.
 *
 * The preamp is offered only while a mode is on, since it changes nothing
 * otherwise. Off by default; the note says what turning it on costs, because
 * routing playback through Web Audio is not free on every phone.
 */
export function ReplayGainPicker() {
  const preferences = useContext(PreferencesContext);
  const { t } = useI18n();
  const preampId = useId();
  if (!preferences)
    throw new Error("ReplayGainPicker requires PreferencesProvider");
  const { replayGain, setReplayGain } = preferences;
  return (
    <div className="replay-gain-picker">
      <label className="theme-picker">
        <span>{t("replayGain.label")}</span>
        <select
          aria-label={t("replayGain.label")}
          value={replayGain.mode}
          onChange={(event) =>
            setReplayGain({
              ...replayGain,
              mode: event.target.value as ReplayGainMode,
            })
          }
        >
          {REPLAY_GAIN_MODES.map((mode) => (
            <option key={mode} value={mode}>
              {t(MODE_LABELS[mode])}
            </option>
          ))}
        </select>
      </label>
      {replayGain.mode !== "off" ? (
        <div className="replay-gain-preamp">
          {/* The reading sits beside the label rather than inside it: inside,
              it would become part of the slider's accessible name. */}
          <div className="replay-gain-preamp-heading">
            <label htmlFor={preampId}>{t("replayGain.preamp")}</label>
            <output htmlFor={preampId}>
              {formatPreamp(replayGain.preampDb)}
            </output>
          </div>
          <input
            id={preampId}
            type="range"
            aria-valuetext={formatPreamp(replayGain.preampDb)}
            min={PREAMP_MIN_DB}
            max={PREAMP_MAX_DB}
            step={1}
            value={replayGain.preampDb}
            onChange={(event) =>
              setReplayGain({
                ...replayGain,
                preampDb: Number(event.target.value),
              })
            }
          />
        </div>
      ) : null}
      <p className="muted">{t("replayGain.detail")}</p>
    </div>
  );
}
