// ThemePicker — 14-tile grid (6 light + 8 dark) that swaps the
// active theme on click. The active tile carries a check overlay;
// keyboard activation is built in via native <button>. Each tile
// renders a 4-shade swatch (200 / 400 / 600 / 800) so the user
// can tell Lavender from Crimson before clicking.
//
// The picker takes no props beyond `t(...)` for label translation —
// it pulls THEME_PRESETS from the package and routes selection
// through the ThemeProvider's `setTheme`, so a new preset added
// to the package's roster shows up automatically.

import { THEME_PRESETS, type ThemePreset } from '@waveflow/design-tokens'

import { useTheme } from './ThemeProvider'

interface SwatchRowProps {
  accent: ThemePreset['accent']
}

function SwatchRow({ accent }: SwatchRowProps) {
  // Four evenly-spaced shades — the visual fingerprint of the
  // palette. Pulling the same shades for every preset keeps the
  // grid visually consistent (no jumping between 3 / 4 / 5 swatch
  // counts as palette densities differ).
  const shades: Array<keyof typeof accent> = ['200', '400', '600', '800']
  return (
    <div className="flex gap-1" aria-hidden="true">
      {shades.map((shade) => (
        <span
          key={shade}
          className="h-3 w-3 rounded-full ring-1 ring-black/10 dark:ring-white/10"
          style={{ backgroundColor: accent[shade] }}
        />
      ))}
    </div>
  )
}

export interface ThemePickerProps {
  /**
   * Label resolver. Receives the preset's `labelKey` (e.g.
   * `appearance.themes.lavender`) and returns the localised
   * display name. Default = strip the prefix + title-case, so the
   * picker still renders something coherent before i18n is wired.
   */
  resolveLabel?: (labelKey: string) => string
}

function defaultResolveLabel(labelKey: string): string {
  const tail = labelKey.split('.').pop() ?? labelKey
  // camelCase -> "Camel Case"
  return tail
    .replace(/([A-Z])/g, ' $1')
    .replace(/^./, (c) => c.toUpperCase())
    .trim()
}

export function ThemePicker({ resolveLabel = defaultResolveLabel }: ThemePickerProps) {
  const { theme, setTheme } = useTheme()
  const lightPresets = THEME_PRESETS.filter((p) => p.mode === 'light')
  const darkPresets = THEME_PRESETS.filter((p) => p.mode === 'dark')

  return (
    <div className="flex flex-col gap-6">
      <ThemeRow
        heading="Light"
        presets={lightPresets}
        activeId={theme.id}
        onSelect={setTheme}
        resolveLabel={resolveLabel}
      />
      <ThemeRow
        heading="Dark"
        presets={darkPresets}
        activeId={theme.id}
        onSelect={setTheme}
        resolveLabel={resolveLabel}
      />
    </div>
  )
}

interface ThemeRowProps {
  heading: string
  presets: ThemePreset[]
  activeId: string
  onSelect: (id: string) => void
  resolveLabel: (labelKey: string) => string
}

function ThemeRow({ heading, presets, activeId, onSelect, resolveLabel }: ThemeRowProps) {
  return (
    <section className="flex flex-col gap-2">
      <h3 className="text-xs font-semibold uppercase tracking-wider text-[var(--sea-ink-soft)]">
        {heading}
      </h3>
      <div
        role="radiogroup"
        aria-label={`${heading} themes`}
        className="grid grid-cols-2 gap-2 sm:grid-cols-3 lg:grid-cols-4"
      >
        {presets.map((preset) => {
          const isActive = preset.id === activeId
          return (
            <button
              key={preset.id}
              type="button"
              role="radio"
              aria-checked={isActive}
              onClick={() => onSelect(preset.id)}
              className={`group flex items-center gap-3 rounded-xl border px-3 py-2.5 text-left transition ${
                isActive
                  ? 'border-[var(--sea-ink)] bg-white/80 shadow-sm dark:bg-black/30'
                  : 'border-[var(--line)] bg-white/40 hover:border-[var(--sea-ink)]/40 hover:bg-white/60 dark:bg-black/15 dark:hover:bg-black/25'
              }`}
            >
              <SwatchRow accent={preset.accent} />
              <span className="flex-1 truncate text-sm font-medium text-[var(--sea-ink)]">
                {resolveLabel(preset.labelKey)}
              </span>
              {isActive && (
                <span aria-hidden="true" className="text-xs font-semibold text-[var(--sea-ink)]">
                  ✓
                </span>
              )}
            </button>
          )
        })}
      </div>
    </section>
  )
}
