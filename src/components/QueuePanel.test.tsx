// QueuePanel — render + interaction smoke tests. Exercises the
// open/close lifecycle, the queue-row jump action, the empty
// state, and the ESC-to-close keyboard handler.

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
const { QueuePanel } = await import('./QueuePanel')

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
          await player.playTrack(makeEntry(1), [makeEntry(2), makeEntry(3)])
        }}
      >
        seed
      </button>
      <QueuePanel open={open} onClose={onClose} />
    </>
  )
}

describe('QueuePanel', () => {
  it('renders nothing when closed', () => {
    render(
      <PlayerProvider>
        <Harness open={false} onClose={() => {}} />
      </PlayerProvider>,
    )
    expect(screen.queryByRole('dialog')).toBeNull()
  })

  it('renders the dialog + close button when open', () => {
    render(
      <PlayerProvider>
        <Harness open={true} onClose={() => {}} />
      </PlayerProvider>,
    )
    expect(screen.getByRole('dialog', { name: /playback queue/i })).toBeTruthy()
    expect(screen.getByRole('button', { name: /close queue/i })).toBeTruthy()
  })

  it('renders the empty-queue copy when the queue is empty', () => {
    render(
      <PlayerProvider>
        <Harness open={true} onClose={() => {}} />
      </PlayerProvider>,
    )
    expect(screen.getByText(/the queue is empty/i)).toBeTruthy()
  })

  it('lists Now playing + Next up + Recently played sections after a play', async () => {
    const user = userEvent.setup()
    render(
      <PlayerProvider>
        <Harness open={true} onClose={() => {}} />
      </PlayerProvider>,
    )
    await user.click(screen.getByRole('button', { name: /seed/i }))

    // Wait for the autoplay state to flush: the seed click awaits
    // playTrack so by the time it returns, the queue + current are
    // already in place. Assert directly.
    expect(screen.getByText(/now playing/i)).toBeTruthy()
    expect(screen.getByText(/next up/i)).toBeTruthy()
    // Track 1 is the current, 2 + 3 fill the queue. History is
    // empty so the "Recently played" header is hidden.
    expect(screen.queryByText(/recently played/i)).toBeNull()
  })

  it('clicking a queue row jumps via playQueueAt', async () => {
    const user = userEvent.setup()
    render(
      <PlayerProvider>
        <Harness open={true} onClose={() => {}} />
      </PlayerProvider>,
    )
    await user.click(screen.getByRole('button', { name: /seed/i }))

    // Click "Jump to Track 3" → playQueueAt(1) → current=3, queue=[],
    // history=[1, 2].
    await user.click(screen.getByRole('button', { name: /jump to track 3/i }))

    // After the jump, "Recently played" appears with tracks 1 + 2.
    expect(screen.getByText(/recently played/i)).toBeTruthy()
  })

  it('fires onClose when the user presses Escape', async () => {
    const user = userEvent.setup()
    const onClose = vi.fn()
    render(
      <PlayerProvider>
        <Harness open={true} onClose={onClose} />
      </PlayerProvider>,
    )
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
    // The backdrop is the dimmed div behind the dialog — it has
    // aria-hidden so we reach it via its class signature.
    const backdrop = document.querySelector('[aria-hidden="true"].fixed')
    expect(backdrop).toBeTruthy()
    await user.click(backdrop as HTMLElement)
    expect(onClose).toHaveBeenCalled()
  })
})
