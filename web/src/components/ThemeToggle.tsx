import { useLayoutEffect, useState } from 'react'

type ThemeMode = 'light' | 'dark' | 'auto'

function getInitialMode(): ThemeMode {
  if (typeof window === 'undefined') {
    return 'auto'
  }

  const stored = window.localStorage.getItem('theme')
  if (stored === 'light' || stored === 'dark' || stored === 'auto') {
    return stored
  }

  return 'auto'
}

function applyThemeMode(mode: ThemeMode) {
  const prefersDark = window.matchMedia('(prefers-color-scheme: dark)').matches
  const resolved = mode === 'auto' ? (prefersDark ? 'dark' : 'light') : mode

  document.documentElement.classList.remove('light', 'dark')
  document.documentElement.classList.add(resolved)

  if (mode === 'auto') {
    document.documentElement.removeAttribute('data-theme')
  } else {
    document.documentElement.setAttribute('data-theme', mode)
  }

  document.documentElement.style.colorScheme = resolved
}

export default function ThemeToggle() {
  // Lazy initial state — reads from localStorage on mount instead of
  // setState-in-effect (which `react-hooks/set-state-in-effect`
  // catches as a code smell). SSR-safe because `getInitialMode()`
  // checks for `window` and falls back to `'auto'`.
  const [mode, setMode] = useState<ThemeMode>(getInitialMode)

  // Apply the theme synchronously before paint whenever `mode`
  // changes. `useLayoutEffect` avoids the flash a regular
  // `useEffect` would produce (paint with the old class, then
  // re-paint with the new one). SSR-safe: React skips
  // useLayoutEffect on the server, and `applyThemeMode` touches
  // `document` which doesn't exist there anyway. Depending on
  // `[mode]` means both the initial mount AND every `toggleMode`
  // call route through the same code path — no exhaustive-deps
  // workaround needed.
  useLayoutEffect(() => {
    applyThemeMode(mode)
  }, [mode])

  useLayoutEffect(() => {
    if (mode !== 'auto') {
      return
    }

    const media = window.matchMedia('(prefers-color-scheme: dark)')
    const onChange = () => applyThemeMode('auto')

    media.addEventListener('change', onChange)
    return () => {
      media.removeEventListener('change', onChange)
    }
  }, [mode])

  function toggleMode() {
    const nextMode: ThemeMode = mode === 'light' ? 'dark' : mode === 'dark' ? 'auto' : 'light'
    setMode(nextMode)
    // No direct `applyThemeMode` call — the useLayoutEffect above
    // owns theme application and fires synchronously on the state
    // change before the next paint.
    window.localStorage.setItem('theme', nextMode)
  }

  const label =
    mode === 'auto'
      ? 'Theme mode: auto (system). Click to switch to light mode.'
      : `Theme mode: ${mode}. Click to switch mode.`

  return (
    <button
      type="button"
      onClick={toggleMode}
      aria-label={label}
      title={label}
      className="rounded-full border border-[var(--chip-line)] bg-[var(--chip-bg)] px-3 py-1.5 text-sm font-semibold text-[var(--sea-ink)] shadow-[0_8px_22px_rgba(30,90,72,0.08)] transition hover:-translate-y-0.5"
    >
      {mode === 'auto' ? 'Auto' : mode === 'dark' ? 'Dark' : 'Light'}
    </button>
  )
}
