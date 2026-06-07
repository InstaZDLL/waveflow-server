// DeletePlaylistDialog tests — closed/open lifecycle, autofocus
// lands on Cancel (not on the destructive button), the submit
// success path, the rejection path, the various dismissals.

import { describe, expect, it, vi } from 'vitest'
import { render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'

import { DeletePlaylistDialog } from './DeletePlaylistDialog'

describe('DeletePlaylistDialog', () => {
  it('renders nothing when closed', () => {
    render(
      <DeletePlaylistDialog
        open={false}
        playlistName="Test"
        onClose={() => {}}
        submit={vi.fn()}
        onDeleted={() => {}}
      />,
    )
    expect(screen.queryByRole('dialog')).toBeNull()
  })

  it('renders the confirmation copy with the playlist name', () => {
    render(
      <DeletePlaylistDialog
        open={true}
        playlistName="Hero playlist"
        onClose={() => {}}
        submit={vi.fn()}
        onDeleted={() => {}}
      />,
    )
    expect(screen.getByRole('dialog', { name: /delete playlist/i })).toBeTruthy()
    expect(screen.getByText('Hero playlist')).toBeTruthy()
    expect(screen.getByRole('button', { name: /^delete playlist$/i })).toBeTruthy()
    expect(screen.getByRole('button', { name: /cancel/i })).toBeTruthy()
  })

  it('autofocuses the Cancel button so the destructive choice is not one keystroke away', async () => {
    render(
      <DeletePlaylistDialog
        open={true}
        playlistName="X"
        onClose={() => {}}
        submit={vi.fn()}
        onDeleted={() => {}}
      />,
    )
    await waitFor(() => {
      expect(document.activeElement).toBe(screen.getByRole('button', { name: /cancel/i }))
    })
  })

  it('calls submit + onDeleted on confirm', async () => {
    const user = userEvent.setup()
    const submit = vi.fn().mockResolvedValueOnce(undefined)
    const onDeleted = vi.fn()
    render(
      <DeletePlaylistDialog
        open={true}
        playlistName="Doomed"
        onClose={() => {}}
        submit={submit}
        onDeleted={onDeleted}
      />,
    )
    await user.click(screen.getByRole('button', { name: /^delete playlist$/i }))
    await waitFor(() => {
      expect(submit).toHaveBeenCalled()
      expect(onDeleted).toHaveBeenCalled()
    })
  })

  it('surfaces a submit rejection + leaves the dialog open', async () => {
    const user = userEvent.setup()
    const submit = vi.fn().mockRejectedValueOnce(new Error('Not allowed.'))
    const onDeleted = vi.fn()
    render(
      <DeletePlaylistDialog
        open={true}
        playlistName="Untouchable"
        onClose={() => {}}
        submit={submit}
        onDeleted={onDeleted}
      />,
    )
    await user.click(screen.getByRole('button', { name: /^delete playlist$/i }))
    const alert = await screen.findByRole('alert')
    expect(alert.textContent).toMatch(/not allowed/i)
    expect(onDeleted).not.toHaveBeenCalled()
    expect(screen.getByRole('dialog')).toBeTruthy()
  })

  it('fires onClose on Escape / Cancel / backdrop', async () => {
    const user = userEvent.setup()
    const onClose = vi.fn()

    const { rerender } = render(
      <DeletePlaylistDialog
        open={true}
        playlistName="X"
        onClose={onClose}
        submit={vi.fn()}
        onDeleted={() => {}}
      />,
    )
    await user.keyboard('{Escape}')
    expect(onClose).toHaveBeenCalledTimes(1)

    rerender(
      <DeletePlaylistDialog
        open={true}
        playlistName="X"
        onClose={onClose}
        submit={vi.fn()}
        onDeleted={() => {}}
      />,
    )
    await user.click(screen.getByRole('button', { name: /cancel/i }))
    expect(onClose).toHaveBeenCalledTimes(2)

    const backdrop = document.querySelector('[aria-hidden="true"].fixed')
    await user.click(backdrop as HTMLElement)
    expect(onClose).toHaveBeenCalledTimes(3)
  })
})
