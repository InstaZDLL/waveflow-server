// ThemeProvider integration — verify the optimistic apply +
// cookie write, the rollback on a server-fn rejection, and that
// `useTheme` throws when called outside the provider.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { act, render, renderHook, screen, waitFor } from '@testing-library/react'

const setStoredThemeId = vi.fn()

vi.mock('@/server-fns/theme', () => ({
  setStoredThemeId: (...args: unknown[]) => setStoredThemeId(...args),
}))

const { ThemeProvider, useTheme } = await import('./ThemeProvider')

beforeEach(() => {
  setStoredThemeId.mockReset()
  setStoredThemeId.mockResolvedValue({ themeId: 'midnight' })
  // Reset the documentElement between tests so a prior preset's
  // accent leak doesn't poison the next assertion.
  document.documentElement.removeAttribute('style')
  document.documentElement.removeAttribute('data-theme')
  document.documentElement.classList.remove('dark')
})

afterEach(() => {
  document.documentElement.removeAttribute('style')
  document.documentElement.removeAttribute('data-theme')
  document.documentElement.classList.remove('dark')
})

describe('ThemeProvider', () => {
  it('applies the initial theme on mount', () => {
    render(
      <ThemeProvider initialThemeId="lavender">
        <div data-testid="child">child</div>
      </ThemeProvider>,
    )
    expect(screen.getByTestId('child')).toBeTruthy()
    expect(document.documentElement.getAttribute('data-theme')).toBe('lavender')
    expect(document.documentElement.classList.contains('dark')).toBe(true)
  })

  it('falls back to default-dark on an unknown initialThemeId', () => {
    render(
      <ThemeProvider initialThemeId="this-preset-does-not-exist">
        <div />
      </ThemeProvider>,
    )
    expect(document.documentElement.getAttribute('data-theme')).toBe('default-dark')
  })

  it('setTheme applies optimistically AND calls the server-fn', async () => {
    const { result } = renderHook(() => useTheme(), {
      wrapper: ({ children }) => (
        <ThemeProvider initialThemeId="default-dark">{children}</ThemeProvider>
      ),
    })
    expect(result.current.theme.id).toBe('default-dark')

    await act(async () => {
      result.current.setTheme('crimson')
    })

    expect(result.current.theme.id).toBe('crimson')
    expect(document.documentElement.getAttribute('data-theme')).toBe('crimson')
    expect(setStoredThemeId).toHaveBeenCalledWith({ data: { themeId: 'crimson' } })
  })

  it('rolls back the optimistic apply when the server-fn rejects', async () => {
    setStoredThemeId.mockRejectedValueOnce(new Error('server down'))
    const { result } = renderHook(() => useTheme(), {
      wrapper: ({ children }) => (
        <ThemeProvider initialThemeId="default-dark">{children}</ThemeProvider>
      ),
    })

    // Silence the expected console.error from the rollback path so
    // CI doesn't read it as a regression.
    const errorSpy = vi.spyOn(console, 'error').mockImplementation(() => {})

    act(() => {
      result.current.setTheme('lavender')
    })

    // `waitFor` polls until the rejection handler + rollback have
    // actually run. The previous `await Promise.resolve()` only
    // yielded one microtask, which happened to work today but
    // would flake on any change that adds a second microtask hop
    // (e.g. a future `requestIdleCallback` shim, an async wrapper
    // around `applyTheme`).
    await waitFor(() => {
      expect(result.current.theme.id).toBe('default-dark')
      expect(document.documentElement.getAttribute('data-theme')).toBe('default-dark')
    })
    errorSpy.mockRestore()
  })

  it('does not roll back when a newer setTheme has superseded the failing one', async () => {
    // First request: lavender → rejects.
    // Second request: crimson → resolves.
    // The lavender rejection must NOT roll back to default-dark
    // because the user has since asked for crimson — clobbering
    // crimson would leave the UI on a stale preset.
    let resolveFirst: (() => void) | undefined
    setStoredThemeId.mockImplementationOnce(
      () =>
        new Promise((_, reject) => {
          resolveFirst = () => reject(new Error('server down'))
        }),
    )
    setStoredThemeId.mockResolvedValueOnce({ themeId: 'crimson' })

    const { result } = renderHook(() => useTheme(), {
      wrapper: ({ children }) => (
        <ThemeProvider initialThemeId="default-dark">{children}</ThemeProvider>
      ),
    })

    const errorSpy = vi.spyOn(console, 'error').mockImplementation(() => {})

    act(() => {
      result.current.setTheme('lavender')
    })
    act(() => {
      result.current.setTheme('crimson')
    })

    // Trigger the queued rejection now that crimson is the latest
    // requested theme. The handler runs and finds its `resolved`
    // ('lavender') doesn't match `lastRequestedThemeIdRef.current`
    // ('crimson'), so it skips the rollback.
    act(() => {
      resolveFirst?.()
    })

    await waitFor(() => {
      expect(result.current.theme.id).toBe('crimson')
      expect(document.documentElement.getAttribute('data-theme')).toBe('crimson')
    })
    errorSpy.mockRestore()
  })
})

describe('useTheme', () => {
  it('throws when called outside the provider', () => {
    // Suppress React's expected "thrown error in component" log so
    // the test output stays clean.
    const errorSpy = vi.spyOn(console, 'error').mockImplementation(() => {})
    expect(() => renderHook(() => useTheme())).toThrow(/inside <ThemeProvider>/)
    errorSpy.mockRestore()
  })
})
