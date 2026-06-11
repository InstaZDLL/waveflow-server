// OKLCH accent palettes — one 50→950 scale per accent family.
// Ported verbatim from the WaveFlow desktop's `src/lib/themes.ts`
// so a Lavender preset on the desktop and on the web tints to the
// exact same violet. Don't drift these without porting on both
// sides; the desktop is the source of truth and lives in the same
// owner so cross-repo coupling is intentional.
//
// Each palette is the matching Tailwind v4 emerald/indigo/etc.
// scale extracted from `node_modules/tailwindcss/preflight` at the
// time of the desktop's `Theme system` ship. Stored as OKLCH
// strings rather than hex so a future Wide-Gamut surface (Display
// P3, Apple display) gets the actual saturated tone instead of an
// sRGB-clipped fallback.

export type AccentShade =
  | '50'
  | '100'
  | '200'
  | '300'
  | '400'
  | '500'
  | '600'
  | '700'
  | '800'
  | '900'
  | '950'

export type AccentPalette = Record<AccentShade, string>

export const EMERALD: AccentPalette = {
  50: 'oklch(0.979 0.021 166.113)',
  100: 'oklch(0.95 0.052 163.051)',
  200: 'oklch(0.905 0.093 164.15)',
  300: 'oklch(0.845 0.143 164.978)',
  400: 'oklch(0.765 0.177 163.223)',
  500: 'oklch(0.696 0.17 162.48)',
  600: 'oklch(0.596 0.145 163.225)',
  700: 'oklch(0.508 0.118 165.612)',
  800: 'oklch(0.432 0.095 166.913)',
  900: 'oklch(0.378 0.077 168.94)',
  950: 'oklch(0.262 0.051 172.552)',
}

export const INDIGO: AccentPalette = {
  50: 'oklch(0.962 0.018 272.314)',
  100: 'oklch(0.93 0.034 272.788)',
  200: 'oklch(0.87 0.065 274.039)',
  300: 'oklch(0.785 0.115 274.713)',
  400: 'oklch(0.673 0.182 276.935)',
  500: 'oklch(0.585 0.233 277.117)',
  600: 'oklch(0.511 0.262 276.966)',
  700: 'oklch(0.457 0.24 277.023)',
  800: 'oklch(0.398 0.195 277.366)',
  900: 'oklch(0.359 0.144 278.697)',
  950: 'oklch(0.257 0.09 281.288)',
}

export const VIOLET: AccentPalette = {
  50: 'oklch(0.969 0.016 293.756)',
  100: 'oklch(0.943 0.029 294.588)',
  200: 'oklch(0.894 0.057 293.283)',
  300: 'oklch(0.811 0.111 293.571)',
  400: 'oklch(0.702 0.183 293.541)',
  500: 'oklch(0.606 0.25 292.717)',
  600: 'oklch(0.541 0.281 293.009)',
  700: 'oklch(0.491 0.27 292.581)',
  800: 'oklch(0.432 0.232 292.759)',
  900: 'oklch(0.38 0.189 293.745)',
  950: 'oklch(0.283 0.141 291.089)',
}

export const ROSE: AccentPalette = {
  50: 'oklch(0.969 0.015 12.422)',
  100: 'oklch(0.941 0.03 12.58)',
  200: 'oklch(0.892 0.058 10.001)',
  300: 'oklch(0.81 0.117 11.638)',
  400: 'oklch(0.712 0.194 13.428)',
  500: 'oklch(0.645 0.246 16.439)',
  600: 'oklch(0.586 0.253 17.585)',
  700: 'oklch(0.514 0.222 16.935)',
  800: 'oklch(0.455 0.188 13.697)',
  900: 'oklch(0.41 0.159 10.272)',
  950: 'oklch(0.271 0.105 12.094)',
}

export const AMBER: AccentPalette = {
  50: 'oklch(0.987 0.022 95.277)',
  100: 'oklch(0.962 0.059 95.617)',
  200: 'oklch(0.924 0.12 95.746)',
  300: 'oklch(0.879 0.169 91.605)',
  400: 'oklch(0.828 0.189 84.429)',
  500: 'oklch(0.769 0.188 70.08)',
  600: 'oklch(0.666 0.179 58.318)',
  700: 'oklch(0.555 0.163 48.998)',
  800: 'oklch(0.473 0.137 46.201)',
  900: 'oklch(0.414 0.112 45.904)',
  950: 'oklch(0.279 0.077 45.635)',
}

export const SKY: AccentPalette = {
  50: 'oklch(0.977 0.013 236.62)',
  100: 'oklch(0.951 0.026 236.824)',
  200: 'oklch(0.901 0.058 230.902)',
  300: 'oklch(0.828 0.111 230.318)',
  400: 'oklch(0.746 0.16 232.661)',
  500: 'oklch(0.685 0.169 237.323)',
  600: 'oklch(0.588 0.158 241.966)',
  700: 'oklch(0.5 0.134 242.749)',
  800: 'oklch(0.443 0.11 240.79)',
  900: 'oklch(0.391 0.09 240.876)',
  950: 'oklch(0.293 0.066 243.157)',
}

export const FUCHSIA: AccentPalette = {
  50: 'oklch(0.977 0.017 320.058)',
  100: 'oklch(0.952 0.037 318.852)',
  200: 'oklch(0.903 0.076 319.62)',
  300: 'oklch(0.833 0.145 321.434)',
  400: 'oklch(0.74 0.238 322.16)',
  500: 'oklch(0.667 0.295 322.15)',
  600: 'oklch(0.591 0.293 322.896)',
  700: 'oklch(0.518 0.253 323.949)',
  800: 'oklch(0.452 0.211 324.591)',
  900: 'oklch(0.401 0.17 325.612)',
  950: 'oklch(0.293 0.136 325.661)',
}

/**
 * The seven accent families the 14 theme presets pick from. Exposed
 * as a named map so consumers (Settings picker, theme JSON export,
 * embed widgets) can iterate without having to know the named binds.
 */
export const ACCENT_PALETTES = {
  EMERALD,
  INDIGO,
  VIOLET,
  ROSE,
  AMBER,
  SKY,
  FUCHSIA,
} as const

export type AccentPaletteName = keyof typeof ACCENT_PALETTES
