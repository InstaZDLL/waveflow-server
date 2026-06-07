// ThemePicker UI — verify the 14 tiles render, the active tile is
// marked, and clicking flows through to the ThemeProvider setter.

import { describe, expect, it, vi } from 'vitest'
import { render, screen, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'

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

  it('clicking a tile flips the active state', async () => {
    const user = userEvent.setup()
    mount('default-dark')
    const lavender = screen
      .getAllByRole('radio')
      .find(
        (t) =>
          t.textContent?.toLowerCase().includes('lavender') &&
          !t.textContent?.toLowerCase().includes('light'),
      )
    expect(lavender).toBeDefined()
    await user.click(lavender!)
    expect(lavender!.getAttribute('aria-checked')).toBe('true')
    expect(document.documentElement.getAttribute('data-theme')).toBe('lavender')
  })

  it('only the active tile in each row has tabIndex=0 (roving tabindex)', () => {
    mount('crimson')
    const darkGroup = screen.getByRole('radiogroup', { name: /dark themes/i })
    const darkTiles = within(darkGroup).getAllByRole('radio')
    const tabStops = darkTiles.filter((t) => t.tabIndex === 0)
    expect(tabStops).toHaveLength(1)
    expect(tabStops[0].getAttribute('aria-checked')).toBe('true')
  })

  it('ArrowRight moves focus + selection to the next tile in the same row', async () => {
    const user = userEvent.setup()
    mount('default-dark')
    const darkGroup = screen.getByRole('radiogroup', { name: /dark themes/i })
    const darkTiles = within(darkGroup).getAllByRole('radio')
    // default-dark is the first tile of the dark row; ArrowRight
    // should jump to oled (index 1) and select it.
    const start = darkTiles[0]
    start.focus()
    await user.keyboard('{ArrowRight}')
    expect(document.activeElement).toBe(darkTiles[1])
    expect(document.documentElement.getAttribute('data-theme')).toBe('oled')
  })

  it('ArrowLeft from the first tile wraps to the last (radio-pattern convention)', async () => {
    const user = userEvent.setup()
    mount('default-dark')
    const darkGroup = screen.getByRole('radiogroup', { name: /dark themes/i })
    const darkTiles = within(darkGroup).getAllByRole('radio')
    darkTiles[0].focus()
    await user.keyboard('{ArrowLeft}')
    // Wraps to the last dark tile (`neon`).
    expect(document.activeElement).toBe(darkTiles[darkTiles.length - 1])
    expect(document.documentElement.getAttribute('data-theme')).toBe('neon')
  })

  it('Space selects the focused tile without moving focus', async () => {
    const user = userEvent.setup()
    mount('default-dark')
    const darkGroup = screen.getByRole('radiogroup', { name: /dark themes/i })
    const darkTiles = within(darkGroup).getAllByRole('radio')
    // The lavender tile sits at dark index 4 (default-dark, oled,
    // midnight, sunset, lavender, crimson, ocean, neon).
    const lavender = darkTiles[4]
    lavender.focus()
    await user.keyboard(' ')
    expect(document.activeElement).toBe(lavender)
    expect(document.documentElement.getAttribute('data-theme')).toBe('lavender')
  })
})
