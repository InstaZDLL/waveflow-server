// ThemeStyle — server-rendered <style> block that ships the
// active preset's CSS variables in the initial HTML payload, so
// the first paint already has the right tint and there's no flash
// of the brand palette before React hydrates and applies the
// stored theme.
//
// Mounted inside `<head>` from `__root.tsx`. Idempotent against a
// client re-render — React will keep the matching `:root { ... }`
// block in place; the provider's `applyTheme` effect overwrites
// the same custom properties on the documentElement via
// `style.setProperty`, which wins over a stylesheet rule, so
// switching themes after hydration still flips the tint without
// having to re-mint this string.

import { findTheme, themeCssDeclarations } from '@waveflow/design-tokens'

export interface ThemeStyleProps {
  themeId: string
}

export function ThemeStyle({ themeId }: ThemeStyleProps) {
  // `findTheme` validates against the package's known preset
  // roster — an unknown / attacker-supplied id falls back to
  // DEFAULT_THEME_ID, never lands as-is. The string output of
  // `themeCssDeclarations` is built from hardcoded OKLCH constants
  // + the preset's typed surface fields (hex literals in the source
  // table), so the body is fully controlled and contains no user
  // input. Using `dangerouslySetInnerHTML` is the only way to
  // server-render a literal CSS block — React's `<style>{...}</style>`
  // path escapes its children, which would corrupt the declarations.
  const theme = findTheme(themeId)
  const css = `:root {\n  ${themeCssDeclarations(theme)}\n}`
  return <style dangerouslySetInnerHTML={{ __html: css }} />
}
