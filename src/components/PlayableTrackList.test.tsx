// PlayableTrackList — render + interaction smoke tests. Mocks the
// stream-url server-fn (so the real PlayerProvider can resolve a
// fake stream URL) and exercises the empty state, the row render,
// and the play-click → context-queue handshake.

import { describe, expect, it, vi } from 'vitest'
import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'

const getStreamUrl = vi.fn(async ({ data }: { data: { trackId: number } }) => ({
  url: `https://stream.example/track/${data.trackId}.mp3`,
}))

vi.mock('@/server-fns/stream', () => ({
  getStreamUrl: (arg: { data: { trackId: number } }) => getStreamUrl(arg),
}))

const { PlayerProvider, usePlayer } = await import('@/lib/player-context')
const { PlayableTrackList } = await import('./PlayableTrackList')

function makeTrack(id: number, overrides: Record<string, unknown> = {}) {
  return {
    id,
    library_id: 2,
    album_id: null,
    title: `Track ${id}`,
    file_path: `/m/${id}.flac`,
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
    ...overrides,
  }
}

function CurrentTrackProbe() {
  const player = usePlayer()
  return <span data-testid="current">{player.current?.trackId ?? 'none'}</span>
}

describe('PlayableTrackList', () => {
  it('renders the empty-state message when no tracks are passed', () => {
    render(
      <PlayerProvider>
        <PlayableTrackList
          profileId={1}
          libraryId={2}
          tracks={[]}
          emptyMessage="Nothing here yet."
          label="Test tracks"
        />
      </PlayerProvider>,
    )
    expect(screen.getByText('Nothing here yet.')).toBeTruthy()
    // No play button when there's nothing to play.
    expect(screen.queryByRole('button')).toBeNull()
  })

  it('renders one play button per track + the optional aria label', () => {
    render(
      <PlayerProvider>
        <PlayableTrackList
          profileId={1}
          libraryId={2}
          tracks={[makeTrack(11, { title: 'Alpha' }), makeTrack(12, { title: 'Beta' })]}
          emptyMessage="empty"
          label="Album tracks"
        />
      </PlayerProvider>,
    )
    expect(screen.getByRole('list', { name: /album tracks/i })).toBeTruthy()
    expect(screen.getByRole('button', { name: /play alpha/i })).toBeTruthy()
    expect(screen.getByRole('button', { name: /play beta/i })).toBeTruthy()
    expect(screen.getByText('Alpha')).toBeTruthy()
    expect(screen.getByText('Beta')).toBeTruthy()
  })

  it('promotes the clicked track to player.current + mints its stream url', async () => {
    const user = userEvent.setup()
    render(
      <PlayerProvider>
        <CurrentTrackProbe />
        <PlayableTrackList
          profileId={1}
          libraryId={2}
          tracks={[makeTrack(11, { title: 'Alpha' }), makeTrack(12, { title: 'Beta' })]}
          emptyMessage="empty"
          label="Test tracks"
        />
      </PlayerProvider>,
    )
    expect(screen.getByTestId('current').textContent).toBe('none')

    await user.click(screen.getByRole('button', { name: /play alpha/i }))

    expect(getStreamUrl).toHaveBeenCalledWith(
      expect.objectContaining({ data: expect.objectContaining({ trackId: 11 }) }),
    )
    // PlayerProvider promotes the entry to `current` synchronously
    // after the URL resolves — the button click `await`s the
    // playTrack promise so by here it's settled.
    expect(screen.getByTestId('current').textContent).toBe('11')
  })

  it('renders the codec subline only when present', () => {
    render(
      <PlayerProvider>
        <PlayableTrackList
          profileId={1}
          libraryId={2}
          tracks={[
            makeTrack(11, { title: 'With codec', codec: 'FLAC' }),
            makeTrack(12, { title: 'No codec' }),
          ]}
          emptyMessage="empty"
          label="Test tracks"
        />
      </PlayerProvider>,
    )
    expect(screen.getByText('FLAC')).toBeTruthy()
    // The "No codec" row mustn't surface any FLAC text leaked from
    // the sibling row.
    const beta = screen.getByText('No codec').closest('li')
    expect(beta?.textContent).not.toContain('FLAC')
  })
})
