// CreatePlaylistDialog tests — exercise the open/close lifecycle,
// the client-side validation gate, the success path that fires
// onCreated, the surfaced server error, and the cancel button.

import { describe, expect, it, vi } from 'vitest'
import { render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'

const createPlaylist = vi.fn()

vi.mock('@/server-fns/playlists', () => ({
  createPlaylist: (arg: unknown) => createPlaylist(arg),
}))

const { CreatePlaylistDialog } = await import('./CreatePlaylistDialog')

function makePlaylist(overrides: Record<string, unknown> = {}) {
  return {
    id: 42,
    name: 'Mock',
    description: null,
    color_id: 'violet',
    icon_id: 'music',
    is_smart: 0,
    cover_hash: null,
    cover_is_auto: 1,
    position: 1,
    created_at: 0,
    updated_at: 0,
    track_count: 0,
    total_duration_ms: 0,
    smart_rules: null,
    ...overrides,
  }
}

describe('CreatePlaylistDialog', () => {
  it('renders nothing when closed', () => {
    render(
      <CreatePlaylistDialog open={false} profileId={1} onClose={() => {}} onCreated={() => {}} />,
    )
    expect(screen.queryByRole('dialog')).toBeNull()
  })

  it('renders the form when open + autofocuses the name input', async () => {
    render(
      <CreatePlaylistDialog open={true} profileId={1} onClose={() => {}} onCreated={() => {}} />,
    )
    expect(screen.getByRole('dialog', { name: /create playlist/i })).toBeTruthy()
    await waitFor(() => {
      expect(document.activeElement).toBe(screen.getByRole('textbox', { name: /name/i }))
    })
  })

  it('blocks submit + shows an alert when the name is empty', async () => {
    const user = userEvent.setup()
    const onCreated = vi.fn()
    render(
      <CreatePlaylistDialog open={true} profileId={1} onClose={() => {}} onCreated={onCreated} />,
    )
    await user.click(screen.getByRole('button', { name: /create playlist/i }))
    const alert = await screen.findByRole('alert')
    expect(alert.textContent).toMatch(/name is required/i)
    expect(createPlaylist).not.toHaveBeenCalled()
    expect(onCreated).not.toHaveBeenCalled()
  })

  it('calls createPlaylist + onCreated on a valid submit', async () => {
    const user = userEvent.setup()
    const onCreated = vi.fn()
    const created = makePlaylist({ id: 77, name: 'Morning ride' })
    createPlaylist.mockResolvedValueOnce(created)

    render(
      <CreatePlaylistDialog open={true} profileId={1} onClose={() => {}} onCreated={onCreated} />,
    )
    await user.type(screen.getByRole('textbox', { name: /name/i }), 'Morning ride')
    await user.type(screen.getByRole('textbox', { name: /description/i }), 'Quiet drives.')
    await user.click(screen.getByRole('button', { name: /create playlist/i }))

    await waitFor(() => {
      expect(createPlaylist).toHaveBeenCalledWith({
        data: { profileId: 1, name: 'Morning ride', description: 'Quiet drives.' },
      })
      expect(onCreated).toHaveBeenCalledWith(created)
    })
  })

  it('surfaces a server error + does not fire onCreated', async () => {
    const user = userEvent.setup()
    const onCreated = vi.fn()
    createPlaylist.mockRejectedValueOnce(new Error('Server is down.'))

    render(
      <CreatePlaylistDialog open={true} profileId={1} onClose={() => {}} onCreated={onCreated} />,
    )
    await user.type(screen.getByRole('textbox', { name: /name/i }), 'X')
    await user.click(screen.getByRole('button', { name: /create playlist/i }))

    const alert = await screen.findByRole('alert')
    expect(alert.textContent).toMatch(/server is down/i)
    expect(onCreated).not.toHaveBeenCalled()
  })

  it('fires onClose when the user presses Escape', async () => {
    const user = userEvent.setup()
    const onClose = vi.fn()
    render(
      <CreatePlaylistDialog open={true} profileId={1} onClose={onClose} onCreated={() => {}} />,
    )
    await user.keyboard('{Escape}')
    expect(onClose).toHaveBeenCalled()
  })

  it('fires onClose when the user clicks Cancel', async () => {
    const user = userEvent.setup()
    const onClose = vi.fn()
    render(
      <CreatePlaylistDialog open={true} profileId={1} onClose={onClose} onCreated={() => {}} />,
    )
    await user.click(screen.getByRole('button', { name: /cancel/i }))
    expect(onClose).toHaveBeenCalled()
  })

  it('fires onClose when the user clicks the backdrop', async () => {
    const user = userEvent.setup()
    const onClose = vi.fn()
    render(
      <CreatePlaylistDialog open={true} profileId={1} onClose={onClose} onCreated={() => {}} />,
    )
    const backdrop = document.querySelector('[aria-hidden="true"].fixed')
    expect(backdrop).toBeTruthy()
    await user.click(backdrop as HTMLElement)
    expect(onClose).toHaveBeenCalled()
  })

  it('omits a whitespace-only description from the server-fn payload', async () => {
    const user = userEvent.setup()
    createPlaylist.mockResolvedValueOnce(makePlaylist())
    render(
      <CreatePlaylistDialog open={true} profileId={1} onClose={() => {}} onCreated={() => {}} />,
    )
    await user.type(screen.getByRole('textbox', { name: /name/i }), 'Bare')
    await user.type(screen.getByRole('textbox', { name: /description/i }), '   ')
    await user.click(screen.getByRole('button', { name: /create playlist/i }))
    await waitFor(() => {
      expect(createPlaylist).toHaveBeenCalledWith({
        data: { profileId: 1, name: 'Bare' },
      })
    })
  })

  it('resets the form fields when the dialog reopens', async () => {
    const user = userEvent.setup()
    const { rerender } = render(
      <CreatePlaylistDialog open={true} profileId={1} onClose={() => {}} onCreated={() => {}} />,
    )
    const nameInput = screen.getByRole('textbox', { name: /name/i }) as HTMLInputElement
    await user.type(nameInput, 'Stale draft')
    expect(nameInput.value).toBe('Stale draft')

    // Close the dialog…
    rerender(
      <CreatePlaylistDialog open={false} profileId={1} onClose={() => {}} onCreated={() => {}} />,
    )
    // …then reopen it. The adjust-state-on-prop-change reset
    // should clear the previous draft before the user types again.
    rerender(
      <CreatePlaylistDialog open={true} profileId={1} onClose={() => {}} onCreated={() => {}} />,
    )
    expect((screen.getByRole('textbox', { name: /name/i }) as HTMLInputElement).value).toBe('')
  })
})
