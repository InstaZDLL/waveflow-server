// ThemeProvider — React context that mirrors the cookie-backed
// theme choice on the client + applies the preset's CSS variables
// on every change. SSR-safe: the value passed in from the root
// loader is what gets rendered first, so the inline `<style>` in
// `<head>` (see `ThemeStyle`) and the React tree agree on which
// preset is active.
//
// The cookie write happens through a `setStoredThemeId` server-fn
// call rather than `document.cookie =` so the prod build's `secure`
// + `httpOnly: false` + `SameSite=Lax` shape is centralised in one
// place. The provider optimistically applies the new theme to the
// DOM before the server-fn round-trips so the UI feels snappy.

import { applyTheme, DEFAULT_THEME_ID, findTheme, type ThemePreset } from '@waveflow/design-tokens'
import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useState,
  type ReactNode,
} from 'react'

import { setStoredThemeId } from '@/server-fns/theme'

interface ThemeContextValue {
  /** Currently active preset — fully resolved (never null). */
  theme: ThemePreset
  /**
   * Switch presets. Applies the new tint to the DOM immediately
   * and persists the choice to the cookie in the background. A
   * concurrent failed write logs to the console and rolls back
   * the optimistic apply so the cookie + UI stay in sync.
   */
  setTheme: (id: string) => void
}

const ThemeContext = createContext<ThemeContextValue | null>(null)

export interface ThemeProviderProps {
  /** Initial id resolved by the SSR loader from the cookie. */
  initialThemeId: string
  children: ReactNode
}

export function ThemeProvider({ initialThemeId, children }: ThemeProviderProps) {
  // findTheme falls back to default on an unknown id, so a stale
  // cookie value from a removed preset can't crash the tree.
  const [themeId, setThemeId] = useState<string>(() => findTheme(initialThemeId).id)
  const theme = useMemo(() => findTheme(themeId), [themeId])

  // Re-apply on every change so the optimistic update on
  // `setTheme` and a future external write (e.g. multi-tab sync
  // via `storage` event) both flow through the same DOM helper.
  useEffect(() => {
    applyTheme(theme)
  }, [theme])

  const setTheme = useCallback(
    (id: string) => {
      const resolved = findTheme(id).id
      const previous = themeId
      setThemeId(resolved)
      // Optimistic — the server-fn writes the cookie. On failure,
      // roll back so the DOM, cookie, and React state agree.
      setStoredThemeId({ data: { themeId: resolved } }).catch((err) => {
        console.error('ThemeProvider: failed to persist theme', err)
        setThemeId(previous)
      })
    },
    [themeId],
  )

  const value = useMemo(() => ({ theme, setTheme }), [theme, setTheme])
  return <ThemeContext.Provider value={value}>{children}</ThemeContext.Provider>
}

/**
 * Read the active theme + the setter from anywhere inside the
 * provider. Throws if called outside (a programming error — the
 * provider is mounted at the root). Components that may render
 * outside a provider (a Storybook isolate, a hypothetical static
 * snapshot) should reach for the package's `findTheme` directly.
 */
export function useTheme(): ThemeContextValue {
  const ctx = useContext(ThemeContext)
  if (!ctx) {
    throw new Error('useTheme: must be called inside <ThemeProvider>')
  }
  return ctx
}

export { DEFAULT_THEME_ID }
