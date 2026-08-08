// 14 theme presets — 6 light + 8 dark. Each preset re-skins the
// app by remapping the accent palette + ambient surface tints; the
// shape mirrors the desktop's `ThemePreset` interface so a JSON
// dump of the desktop's stored preference can hydrate the web side
// without translation.
//
// Adding a preset is a one-stop process: add it to THEME_PRESETS,
// optionally point `pair` at its opposite-mode mirror, and the
// Settings picker discovers it.

import {
  type AccentPalette,
  AMBER,
  EMERALD,
  FUCHSIA,
  INDIGO,
  ROSE,
  SKY,
  VIOLET,
} from "./palettes";

export type ThemeMode = "light" | "dark";

export interface ThemePreset {
  /** Stable id used as the persisted value AND as `data-theme=` on the root. */
  id: string;
  /**
   * i18n key suffix routed through whatever translation layer the
   * consuming app uses. The web app currently ships English-only;
   * the desktop ports to fr/en/es/.../17 locales via i18next. The
   * field carries the suffix only so neither side has to commit to
   * a key naming scheme that doesn't fit its convention.
   */
  labelKey: string;
  /** Whether the root should carry the legacy `dark` class. */
  mode: ThemeMode;
  /** 50→950 accent scale — drives the `bg-emerald-*` utility rebind. */
  accent: AccentPalette;
  /** Page-body ambient tint. `null` keeps the Tailwind default. */
  ambient?: string | null;
  /** Override for `--color-surface-dark` (sidebar / right panels). */
  surfaceDark?: string | null;
  /** Override for `--color-surface-dark-elevated` (PlayerBar / cards). */
  surfaceDarkElevated?: string | null;
  /** Light-mode mirror of `surfaceDark` (sidebar / right panels in light themes). */
  surfaceLight?: string | null;
  /** Light-mode mirror of `surfaceDarkElevated`. */
  surfaceLightElevated?: string | null;
  /**
   * Id of the opposite-mode counterpart — drives the sun/moon
   * binary toggle so a click on `lavender` (dark) flips to
   * `lavender-light` rather than resetting to default Emerald.
   * Leave undefined for one-off presets (OLED, Neon) — the toggle
   * falls back to the global default pair in that case.
   */
  pair?: string;
}

/**
 * Theme catalogue organised in two visual rows for the Settings
 * picker. Order matters: it's the order the grid renders.
 *
 *   Row 1 (light, 6) — Emerald · Midnight · Sunset · Lavender · Crimson · Ocean
 *   Row 2 (dark, 8)  — Emerald · OLED · Midnight · Sunset · Lavender · Crimson · Ocean · Neon
 */
export const THEME_PRESETS: ThemePreset[] = [
  // ── Light row ───────────────────────────────────────────────────
  {
    id: "default",
    labelKey: "appearance.themes.default",
    mode: "light",
    accent: EMERALD,
    ambient: null,
    pair: "default-dark",
  },
  {
    id: "midnight-light",
    labelKey: "appearance.themes.midnightLight",
    mode: "light",
    accent: INDIGO,
    ambient: "#e0e7ff",
    surfaceLight: "#e0e7ff",
    pair: "midnight",
  },
  {
    id: "sunset-light",
    labelKey: "appearance.themes.sunsetLight",
    mode: "light",
    accent: AMBER,
    ambient: "#fef3c7",
    surfaceLight: "#fef3c7",
    pair: "sunset",
  },
  {
    id: "lavender-light",
    labelKey: "appearance.themes.lavenderLight",
    mode: "light",
    accent: VIOLET,
    ambient: "#ede9fe",
    surfaceLight: "#ede9fe",
    pair: "lavender",
  },
  {
    id: "crimson-light",
    labelKey: "appearance.themes.crimsonLight",
    mode: "light",
    accent: ROSE,
    ambient: "#ffe4e6",
    surfaceLight: "#ffe4e6",
    pair: "crimson",
  },
  {
    id: "ocean-light",
    labelKey: "appearance.themes.oceanLight",
    mode: "light",
    accent: SKY,
    ambient: "#e0f2fe",
    surfaceLight: "#e0f2fe",
    pair: "ocean",
  },
  // ── Dark row ────────────────────────────────────────────────────
  {
    id: "default-dark",
    labelKey: "appearance.themes.defaultDark",
    mode: "dark",
    accent: EMERALD,
    ambient: null,
    pair: "default",
  },
  {
    id: "oled",
    labelKey: "appearance.themes.oled",
    mode: "dark",
    accent: EMERALD,
    ambient: "#000000",
    // True black canvas — keep surface pitch black, elevate by the
    // smallest perceivable step so cards still read above the body.
    surfaceDark: "#000000",
    surfaceDarkElevated: "#0a0a0a",
  },
  {
    id: "midnight",
    labelKey: "appearance.themes.midnight",
    mode: "dark",
    accent: INDIGO,
    ambient: "#0b1020",
    surfaceDark: "#0b1020",
    surfaceDarkElevated: "#141a2e",
    pair: "midnight-light",
  },
  {
    id: "sunset",
    labelKey: "appearance.themes.sunset",
    mode: "dark",
    accent: AMBER,
    ambient: "#1a0e08",
    surfaceDark: "#1a0e08",
    surfaceDarkElevated: "#241612",
    pair: "sunset-light",
  },
  {
    id: "lavender",
    labelKey: "appearance.themes.lavender",
    mode: "dark",
    accent: VIOLET,
    ambient: "#15101e",
    surfaceDark: "#15101e",
    surfaceDarkElevated: "#1f1828",
    pair: "lavender-light",
  },
  {
    id: "crimson",
    labelKey: "appearance.themes.crimson",
    mode: "dark",
    accent: ROSE,
    ambient: "#19090c",
    surfaceDark: "#19090c",
    surfaceDarkElevated: "#241016",
    pair: "crimson-light",
  },
  {
    id: "ocean",
    labelKey: "appearance.themes.ocean",
    mode: "dark",
    accent: SKY,
    ambient: "#081420",
    surfaceDark: "#081420",
    surfaceDarkElevated: "#0f1c30",
    pair: "ocean-light",
  },
  {
    id: "neon",
    labelKey: "appearance.themes.neon",
    mode: "dark",
    accent: FUCHSIA,
    ambient: "#1a0a18",
    surfaceDark: "#1a0a18",
    surfaceDarkElevated: "#231022",
  },
];

export const DEFAULT_THEME_ID = "default-dark";

/**
 * Locate a preset by id; falls back to `default-dark` when the id is
 * absent or unknown. Pure read — never throws, never logs (a stored
 * preference referring to a removed theme would otherwise crash on
 * first paint, which is the wrong place to surface a config bug).
 */
export function findTheme(id: string | null | undefined): ThemePreset {
  const def = THEME_PRESETS.find((t) => t.id === DEFAULT_THEME_ID);
  if (!def) {
    // Indicates THEME_PRESETS was edited without keeping the default
    // around — a programming error, surfaced loud rather than silent.
    throw new Error(
      `design-tokens: default theme "${DEFAULT_THEME_ID}" missing from THEME_PRESETS`,
    );
  }
  if (!id) return def;
  return THEME_PRESETS.find((t) => t.id === id) ?? def;
}
