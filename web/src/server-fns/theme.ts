// Theme persistence — read + write the user's chosen theme id from a
// cookie so SSR can resolve it and inline the right CSS variables in
// `<head>` BEFORE React hydrates. localStorage would force a
// client-side flash (the server would always default-paint the
// brand palette and the React effect would re-tint after mount);
// the cookie is the only mechanism the server can read.
//
// Cookie shape: `waveflow-theme-id=<preset.id>`. Not httpOnly —
// the bootstrap script + the React provider both need to read it.
// SameSite=Lax so a follow-link arrival from another origin
// inherits the choice, and Secure under HTTPS for prod hygiene.

import { createServerFn } from '@tanstack/react-start'
import { getCookie, setCookie } from '@tanstack/react-start/server'
import { DEFAULT_THEME_ID, THEME_PRESETS } from '@waveflow/design-tokens'

export const THEME_COOKIE_NAME = 'waveflow-theme-id'

/**
 * Validated against the known preset roster — an attacker pasting
 * `theme=<script>` into a cookie should bounce back to the default
 * rather than land as a `data-theme=<script>` attribute. The same
 * check runs server-side (here) AND client-side (the provider) so
 * the contract holds regardless of which entry point trusts the
 * value.
 */
function isKnownThemeId(value: string): boolean {
  return THEME_PRESETS.some((preset) => preset.id === value)
}

/**
 * Read the stored theme id at SSR time. Falls back to the default
 * id when the cookie is absent or carries an unknown value — never
 * throws, never logs.
 */
export const getStoredThemeId = createServerFn({ method: 'GET' }).handler(
  async (): Promise<string> => {
    const raw = getCookie(THEME_COOKIE_NAME)
    if (raw && isKnownThemeId(raw)) return raw
    return DEFAULT_THEME_ID
  },
)

/**
 * Persist the chosen theme id. Validates against the known preset
 * roster so the cookie never carries a value the lookup chain
 * can't resolve.
 *
 * Cookie attributes:
 * - `maxAge: 1 year` — re-set on each call so an active user keeps
 *   their choice across browser sessions.
 * - `sameSite: 'lax'` — preserves the choice when arriving from
 *   the desktop OAuth redirect (`/desktop-login`).
 * - `secure: NODE_ENV === 'production'` — drop the flag in dev so
 *   `http://localhost:3000` can still set the cookie.
 * - `httpOnly: false` — the React provider + the bootstrap script
 *   both need DOM-side access to mirror the server-side state.
 */
export const setStoredThemeId = createServerFn({ method: 'POST' })
  .inputValidator((data: unknown): { themeId: string } => {
    if (typeof data !== 'object' || data === null) {
      throw new Error('setStoredThemeId: expected { themeId } body')
    }
    const themeId = (data as { themeId?: unknown }).themeId
    if (typeof themeId !== 'string') {
      throw new Error('setStoredThemeId: themeId must be a string')
    }
    if (!isKnownThemeId(themeId)) {
      throw new Error(`setStoredThemeId: unknown theme id "${themeId}"`)
    }
    return { themeId }
  })
  .handler(async ({ data }): Promise<{ themeId: string }> => {
    setCookie(THEME_COOKIE_NAME, data.themeId, {
      maxAge: 60 * 60 * 24 * 365,
      sameSite: 'lax',
      secure: process.env.NODE_ENV === 'production',
      httpOnly: false,
      path: '/',
    })
    return { themeId: data.themeId }
  })
