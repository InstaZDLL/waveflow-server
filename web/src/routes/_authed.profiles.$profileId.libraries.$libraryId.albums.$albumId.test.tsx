// AlbumDetailView render tests. Mocks `PlayableTrackList` as a
// passthrough probe so the spec focuses on the header matrix +
// loader branches; the player-side tests live in
// `PlayableTrackList.test.tsx`.

import { describe, expect, it, vi } from 'vitest'
import { render, screen } from '@testing-library/react'

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
  getAlbumTracks: vi.fn(),
  listAlbums: vi.fn(),
}))

// Stub the player surface so the spec doesn't have to drag the
// PlayerProvider + getStreamUrl mock in just to assert the
// header. We render a probe that surfaces the prop shape so the
// detail view's wiring is still validated.
vi.mock('@/components/PlayableTrackList', () => ({
  PlayableTrackList: ({
    tracks,
    label,
    emptyMessage,
  }: {
    tracks: unknown[]
    label: string
    emptyMessage: string
  }) => (
    <div data-testid="tracklist" data-label={label} data-empty={emptyMessage}>
      {tracks.length} tracks
    </div>
  ),
}))

const { AlbumDetailView } =
  await import('./_authed.profiles.$profileId.libraries.$libraryId.albums.$albumId')

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

function makeTrack(id: number) {
  return {
    id,
    library_id: 2,
    album_id: 7,
    title: `Track ${id}`,
    file_path: `/${id}.flac`,
    file_size: 1024,
    duration_ms: 60_000,
    track_number: null,
    disc_number: null,
    year: null,
    bitrate: null,
    sample_rate: null,
    channels: null,
    bit_depth: null,
    codec: null,
    rating: null,
  }
}

describe('AlbumDetailView', () => {
  it('renders the resolved album title, artist, year, and the tracks pipe', () => {
    loaderData = {
      kind: 'ready',
      profileId: 1,
      libraryId: 2,
      albumId: 7,
      albumResult: {
        ok: true,
        album: makeAlbum(7, {
          canonical_title: 'Discovery',
          album_artist_name: 'Daft Punk',
          year: 2001,
        }),
      },
      tracks: [makeTrack(11), makeTrack(12)],
    }
    render(<AlbumDetailView />)
    expect(screen.getByRole('heading', { level: 1, name: 'Discovery' })).toBeTruthy()
    expect(screen.getByText(/Daft Punk · 2001/)).toBeTruthy()
    const list = screen.getByTestId('tracklist')
    expect(list.dataset.label).toBe('Album tracks')
    expect(list.textContent).toBe('2 tracks')
  })

  it('renders the "Various Artists" subtitle + Compilation pill', () => {
    loaderData = {
      kind: 'ready',
      profileId: 1,
      libraryId: 2,
      albumId: 7,
      albumResult: {
        ok: true,
        album: makeAlbum(7, {
          canonical_title: "Now That's What I Call Music",
          is_compilation: true,
        }),
      },
      tracks: [],
    }
    render(<AlbumDetailView />)
    expect(screen.getByText('Various Artists')).toBeTruthy()
    expect(screen.getByText('Compilation')).toBeTruthy()
  })

  it('falls back to a neutral header when the list resolved but the album is missing (race)', () => {
    // Race window: peer device deleted the album between the list
    // and the tracks fetch. Tracks already came back OK (this can
    // happen on a real eventually-consistent setup) so the page
    // is still useful.
    loaderData = {
      kind: 'ready',
      profileId: 1,
      libraryId: 2,
      albumId: 9999,
      albumResult: { ok: true, album: null },
      tracks: [makeTrack(11)],
    }
    render(<AlbumDetailView />)
    expect(screen.getByRole('heading', { level: 1, name: 'Album' })).toBeTruthy()
    // The neutral fallback must NOT surface an "unavailable" subtitle
    // when the list itself succeeded — that copy is for the
    // ok=false branch only.
    expect(screen.queryByText(/details unavailable/i)).toBeNull()
  })

  it('renders the neutral header with a soft subtitle when the list fetch errored', () => {
    loaderData = {
      kind: 'ready',
      profileId: 1,
      libraryId: 2,
      albumId: 7,
      albumResult: { ok: false, error: 'waveflow-server is unavailable. Please try again.' },
      tracks: [makeTrack(11)],
    }
    render(<AlbumDetailView />)
    expect(screen.getByRole('heading', { level: 1, name: 'Album' })).toBeTruthy()
    expect(screen.getByText(/details unavailable/i)).toBeTruthy()
  })

  it('renders the loader error path when the tracks fetch failed', () => {
    loaderData = { kind: 'error', message: 'Not found.' }
    render(<AlbumDetailView />)
    const alert = screen.getByRole('alert')
    expect(alert.textContent).toMatch(/not found/i)
    // No tracklist probe should mount on the error path.
    expect(screen.queryByTestId('tracklist')).toBeNull()
  })
})
