// PlaylistDetailView render tests. Mocks the file-route so we
// can mount the component without spinning up the router.

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'

let loaderData: unknown = null
const navigate = vi.fn()
const invalidate = vi.fn().mockResolvedValue(undefined)

vi.mock('@tanstack/react-router', () => ({
  createFileRoute: () => (config: unknown) => ({
    ...(config as Record<string, unknown>),
    useLoaderData: () => loaderData,
  }),
  useRouter: () => ({ invalidate }),
  useNavigate: () => navigate,
  Link: ({ children, ...rest }: React.PropsWithChildren<Record<string, unknown>>) => (
    <a {...rest}>{children}</a>
  ),
}))

// The route file imports the server-fns at the module top, which
// transitively pulls in the server-fn internals (DB driver, JWT
// mint) — none of which is appropriate for a unit-test render.
const updatePlaylist = vi.fn()
const deletePlaylist = vi.fn()
vi.mock('@/server-fns/playlists', () => ({
  getPlaylist: vi.fn(),
  updatePlaylist: (arg: unknown) => updatePlaylist(arg),
  deletePlaylist: (arg: unknown) => deletePlaylist(arg),
}))

// Replace the real dialogs with interactive harnesses that expose
// "fire submit" buttons. The detail route doesn't need to test the
// dialog's a11y / focus-trap surface — those are covered by the
// dialog suites — but it DOES need to test the integration flow:
// clicking Edit / Delete must reach the route's onEdited /
// onDeleted callbacks, which fire invalidate + navigate.
vi.mock('@/components/PlaylistFormDialog', () => ({
  PlaylistFormDialog: ({
    open,
    submit,
    onSubmitted,
  }: {
    open: boolean
    submit: (values: { name: string; description?: string }) => Promise<unknown>
    onSubmitted: (playlist: unknown) => void
  }) =>
    open ? (
      <button
        type="button"
        data-testid="form-submit"
        onClick={async () => {
          const playlist = await submit({ name: 'Edited' })
          onSubmitted(playlist)
        }}
      >
        fire-submit
      </button>
    ) : null,
}))
vi.mock('@/components/DeletePlaylistDialog', () => ({
  DeletePlaylistDialog: ({
    open,
    submit,
    onDeleted,
  }: {
    open: boolean
    submit: () => Promise<void>
    onDeleted: () => void
  }) =>
    open ? (
      <button
        type="button"
        data-testid="delete-submit"
        onClick={async () => {
          await submit()
          onDeleted()
        }}
      >
        fire-delete
      </button>
    ) : null,
}))

const { PlaylistDetailView } = await import('./_authed.profiles.$profileId.playlists.$playlistId')

function makePlaylist(overrides: Record<string, unknown> = {}) {
  return {
    id: 7,
    name: 'Hero playlist',
    description: 'A long quiet road.',
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

beforeEach(() => {
  navigate.mockReset()
  invalidate.mockReset().mockResolvedValue(undefined)
  updatePlaylist.mockReset()
  deletePlaylist.mockReset()
})

describe('PlaylistDetailView', () => {
  it('renders the playlist name + description + the tracks-coming-soon notice', () => {
    loaderData = {
      kind: 'ready',
      profileId: 1,
      playlist: makePlaylist(),
    }
    render(<PlaylistDetailView />)
    expect(screen.getByRole('heading', { name: /hero playlist/i })).toBeTruthy()
    expect(screen.getByText(/a long quiet road/i)).toBeTruthy()
    expect(screen.getByText(/tracks coming soon/i)).toBeTruthy()
  })

  it('shows the Smart label for smart playlists', () => {
    loaderData = {
      kind: 'ready',
      profileId: 1,
      playlist: makePlaylist({ is_smart: 1, name: 'Daily Mix 2' }),
    }
    render(<PlaylistDetailView />)
    // Kicker reads "Smart playlist" + the badge reads "Smart" —
    // two matches confirm both surfaces are tagged.
    expect(screen.getByText(/smart playlist/i)).toBeTruthy()
    expect(screen.getAllByText(/^smart$/i)).toHaveLength(1)
  })

  it('renders the loader error path', () => {
    loaderData = {
      kind: 'error',
      message: 'Not found.',
    }
    render(<PlaylistDetailView />)
    const alert = screen.getByRole('alert')
    expect(alert.textContent).toMatch(/not found/i)
  })

  it('shows the Edit + Delete buttons on a non-smart playlist', () => {
    loaderData = {
      kind: 'ready',
      profileId: 1,
      playlist: makePlaylist(),
    }
    render(<PlaylistDetailView />)
    expect(screen.getByRole('button', { name: /^edit$/i })).toBeTruthy()
    expect(screen.getByRole('button', { name: /^delete$/i })).toBeTruthy()
  })

  it('hides the Edit + Delete buttons on a smart playlist', () => {
    loaderData = {
      kind: 'ready',
      profileId: 1,
      playlist: makePlaylist({ is_smart: 1, name: 'Daily Mix 2' }),
    }
    render(<PlaylistDetailView />)
    expect(screen.queryByRole('button', { name: /^edit$/i })).toBeNull()
    expect(screen.queryByRole('button', { name: /^delete$/i })).toBeNull()
  })

  it('clicking Edit + submitting the form invalidates the loader', async () => {
    const user = userEvent.setup()
    loaderData = {
      kind: 'ready',
      profileId: 1,
      playlist: makePlaylist({ id: 9 }),
    }
    updatePlaylist.mockResolvedValueOnce(makePlaylist({ id: 9, name: 'Edited' }))
    render(<PlaylistDetailView />)
    await user.click(screen.getByRole('button', { name: /^edit$/i }))
    await user.click(screen.getByTestId('form-submit'))
    await waitFor(() => {
      expect(updatePlaylist).toHaveBeenCalledWith({
        data: { profileId: 1, playlistId: 9, name: 'Edited' },
      })
      expect(invalidate).toHaveBeenCalled()
    })
  })

  it('clicking Delete + confirming invalidates + navigates back to the listing', async () => {
    const user = userEvent.setup()
    loaderData = {
      kind: 'ready',
      profileId: 1,
      playlist: makePlaylist({ id: 9 }),
    }
    deletePlaylist.mockResolvedValueOnce(undefined)
    render(<PlaylistDetailView />)
    await user.click(screen.getByRole('button', { name: /^delete$/i }))
    await user.click(screen.getByTestId('delete-submit'))
    await waitFor(() => {
      expect(deletePlaylist).toHaveBeenCalledWith({
        data: { profileId: 1, playlistId: 9 },
      })
      expect(invalidate).toHaveBeenCalled()
      expect(navigate).toHaveBeenCalledWith({
        to: '/profiles/$profileId/playlists',
        params: { profileId: '1' },
      })
    })
  })
})
