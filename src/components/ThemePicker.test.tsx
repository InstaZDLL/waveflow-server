// ThemePicker UI — verify the 14 tiles render, the active tile is
// marked, and clicking flows through to the ThemeProvider setter.

import { describe, expect, it, vi } from 'vitest'
import { fireEvent, render, screen, within } from '@testing-library/react'

vi.mock('@/server-fns/theme', () => ({
  setStoredThemeId: vi.fn().mockResolvedValue({ themeId: 'lavender' }),
}))

const { ThemeProvider } = await import('./ThemeProvider')
const { ThemePicker } = await import('./ThemePicker')

function mount(initialThemeId = 'default-dark') {
  return render(
    <ThemeProvider initialThemeId={initialThemeId}>
      <ThemePicker />
    </ThemeProvider>,
  )
}

describe('ThemePicker', () => {
  it('renders 6 light + 8 dark tiles', () => {
    mount()
    const lightGroup = screen.getByRole('radiogroup', { name: /light themes/i })
    const darkGroup = screen.getByRole('radiogroup', { name: /dark themes/i })
    expect(within(lightGroup).getAllByRole('radio')).toHaveLength(6)
    expect(within(darkGroup).getAllByRole('radio')).toHaveLength(8)
  })

  it('marks the active preset with aria-checked', () => {
    mount('crimson')
    const tiles = screen.getAllByRole('radio')
    const active = tiles.filter((t) => t.getAttribute('aria-checked') === 'true')
    expect(active).toHaveLength(1)
    expect(active[0].textContent).toMatch(/crimson/i)
  })

  it('clicking a tile flips the active state', () => {
    mount('default-dark')
    const lavender = screen
      .getAllByRole('radio')
      .find(
        (t) =>
          t.textContent?.toLowerCase().includes('lavender') &&
          !t.textContent?.toLowerCase().includes('light'),
      )
    expect(lavender).toBeDefined()
    fireEvent.click(lavender!)
    expect(lavender!.getAttribute('aria-checked')).toBe('true')
    expect(document.documentElement.getAttribute('data-theme')).toBe('lavender')
  })
})
