// ArtistDetailView render tests. Same harness as the album
// drill-down spec — passthrough probe for the tracklist, focus on
// the header matrix + loader branches.

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

vi.mock('@/server-fns/artists', () => ({
  getArtistTracks: vi.fn(),
  listArtists: vi.fn(),
}))

vi.mock('@/components/PlayableTrackList', () => ({
  PlayableTrackList: ({ tracks, label }: { tracks: unknown[]; label: string }) => (
    <div data-testid="tracklist" data-label={label}>
      {tracks.length} tracks
    </div>
  ),
}))

const { ArtistDetailView } =
  await import('./_authed.profiles.$profileId.libraries.$libraryId.artists.$artistId')

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

function makeTrack(id: number) {
  return {
    id,
    library_id: 2,
    album_id: null,
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

describe('ArtistDetailView', () => {
  it('renders the resolved artist name + the tracks pipe', () => {
    loaderData = {
      kind: 'ready',
      profileId: 1,
      libraryId: 2,
      artistId: 7,
      artistResult: { ok: true, artist: makeArtist(7, { name: 'Daft Punk' }) },
      tracks: [makeTrack(11), makeTrack(12)],
    }
    render(<ArtistDetailView />)
    expect(screen.getByRole('heading', { level: 1, name: 'Daft Punk' })).toBeTruthy()
    const list = screen.getByTestId('tracklist')
    expect(list.dataset.label).toBe('Artist tracks')
    expect(list.textContent).toBe('2 tracks')
  })

  it('falls back to a neutral header when the list resolved but the artist is missing (race)', () => {
    loaderData = {
      kind: 'ready',
      profileId: 1,
      libraryId: 2,
      artistId: 9999,
      artistResult: { ok: true, artist: null },
      tracks: [makeTrack(11)],
    }
    render(<ArtistDetailView />)
    expect(screen.getByRole('heading', { level: 1, name: 'Artist' })).toBeTruthy()
    expect(screen.queryByText(/details unavailable/i)).toBeNull()
  })

  it('renders the neutral header with a soft subtitle when the list fetch errored', () => {
    loaderData = {
      kind: 'ready',
      profileId: 1,
      libraryId: 2,
      artistId: 7,
      artistResult: { ok: false, error: 'waveflow-server is unavailable. Please try again.' },
      tracks: [makeTrack(11)],
    }
    render(<ArtistDetailView />)
    expect(screen.getByRole('heading', { level: 1, name: 'Artist' })).toBeTruthy()
    expect(screen.getByText(/details unavailable/i)).toBeTruthy()
  })

  it('renders the loader error path when the tracks fetch failed', () => {
    loaderData = { kind: 'error', message: 'Not found.' }
    render(<ArtistDetailView />)
    const alert = screen.getByRole('alert')
    expect(alert.textContent).toMatch(/not found/i)
    expect(screen.queryByTestId('tracklist')).toBeNull()
  })
})
