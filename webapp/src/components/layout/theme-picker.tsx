import { type CSSProperties, useId } from "react";
import { type TranslationKey, useI18n } from "../../state/i18n";
import { useThemePreference } from "../../state/preferences";
import {
  findTheme,
  THEME_PRESETS,
  type ThemeMode,
  type ThemePreset,
  themeSurfaces,
} from "../../styles/design-tokens";

const MODES: ReadonlyArray<{ mode: ThemeMode; label: TranslationKey }> = [
  { mode: "light", label: "preferences.themeLight" },
  { mode: "dark", label: "preferences.themeDark" },
];

/**
 * The cards in the order they are drawn, and so walked by the arrow keys: the
 * light row, then the dark one, each in the catalogue's order.
 */
const ORDER: ReadonlyArray<ThemePreset> = MODES.flatMap(({ mode }) =>
  THEME_PRESETS.filter((preset) => preset.mode === mode),
);

/**
 * One preset's miniature, as custom properties the card's stylesheet reads.
 *
 * Drawn from the preset's own tokens — its ground, panel and raised surface,
 * and its accent at the stops the material spends — so every card shows its
 * own theme whichever one is applied, the way the desktop's picker does.
 */
function previewStyle(preset: ThemePreset): CSSProperties {
  const surfaces = themeSurfaces(preset);
  const dark = preset.mode === "dark";
  const ink = dark ? "#f3f1ec" : "#1b1a17";
  return {
    "--preview-ground": surfaces.ground,
    // A dark preset's panel is the page's own `--fe-surface`, already a step
    // above the ground. A light preset's surface is often its ground itself,
    // so the miniature shades it to keep the sidebar visible.
    "--preview-sidebar": dark
      ? surfaces.surface
      : `color-mix(in srgb, ${surfaces.surface} 94%, ${ink})`,
    "--preview-raised": surfaces.raised,
    "--preview-ink": ink,
    "--preview-accent": dark ? preset.accent[400] : preset.accent[600],
    "--preview-accent-deep": dark ? preset.accent[600] : preset.accent[800],
  } as CSSProperties;
}

/**
 * The theme picker: a card per preset, under a Light and a Dark heading, and
 * the box that hands the choice between a preset and its pair to the device.
 *
 * **Native radios under the cards.** Each card is the label of a real
 * `<input type="radio">`, all fourteen under one name, so the browser does
 * what a radio group must and a hand-rolled one gets wrong somewhere: one
 * stop in the tab order (the checked card), arrow keys that move and select
 * across both rows and wrap, Space on a focused card, and the checked state
 * read out. The cards draw the state; the inputs carry it.
 *
 * **The checked card is the one on screen.** While following the device,
 * picking Midnight light at night checks Midnight: the pick chose the
 * family, and the device chose its member.
 */
export function ThemePicker() {
  const { t } = useI18n();
  const preferences = useThemePreference();
  const headingId = useId();
  const name = (preset: ThemePreset) => t(preset.labelKey as TranslationKey);

  const chosen = findTheme(preferences.chosenThemeId);
  const shown = findTheme(preferences.themeId);
  // A one-off with no pair (OLED, Neon) cannot follow the device across:
  // Emerald stands in, and the page says so instead of seeming to ignore the
  // pick.
  const standIn =
    preferences.followSystem &&
    shown.id !== chosen.id &&
    shown.id !== chosen.pair;

  return (
    <div className="theme-gallery">
      <h4 id={headingId} className="theme-gallery-heading">
        {t("preferences.theme")}
      </h4>
      {preferences.systemScheme !== null ? (
        <div className="theme-follow">
          <label className="toggle-row">
            <input
              type="checkbox"
              className="toggle-box"
              checked={preferences.followSystem}
              onChange={(event) =>
                preferences.setFollowSystem(event.target.checked)
              }
            />
            <span>{t("preferences.followSystem")}</span>
          </label>
          <p className="muted">
            {!preferences.followSystem
              ? t("preferences.followSystemDetail")
              : preferences.systemScheme === "dark"
                ? t("preferences.followingDark")
                : t("preferences.followingLight")}
          </p>
          {standIn ? (
            <p className="muted">
              {t(
                shown.mode === "light"
                  ? "preferences.standInLight"
                  : "preferences.standInDark",
                { theme: name(chosen), fallback: name(shown) },
              )}
            </p>
          ) : null}
        </div>
      ) : null}
      <div
        role="radiogroup"
        aria-labelledby={headingId}
        className="theme-groups"
      >
        {MODES.map(({ mode, label }) => {
          const groupHeadingId = `${headingId}-${mode}`;
          return (
            <fieldset
              key={mode}
              className="theme-group"
              aria-labelledby={groupHeadingId}
            >
              <h5 id={groupHeadingId} className="theme-group-heading">
                {t(label)}
              </h5>
              <div className="theme-grid">
                {ORDER.filter((preset) => preset.mode === mode).map(
                  (preset) => {
                    const checked = preset.id === preferences.themeId;
                    const nameId = `${headingId}-${preset.id}-name`;
                    const modeId = `${headingId}-${preset.id}-mode`;
                    return (
                      <label
                        key={preset.id}
                        className="theme-card"
                        data-theme-id={preset.id}
                        style={previewStyle(preset)}
                      >
                        <input
                          type="radio"
                          className="theme-card-input"
                          name={headingId}
                          value={preset.id}
                          checked={checked}
                          aria-labelledby={nameId}
                          aria-describedby={modeId}
                          onChange={() => preferences.setThemeId(preset.id)}
                        />
                        <span className="theme-card-preview" aria-hidden="true">
                          <span className="theme-card-sidebar">
                            <span className="theme-card-dot" />
                            <span className="theme-card-line" />
                            <span className="theme-card-line" />
                            <span className="theme-card-line" />
                          </span>
                          <span className="theme-card-content">
                            <span className="theme-card-bar" />
                            <span className="theme-card-tiles">
                              <span />
                              <span />
                              <span />
                            </span>
                          </span>
                          <span className="theme-card-player">
                            <span className="theme-card-dot" />
                            <span className="theme-card-progress" />
                          </span>
                        </span>
                        <span id={nameId} className="theme-card-name">
                          {name(preset)}
                        </span>
                        <span id={modeId} className="theme-card-mode">
                          {t(label)}
                        </span>
                        {checked ? (
                          <span className="theme-card-check">
                            <svg
                              aria-hidden="true"
                              viewBox="0 0 16 16"
                              width="12"
                              height="12"
                            >
                              <path
                                d="M3.5 8.5 6.5 11.5 12.5 4.5"
                                fill="none"
                                stroke="currentColor"
                                strokeWidth="2"
                                strokeLinecap="round"
                                strokeLinejoin="round"
                              />
                            </svg>
                          </span>
                        ) : null}
                      </label>
                    );
                  },
                )}
              </div>
            </fieldset>
          );
        })}
      </div>
    </div>
  );
}
