// ThemeProvider integration — verify the optimistic apply +
// cookie write, the rollback on a server-fn rejection, and that
// `useTheme` throws when called outside the provider.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { act, render, renderHook, screen } from '@testing-library/react'

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

    await act(async () => {
      result.current.setTheme('lavender')
      // Yield to the rejected microtask so the rollback runs before
      // we assert.
      await Promise.resolve()
    })

    expect(result.current.theme.id).toBe('default-dark')
    expect(document.documentElement.getAttribute('data-theme')).toBe('default-dark')
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
