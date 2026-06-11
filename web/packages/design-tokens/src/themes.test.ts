// Lock the 14-preset roster, the pair invariants, and the resolver
// fallback. The desktop renders the picker as `6 light + 8 dark`
// rows; a future PR that adds or removes a preset MUST land here
// first so the Settings UI doesn't quietly regress to 13 or 15.

import { describe, expect, it } from 'vitest'

import { ACCENT_PALETTES } from './palettes'
import { DEFAULT_THEME_ID, findTheme, THEME_PRESETS } from './themes'

describe('THEME_PRESETS', () => {
  it('ships exactly 14 presets (6 light + 8 dark)', () => {
    expect(THEME_PRESETS).toHaveLength(14)
    expect(THEME_PRESETS.filter((t) => t.mode === 'light')).toHaveLength(6)
    expect(THEME_PRESETS.filter((t) => t.mode === 'dark')).toHaveLength(8)
  })

  it('has unique stable ids', () => {
    const ids = new Set(THEME_PRESETS.map((t) => t.id))
    expect(ids.size).toBe(THEME_PRESETS.length)
  })

  it('uses a full 11-shade scale for every accent', () => {
    const shades = ['50', '100', '200', '300', '400', '500', '600', '700', '800', '900', '950']
    for (const preset of THEME_PRESETS) {
      for (const shade of shades) {
        expect(preset.accent[shade as keyof typeof preset.accent]).toMatch(/^oklch\(/)
      }
    }
  })

  it('reuses palettes from ACCENT_PALETTES (no orphan inline scales)', () => {
    // A preset that hand-rolled its own palette would diverge from
    // the desktop on a Tailwind refresh. Reference-equality is the
    // cheapest way to assert "this came from the named bind".
    const known = Object.values(ACCENT_PALETTES) as ReadonlyArray<typeof ACCENT_PALETTES.EMERALD>
    for (const preset of THEME_PRESETS) {
      expect(known).toContain(preset.accent)
    }
  })

  it('every `pair` points at a real preset with the opposite mode', () => {
    const byId = new Map(THEME_PRESETS.map((t) => [t.id, t]))
    for (const preset of THEME_PRESETS) {
      if (preset.pair === undefined) continue
      const mirror = byId.get(preset.pair)
      expect(mirror, `pair "${preset.pair}" referenced by "${preset.id}"`).toBeDefined()
      expect(mirror!.mode).not.toBe(preset.mode)
    }
  })

  it('one-off presets without a pair are documented (OLED, Neon)', () => {
    // The sun/moon toggle falls back to the global default pair
    // for these; locking the list keeps a new preset author from
    // forgetting to wire `pair` and quietly inheriting the fallback.
    // `default` / `default-dark` ARE paired (to each other), so
    // the only unpaired presets are OLED + Neon.
    const unpaired = THEME_PRESETS.filter((t) => t.pair === undefined).map((t) => t.id)
    expect(new Set(unpaired)).toEqual(new Set(['oled', 'neon']))
  })

  it('default + default-dark pair each other for the sun/moon toggle', () => {
    const def = THEME_PRESETS.find((t) => t.id === 'default')
    const defDark = THEME_PRESETS.find((t) => t.id === 'default-dark')
    expect(def?.pair).toBe('default-dark')
    expect(defDark?.pair).toBe('default')
  })
})

describe('findTheme', () => {
  it('resolves a known id', () => {
    expect(findTheme('midnight').id).toBe('midnight')
  })

  it('falls back to default-dark on unknown id', () => {
    expect(findTheme('does-not-exist').id).toBe(DEFAULT_THEME_ID)
  })

  it('falls back to default-dark on null / undefined / empty', () => {
    expect(findTheme(null).id).toBe(DEFAULT_THEME_ID)
    expect(findTheme(undefined).id).toBe(DEFAULT_THEME_ID)
    expect(findTheme('').id).toBe(DEFAULT_THEME_ID)
  })
})
