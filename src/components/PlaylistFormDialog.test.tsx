// PlaylistFormDialog tests — exercise both create + edit modes,
// the validation gate, the submit success + rejection paths, the
// ESC / Cancel / backdrop dismissals, the form reset on reopen,
// the initial-value plumbing, and the per-mode submit labels.

import { describe, expect, it, vi } from 'vitest'
import { render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'

import { PlaylistFormDialog } from './PlaylistFormDialog'

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

describe('PlaylistFormDialog — create mode', () => {
  it('renders nothing when closed', () => {
    render(
      <PlaylistFormDialog
        open={false}
        mode="create"
        onClose={() => {}}
        submit={vi.fn()}
        onSubmitted={() => {}}
      />,
    )
    expect(screen.queryByRole('dialog')).toBeNull()
  })

  it('renders the create form when open + autofocuses the name input', async () => {
    render(
      <PlaylistFormDialog
        open={true}
        mode="create"
        onClose={() => {}}
        submit={vi.fn()}
        onSubmitted={() => {}}
      />,
    )
    expect(screen.getByRole('dialog', { name: /create playlist/i })).toBeTruthy()
    expect(screen.getByRole('button', { name: /create playlist/i })).toBeTruthy()
    await waitFor(() => {
      expect(document.activeElement).toBe(screen.getByRole('textbox', { name: /name/i }))
    })
  })

  it('blocks submit + shows an alert when the name is empty', async () => {
    const user = userEvent.setup()
    const submit = vi.fn()
    const onSubmitted = vi.fn()
    render(
      <PlaylistFormDialog
        open={true}
        mode="create"
        onClose={() => {}}
        submit={submit}
        onSubmitted={onSubmitted}
      />,
    )
    await user.click(screen.getByRole('button', { name: /create playlist/i }))
    const alert = await screen.findByRole('alert')
    expect(alert.textContent).toMatch(/name is required/i)
    expect(submit).not.toHaveBeenCalled()
    expect(onSubmitted).not.toHaveBeenCalled()
  })

  it('calls submit + onSubmitted on a valid submission', async () => {
    const user = userEvent.setup()
    const onSubmitted = vi.fn()
    const created = makePlaylist({ id: 77, name: 'Morning ride' })
    const submit = vi.fn().mockResolvedValueOnce(created)
    render(
      <PlaylistFormDialog
        open={true}
        mode="create"
        onClose={() => {}}
        submit={submit}
        onSubmitted={onSubmitted}
      />,
    )
    await user.type(screen.getByRole('textbox', { name: /name/i }), 'Morning ride')
    await user.type(screen.getByRole('textbox', { name: /description/i }), 'Quiet drives.')
    await user.click(screen.getByRole('button', { name: /create playlist/i }))

    await waitFor(() => {
      expect(submit).toHaveBeenCalledWith({
        name: 'Morning ride',
        description: 'Quiet drives.',
      })
      expect(onSubmitted).toHaveBeenCalledWith(created)
    })
  })

  it('omits a whitespace-only description in create mode', async () => {
    const user = userEvent.setup()
    const submit = vi.fn().mockResolvedValueOnce(makePlaylist())
    render(
      <PlaylistFormDialog
        open={true}
        mode="create"
        onClose={() => {}}
        submit={submit}
        onSubmitted={() => {}}
      />,
    )
    await user.type(screen.getByRole('textbox', { name: /name/i }), 'Bare')
    await user.type(screen.getByRole('textbox', { name: /description/i }), '   ')
    await user.click(screen.getByRole('button', { name: /create playlist/i }))
    await waitFor(() => {
      expect(submit).toHaveBeenCalledWith({ name: 'Bare' })
    })
  })

  it('surfaces a submit rejection + does not fire onSubmitted', async () => {
    const user = userEvent.setup()
    const onSubmitted = vi.fn()
    const submit = vi.fn().mockRejectedValueOnce(new Error('Server is down.'))
    render(
      <PlaylistFormDialog
        open={true}
        mode="create"
        onClose={() => {}}
        submit={submit}
        onSubmitted={onSubmitted}
      />,
    )
    await user.type(screen.getByRole('textbox', { name: /name/i }), 'X')
    await user.click(screen.getByRole('button', { name: /create playlist/i }))
    const alert = await screen.findByRole('alert')
    expect(alert.textContent).toMatch(/server is down/i)
    expect(onSubmitted).not.toHaveBeenCalled()
  })

  it('fires onClose when the user presses Escape', async () => {
    const user = userEvent.setup()
    const onClose = vi.fn()
    render(
      <PlaylistFormDialog
        open={true}
        mode="create"
        onClose={onClose}
        submit={vi.fn()}
        onSubmitted={() => {}}
      />,
    )
    await user.keyboard('{Escape}')
    expect(onClose).toHaveBeenCalled()
  })

  it('fires onClose when the user clicks Cancel', async () => {
    const user = userEvent.setup()
    const onClose = vi.fn()
    render(
      <PlaylistFormDialog
        open={true}
        mode="create"
        onClose={onClose}
        submit={vi.fn()}
        onSubmitted={() => {}}
      />,
    )
    await user.click(screen.getByRole('button', { name: /cancel/i }))
    expect(onClose).toHaveBeenCalled()
  })

  it('fires onClose when the user clicks the backdrop', async () => {
    const user = userEvent.setup()
    const onClose = vi.fn()
    render(
      <PlaylistFormDialog
        open={true}
        mode="create"
        onClose={onClose}
        submit={vi.fn()}
        onSubmitted={() => {}}
      />,
    )
    const backdrop = document.querySelector('[aria-hidden="true"].fixed')
    expect(backdrop).toBeTruthy()
    await user.click(backdrop as HTMLElement)
    expect(onClose).toHaveBeenCalled()
  })

  it('resets to the initial values when the dialog reopens', async () => {
    const user = userEvent.setup()
    const { rerender } = render(
      <PlaylistFormDialog
        open={true}
        mode="create"
        onClose={() => {}}
        submit={vi.fn()}
        onSubmitted={() => {}}
      />,
    )
    const nameInput = screen.getByRole('textbox', { name: /name/i }) as HTMLInputElement
    await user.type(nameInput, 'Stale draft')
    rerender(
      <PlaylistFormDialog
        open={false}
        mode="create"
        onClose={() => {}}
        submit={vi.fn()}
        onSubmitted={() => {}}
      />,
    )
    rerender(
      <PlaylistFormDialog
        open={true}
        mode="create"
        onClose={() => {}}
        submit={vi.fn()}
        onSubmitted={() => {}}
      />,
    )
    expect((screen.getByRole('textbox', { name: /name/i }) as HTMLInputElement).value).toBe('')
  })
})

describe('PlaylistFormDialog — edit mode', () => {
  it('renders the edit copy + initial values', () => {
    render(
      <PlaylistFormDialog
        open={true}
        mode="edit"
        initial={{ name: 'Hero', description: 'A long quiet road.' }}
        onClose={() => {}}
        submit={vi.fn()}
        onSubmitted={() => {}}
      />,
    )
    expect(screen.getByRole('dialog', { name: /edit playlist/i })).toBeTruthy()
    expect(screen.getByRole('button', { name: /save changes/i })).toBeTruthy()
    expect((screen.getByRole('textbox', { name: /name/i }) as HTMLInputElement).value).toBe('Hero')
    expect(
      (screen.getByRole('textbox', { name: /description/i }) as HTMLTextAreaElement).value,
    ).toBe('A long quiet road.')
  })

  it('forwards a CLEARED description as empty string so the server overwrites', async () => {
    const user = userEvent.setup()
    const submit = vi.fn().mockResolvedValueOnce(makePlaylist())
    render(
      <PlaylistFormDialog
        open={true}
        mode="edit"
        initial={{ name: 'Hero', description: 'old' }}
        onClose={() => {}}
        submit={submit}
        onSubmitted={() => {}}
      />,
    )
    const desc = screen.getByRole('textbox', { name: /description/i })
    await user.clear(desc)
    await user.click(screen.getByRole('button', { name: /save changes/i }))
    await waitFor(() => {
      expect(submit).toHaveBeenCalledWith({ name: 'Hero', description: '' })
    })
  })

  it('shows the busy label while the submit promise is pending', async () => {
    const user = userEvent.setup()
    let resolve: ((p: ReturnType<typeof makePlaylist>) => void) | undefined
    const submit = vi.fn(
      () =>
        new Promise<ReturnType<typeof makePlaylist>>((r) => {
          resolve = r
        }),
    )
    render(
      <PlaylistFormDialog
        open={true}
        mode="edit"
        initial={{ name: 'Hero' }}
        onClose={() => {}}
        submit={submit}
        onSubmitted={() => {}}
      />,
    )
    await user.click(screen.getByRole('button', { name: /save changes/i }))
    expect(screen.getByRole('button', { name: /saving/i })).toBeTruthy()
    resolve?.(makePlaylist())
  })
})
