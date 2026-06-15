// PlaylistDetailView render tests. Mocks the file-route so we
// can mount the component without spinning up the router.

import type { PropsWithChildren } from 'react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'

// Typed against the route's `LoaderData` discriminated union so a
// future rename of `tracksResult` (or shape drift in the loader)
// breaks at compile time rather than slipping through as a runtime
// "page is blank" surprise. Initial null is the same shape as
// `useLoaderData` would return mid-suspense — tests reset it
// before each spec.
import type { LoaderData } from './_authed.profiles.$profileId.playlists.$playlistId'

let loaderData: LoaderData | null = null
const navigate = vi.fn()
const invalidate = vi.fn().mockResolvedValue(undefined)

vi.mock('@tanstack/react-router', () => ({
  createFileRoute: () => (config: unknown) => ({
    ...(config as Record<string, unknown>),
    useLoaderData: () => loaderData,
  }),
  useRouter: () => ({ invalidate }),
  useNavigate: () => navigate,
  Link: ({ children, ...rest }: PropsWithChildren<Record<string, unknown>>) => (
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
  getPlaylistTracks: vi.fn(),
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

function emptyTracks() {
  return { ok: true as const, tracks: [] }
}

beforeEach(() => {
  // Reset loaderData FIRST so a test that forgets to set it
  // doesn't accidentally inherit the previous spec's fixture —
  // the module-scoped variable would otherwise leak across
  // tests and mask "I forgot to set loaderData" bugs.
  loaderData = null
  navigate.mockReset()
  invalidate.mockReset().mockResolvedValue(undefined)
  updatePlaylist.mockReset()
  deletePlaylist.mockReset()
})

describe('PlaylistDetailView', () => {
  it('renders the playlist name + description + the empty-tracks notice', () => {
    loaderData = {
      kind: 'ready',
      profileId: 1,
      playlist: makePlaylist(),
      tracksResult: emptyTracks(),
    }
    render(<PlaylistDetailView />)
    expect(screen.getByRole('heading', { name: /hero playlist/i })).toBeTruthy()
    expect(screen.getByText(/a long quiet road/i)).toBeTruthy()
    expect(screen.getByText(/no tracks yet/i)).toBeTruthy()
  })

  it('shows the Smart label for smart playlists', () => {
    loaderData = {
      kind: 'ready',
      profileId: 1,
      playlist: makePlaylist({ is_smart: 1, name: 'Daily Mix 2' }),
      tracksResult: emptyTracks(),
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
      tracksResult: emptyTracks(),
    }
    render(<PlaylistDetailView />)
    expect(screen.getByRole('button', { name: /^edit$/i })).toBeTruthy()
    expect(screen.getByRole('button', { name: /^delete$/i })).toBeTruthy()
  })

  it('hides the Edit + Delete buttons on a smart playlist and never calls the mutating fns', () => {
    loaderData = {
      kind: 'ready',
      profileId: 1,
      playlist: makePlaylist({ is_smart: 1, name: 'Daily Mix 2' }),
      tracksResult: emptyTracks(),
    }
    render(<PlaylistDetailView />)
    expect(screen.queryByRole('button', { name: /^edit$/i })).toBeNull()
    expect(screen.queryByRole('button', { name: /^delete$/i })).toBeNull()
    // Defense-in-depth: even if a future render-time bug surfaces a
    // phantom mutation (e.g. an unmounted dialog firing onSubmitted
    // from a stale closure), the network never gets called.
    expect(updatePlaylist).not.toHaveBeenCalled()
    expect(deletePlaylist).not.toHaveBeenCalled()
  })

  it('clicking Edit + submitting the form invalidates the loader', async () => {
    const user = userEvent.setup()
    loaderData = {
      kind: 'ready',
      profileId: 1,
      playlist: makePlaylist({ id: 9 }),
      tracksResult: emptyTracks(),
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
      tracksResult: emptyTracks(),
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

describe('PlaylistDetailView — TrackList', () => {
  it('renders track rows with title, artist, and formatted duration', () => {
    loaderData = {
      kind: 'ready',
      profileId: 1,
      playlist: makePlaylist(),
      tracksResult: {
        ok: true,
        tracks: [
          {
            track_id: 901,
            position: 0,
            added_at: 1_700_000_000_000,
            snapshot_title: 'One More Time',
            snapshot_artist: 'Daft Punk',
            snapshot_duration_ms: 320_000,
          },
          {
            track_id: 902,
            position: 1,
            added_at: 1_700_000_000_001,
            snapshot_title: 'Around the World',
            snapshot_artist: 'Daft Punk',
            snapshot_duration_ms: 215_000,
          },
        ],
      },
    }
    render(<PlaylistDetailView />)
    expect(screen.getByText('One More Time')).toBeTruthy()
    expect(screen.getByText('Around the World')).toBeTruthy()
    // Both tracks credit Daft Punk → multiple matches expected.
    expect(screen.getAllByText('Daft Punk')).toHaveLength(2)
    // formatTime renders mm:ss; the artwork-test convention here is
    // a substring match so a trailing space / wrapper character
    // doesn't break the assertion.
    expect(screen.getByText(/5:20/)).toBeTruthy() // 320s
    expect(screen.getByText(/3:35/)).toBeTruthy() // 215s
    // No "no tracks yet" placeholder when tracks are present.
    expect(screen.queryByText(/no tracks yet/i)).toBeNull()
  })

  it('renders a placeholder for rows whose snapshot is NULL, keyed by ordinal not by wire id', () => {
    loaderData = {
      kind: 'ready',
      profileId: 1,
      playlist: makePlaylist(),
      tracksResult: {
        ok: true,
        tracks: [
          {
            track_id: 808,
            position: 0,
            added_at: 1_700_000_000_000,
            snapshot_title: null,
            snapshot_artist: null,
            snapshot_duration_ms: null,
          },
        ],
      },
    }
    render(<PlaylistDetailView />)
    // The owner is allowed to see pre-1.j.b rows; the UI shows a
    // "Track <ordinal>" placeholder rather than hiding them. The
    // ordinal — not the wire `track_id` — is what surfaces, so
    // the desktop's local i64 row id doesn't leak into the UI.
    expect(screen.getByText('Track 1')).toBeTruthy()
    expect(screen.queryByText(/Track #808/i)).toBeNull()
  })

  it('numbers track ordinals 1..N even when wire positions are sparse', () => {
    // Locks in the contract that the rendered ordinal follows the
    // array order, NOT the wire `position`. A sparse `position`
    // sequence (from prior deletes) MUST still produce 1, 2, 3 to
    // the user — mainstream player convention. A regression that
    // started rendering `track.position + 1` would surface as
    // "6, 8, 13" here.
    loaderData = {
      kind: 'ready',
      profileId: 1,
      playlist: makePlaylist(),
      tracksResult: {
        ok: true,
        tracks: [
          {
            track_id: 1001,
            position: 5,
            added_at: 1_700_000_000_000,
            snapshot_title: 'First',
            snapshot_artist: null,
            snapshot_duration_ms: null,
          },
          {
            track_id: 1002,
            position: 7,
            added_at: 1_700_000_000_001,
            snapshot_title: 'Second',
            snapshot_artist: null,
            snapshot_duration_ms: null,
          },
          {
            track_id: 1003,
            position: 12,
            added_at: 1_700_000_000_002,
            snapshot_title: 'Third',
            snapshot_artist: null,
            snapshot_duration_ms: null,
          },
        ],
      },
    }
    render(<PlaylistDetailView />)
    // Ordinals are aria-hidden but render as visible text — the
    // assertion checks the DOM text, not the accessibility tree.
    const items = screen.getAllByRole('listitem')
    expect(items[0].textContent).toMatch(/^\s*1\s*First/)
    expect(items[1].textContent).toMatch(/^\s*2\s*Second/)
    expect(items[2].textContent).toMatch(/^\s*3\s*Third/)
  })

  it('surfaces a track-fetch failure as an inline alert without breaking the metadata render', () => {
    loaderData = {
      kind: 'ready',
      profileId: 1,
      playlist: makePlaylist(),
      tracksResult: { ok: false, error: 'Server is down.' },
    }
    render(<PlaylistDetailView />)
    // Metadata still renders — the user can still edit / delete /
    // navigate out even when the track-list query failed.
    expect(screen.getByRole('heading', { name: /hero playlist/i })).toBeTruthy()
    const alert = screen.getByRole('alert')
    expect(alert.textContent).toMatch(/could not load tracks/i)
    expect(alert.textContent).toMatch(/server is down/i)
  })
})
