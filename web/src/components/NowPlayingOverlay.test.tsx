// NowPlayingOverlay render + interaction tests. Mirrors the
// QueuePanel suite — closed = null, open = dialog + controls,
// ESC + backdrop dismiss, focus moves to the close button on
// open. The seek + volume sliders are mostly visual; we assert
// they're present rather than dragging them (PlayerBar carries
// the same slider implementation and is covered by that surface's
// behaviour tests).

import { describe, expect, it, vi } from 'vitest'
import { render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'

const getStreamUrl = vi.fn(async ({ data }: { data: { trackId: number } }) => ({
  url: `https://stream.example/track/${data.trackId}.mp3`,
}))

vi.mock('@/server-fns/stream', () => ({
  getStreamUrl: (arg: { data: { trackId: number } }) => getStreamUrl(arg),
}))

const { PlayerProvider, usePlayer } = await import('@/lib/player-context')
const { NowPlayingOverlay } = await import('./NowPlayingOverlay')

function makeEntry(trackId: number, title = `Track ${trackId}`) {
  return { profileId: 1, libraryId: 1, trackId, title, durationMs: 60_000 }
}

function Harness({ open, onClose }: { open: boolean; onClose: () => void }) {
  const player = usePlayer()
  return (
    <>
      <button
        type="button"
        onClick={async () => {
          await player.playTrack(makeEntry(1, 'Hero song'))
        }}
      >
        seed
      </button>
      <NowPlayingOverlay open={open} onClose={onClose} />
    </>
  )
}

describe('NowPlayingOverlay', () => {
  it('renders nothing when closed', () => {
    render(
      <PlayerProvider>
        <Harness open={false} onClose={() => {}} />
      </PlayerProvider>,
    )
    expect(screen.queryByRole('dialog')).toBeNull()
  })

  it('renders nothing when there is no current track', () => {
    render(
      <PlayerProvider>
        <Harness open={true} onClose={() => {}} />
      </PlayerProvider>,
    )
    expect(screen.queryByRole('dialog')).toBeNull()
  })

  it('renders the dialog + close button + transport controls once a track is playing', async () => {
    const user = userEvent.setup()
    render(
      <PlayerProvider>
        <Harness open={true} onClose={() => {}} />
      </PlayerProvider>,
    )
    await user.click(screen.getByRole('button', { name: /seed/i }))
    await waitFor(() => {
      expect(screen.getByRole('dialog', { name: /now playing/i })).toBeTruthy()
    })
    expect(screen.getByRole('button', { name: /close now playing/i })).toBeTruthy()
    expect(screen.getByRole('button', { name: /previous track/i })).toBeTruthy()
    expect(screen.getByRole('button', { name: /pause/i })).toBeTruthy()
    expect(screen.getByRole('button', { name: /next track/i })).toBeTruthy()
    expect(screen.getByText('Hero song')).toBeTruthy()
  })

  it('fires onClose when the user presses Escape', async () => {
    const user = userEvent.setup()
    const onClose = vi.fn()
    render(
      <PlayerProvider>
        <Harness open={true} onClose={onClose} />
      </PlayerProvider>,
    )
    await user.click(screen.getByRole('button', { name: /seed/i }))
    await user.keyboard('{Escape}')
    expect(onClose).toHaveBeenCalled()
  })

  it('fires onClose when the user clicks the backdrop', async () => {
    const user = userEvent.setup()
    const onClose = vi.fn()
    render(
      <PlayerProvider>
        <Harness open={true} onClose={onClose} />
      </PlayerProvider>,
    )
    await user.click(screen.getByRole('button', { name: /seed/i }))
    const backdrop = document.querySelector('[aria-hidden="true"].fixed')
    expect(backdrop).toBeTruthy()
    await user.click(backdrop as HTMLElement)
    expect(onClose).toHaveBeenCalled()
  })

  it('moves focus to the close button on open', async () => {
    const user = userEvent.setup()
    render(
      <PlayerProvider>
        <Harness open={true} onClose={() => {}} />
      </PlayerProvider>,
    )
    await user.click(screen.getByRole('button', { name: /seed/i }))
    await waitFor(() => {
      expect(document.activeElement).toBe(
        screen.getByRole('button', { name: /close now playing/i }),
      )
    })
  })
})
