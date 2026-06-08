// ArtistsView render tests. Same harness pattern as the albums
// spec — mock the file-route + the server-fn, drive loaderData
// per-spec.

import { describe, expect, it, vi } from 'vitest'
import { render, screen, within } from '@testing-library/react'

let loaderData: unknown = null

vi.mock('@tanstack/react-router', () => ({
  createFileRoute: () => (config: unknown) => ({
    ...(config as Record<string, unknown>),
    useLoaderData: () => loaderData,
  }),
  Link: ({ children, ...rest }: React.PropsWithChildren<Record<string, unknown>>) => (
    <a {...rest}>{children}</a>
  ),
}))

vi.mock('@/server-fns/artists', () => ({
  listArtists: vi.fn(),
}))

const { ArtistsView } = await import('./_authed.profiles.$profileId.libraries.$libraryId.artists')

function makeArtist(id: number, overrides: Record<string, unknown> = {}) {
  return {
    id,
    name: `Artist ${id}`,
    picture_hash: null,
    created_at: 0,
    updated_at: 0,
    ...overrides,
  }
}

describe('ArtistsView', () => {
  it('renders the empty-state copy when the library has no artists', () => {
    loaderData = { kind: 'ready', profileId: 1, libraryId: 2, artists: [] }
    render(<ArtistsView />)
    expect(screen.getByText(/no artists in this library yet/i)).toBeTruthy()
  })

  it('renders one card per artist', () => {
    loaderData = {
      kind: 'ready',
      profileId: 1,
      libraryId: 2,
      artists: [makeArtist(11, { name: 'Daft Punk' }), makeArtist(12, { name: 'Justice' })],
    }
    render(<ArtistsView />)
    const list = screen.getByRole('list')
    const cards = within(list).getAllByRole('listitem')
    expect(cards).toHaveLength(2)
    expect(screen.getByText('Daft Punk')).toBeTruthy()
    expect(screen.getByText('Justice')).toBeTruthy()
  })

  it('renders the loader error path', () => {
    loaderData = { kind: 'error', message: 'Not found.' }
    render(<ArtistsView />)
    const alert = screen.getByRole('alert')
    expect(alert.textContent).toMatch(/not found/i)
  })
})
