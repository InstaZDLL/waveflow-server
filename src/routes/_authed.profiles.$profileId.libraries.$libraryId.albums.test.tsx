// AlbumsView render tests. Mocks the file-route so the component
// mounts without the router; loaderData is module-scoped so each
// spec sets its own shape.

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

vi.mock('@/server-fns/albums', () => ({
  listAlbums: vi.fn(),
}))

const { AlbumsView } = await import('./_authed.profiles.$profileId.libraries.$libraryId.albums')

function makeAlbum(id: number, overrides: Record<string, unknown> = {}) {
  return {
    id,
    canonical_title: `Album ${id}`,
    album_artist_id: null,
    album_artist_name: null,
    year: null,
    cover_hash: null,
    is_compilation: false,
    created_at: 0,
    updated_at: 0,
    ...overrides,
  }
}

describe('AlbumsView', () => {
  it('renders the empty-state copy when the library has no albums', () => {
    loaderData = { kind: 'ready', profileId: 1, libraryId: 2, albums: [] }
    render(<AlbumsView />)
    expect(screen.getByText(/no albums in this library yet/i)).toBeTruthy()
  })

  it('renders one card per album with subtitle + year', () => {
    loaderData = {
      kind: 'ready',
      profileId: 1,
      libraryId: 2,
      albums: [
        makeAlbum(11, {
          canonical_title: 'Discovery',
          album_artist_name: 'Daft Punk',
          year: 2001,
        }),
        makeAlbum(12, {
          canonical_title: 'Random Access Memories',
          album_artist_name: 'Daft Punk',
          year: 2013,
        }),
      ],
    }
    render(<AlbumsView />)
    const list = screen.getByRole('list')
    const cards = within(list).getAllByRole('listitem')
    expect(cards).toHaveLength(2)
    expect(screen.getByText('Discovery')).toBeTruthy()
    // Subtitle joins artist + year.
    expect(screen.getByText(/Daft Punk · 2001/)).toBeTruthy()
    expect(screen.getByText(/Daft Punk · 2013/)).toBeTruthy()
  })

  it('renders the "Various Artists" subtitle for compilations', () => {
    loaderData = {
      kind: 'ready',
      profileId: 1,
      libraryId: 2,
      albums: [
        makeAlbum(7, {
          canonical_title: "Now That's What I Call Music",
          is_compilation: true,
          // album_artist_name stays null — the row sets the
          // compilation flag instead. The view falls back to
          // "Various Artists" rather than leaking the null.
        }),
      ],
    }
    render(<AlbumsView />)
    expect(screen.getByText('Various Artists')).toBeTruthy()
  })

  it('falls back to "Unknown artist" when neither flag nor name is set', () => {
    loaderData = {
      kind: 'ready',
      profileId: 1,
      libraryId: 2,
      // Defensive — shouldn't happen in production data (every
      // non-compilation album has an album_artist_id) but the
      // server's wire shape allows it.
      albums: [makeAlbum(9, { canonical_title: 'Orphan' })],
    }
    render(<AlbumsView />)
    expect(screen.getByText(/unknown artist/i)).toBeTruthy()
  })

  it('renders the loader error path', () => {
    loaderData = { kind: 'error', message: 'Not found.' }
    render(<AlbumsView />)
    const alert = screen.getByRole('alert')
    expect(alert.textContent).toMatch(/not found/i)
  })
})
