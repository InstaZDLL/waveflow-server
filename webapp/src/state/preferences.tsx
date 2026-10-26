import {
  createContext,
  type ReactNode,
  useCallback,
  useContext,
  useEffect,
  useId,
  useLayoutEffect,
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
  type ThemeMode,
  type ThemePreset,
  themeForScheme,
} from "../styles/design-tokens";
import { type TranslationKey, useI18n } from "./i18n";

const STORAGE_KEY = "waveflow.theme";
/**
 * Per browser, beside the theme it qualifies: whether the device's light or
 * dark setting chooses between a preset and its pair. A phone and a desktop
 * on one account can disagree about it, as they can about the theme.
 */
const FOLLOW_SYSTEM_KEY = "waveflow.themeFollowSystem";
const DARK_SCHEME = "(prefers-color-scheme: dark)";
/**
 * Per browser, like the volume and the theme: how loud a device should be is
 * not a property of the account (RFC-022 decision 1).
 */
const REPLAY_GAIN_KEY = "waveflow.replayGain";

type Preferences = {
  /** The preset on screen: the chosen one, or its pair while following. */
  themeId: string;
  /** The preset last picked, which is what is stored. */
  chosenThemeId: string;
  setThemeId: (id: string) => void;
  followSystem: boolean;
  setFollowSystem: (follow: boolean) => void;
  /** The device's scheme, or null where the browser cannot say. */
  systemScheme: ThemeMode | null;
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

function readStoredTheme(): ThemePreset {
  try {
    return findTheme(localStorage.getItem(STORAGE_KEY));
  } catch {
    return findTheme(null);
  }
}

/** Off unless this browser turned it on: a first visit keeps the default. */
function readStoredFollowSystem(): boolean {
  try {
    return localStorage.getItem(FOLLOW_SYSTEM_KEY) === "true";
  } catch {
    return false;
  }
}

/** The device's scheme query, where the browser has one. */
function schemeQuery(): MediaQueryList | null {
  if (typeof window === "undefined" || typeof window.matchMedia !== "function")
    return null;
  try {
    return window.matchMedia(DARK_SCHEME);
  } catch {
    return null;
  }
}

function readSystemScheme(): ThemeMode | null {
  const query = schemeQuery();
  return query ? (query.matches ? "dark" : "light") : null;
}

/**
 * The preset to paint: the chosen one, or — while following a scheme the
 * browser can report — the member of its family that suits that scheme.
 */
function resolveTheme(
  chosenId: string,
  followSystem: boolean,
  scheme: ThemeMode | null,
): ThemePreset {
  const chosen = findTheme(chosenId);
  return followSystem && scheme ? themeForScheme(chosen, scheme) : chosen;
}

export function PreferencesProvider({ children }: { children: ReactNode }) {
  const [chosenThemeId, setChosenThemeId] = useState(
    () => readStoredTheme().id,
  );
  const [followSystem, setFollowSystemState] = useState(readStoredFollowSystem);
  const [systemScheme, setSystemScheme] = useState(readSystemScheme);
  const [replayGain, setReplayGainState] = useState(readStoredReplayGain);

  const theme = resolveTheme(chosenThemeId, followSystem, systemScheme);
  // Painted during the first render rather than after it, so the page never
  // shows one frame of the default before the stored theme; then again
  // whenever the choice, the box or the device changes it.
  useState(() => applyTheme(theme));
  useLayoutEffect(() => applyTheme(theme), [theme]);

  // Live: a device that turns dark at sunset takes the page with it. Heard
  // only while following, since nothing else reads the scheme.
  useEffect(() => {
    if (!followSystem) return;
    const query = schemeQuery();
    if (!query) return;
    const update = () => setSystemScheme(query.matches ? "dark" : "light");
    update();
    query.addEventListener("change", update);
    return () => query.removeEventListener("change", update);
  }, [followSystem]);

  useEffect(() => {
    try {
      localStorage.setItem(STORAGE_KEY, chosenThemeId);
    } catch {
      // Appearance remains usable when persistent storage is disabled.
    }
  }, [chosenThemeId]);

  useEffect(() => {
    try {
      localStorage.setItem(FOLLOW_SYSTEM_KEY, String(followSystem));
    } catch {
      // Following still works for this session without persistence.
    }
  }, [followSystem]);

  useEffect(() => {
    try {
      localStorage.setItem(REPLAY_GAIN_KEY, JSON.stringify(replayGain));
    } catch {
      // The level still applies for this session without persistence.
    }
  }, [replayGain]);

  const setThemeId = useCallback((id: string) => {
    setChosenThemeId(findTheme(id).id);
  }, []);

  const setFollowSystem = useCallback(
    (follow: boolean) => {
      // Turning it off keeps what is on screen: the page should not jump back
      // to the other half of the family the moment the box is cleared.
      if (!follow) setChosenThemeId(theme.id);
      else setSystemScheme(readSystemScheme());
      setFollowSystemState(follow);
    },
    [theme],
  );

  const setReplayGain = useCallback((preference: ReplayGainPreference) => {
    setReplayGainState({
      mode: preference.mode,
      preampDb: clampPreamp(preference.preampDb),
    });
  }, []);

  return (
    <PreferencesContext.Provider
      value={{
        themeId: theme.id,
        chosenThemeId,
        setThemeId,
        followSystem,
        setFollowSystem,
        systemScheme,
        replayGain,
        setReplayGain,
      }}
    >
      {children}
    </PreferencesContext.Provider>
  );
}

/** The theme preference, for the picker that sets it. */
export function useThemePreference(): Pick<
  Preferences,
  | "themeId"
  | "chosenThemeId"
  | "setThemeId"
  | "followSystem"
  | "setFollowSystem"
  | "systemScheme"
> {
  const preferences = useContext(PreferencesContext);
  if (!preferences)
    throw new Error("useThemePreference requires PreferencesProvider");
  return preferences;
}

/** The ReplayGain preference, for the player that applies it. */
export function useReplayGainPreference(): ReplayGainPreference {
  const preferences = useContext(PreferencesContext);
  if (!preferences)
    throw new Error("useReplayGainPreference requires PreferencesProvider");
  return preferences.replayGain;
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
