// PlaylistsView render tests. Mocks the file-route so we can mount
// the component without spinning up the router; the loader's
// shape (`{ kind: 'ready' | 'error' }`) is what useLoaderData
// returns, which we feed directly.

import { describe, expect, it, vi } from 'vitest'
import { render, screen, within } from '@testing-library/react'

// Loader-data fixture driven by each spec — captured here so the
// `createFileRoute` mock's `useLoaderData` returns whatever the
// current test set without re-creating the Route object every
// time (the file route is module-scoped, so `vi.spyOn(Route, ...)`
// only sees the value at first import).
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

// Stub the form dialog so the listing unit-test doesn't drag
// its own (focus-trap, theming) deps into the listing render.
vi.mock('@/components/PlaylistFormDialog', () => ({
  PlaylistFormDialog: () => null,
}))

// The route file imports `listPlaylists` at the module top, which
// transitively pulls in the server-fn internals (DB driver, JWT
// mint) — none of which is appropriate for a unit-test render.
vi.mock('@/server-fns/playlists', () => ({
  listPlaylists: vi.fn(),
}))

const { PlaylistsView } = await import('./_authed.profiles.$profileId.playlists')

function makePlaylist(id: number, overrides: Record<string, unknown> = {}) {
  return {
    id,
    name: `Playlist ${id}`,
    description: null,
    color_id: 'violet',
    icon_id: 'music',
    is_smart: 0,
    cover_hash: null,
    cover_is_auto: 1,
    position: id,
    created_at: 0,
    updated_at: 0,
    track_count: 0,
    total_duration_ms: 0,
    smart_rules: null,
    ...overrides,
  }
}

describe('PlaylistsView', () => {
  it('renders the empty-state copy when the user has no playlists', () => {
    loaderData = {
      kind: 'ready',
      profileId: 1,
      playlists: [],
    }
    render(<PlaylistsView />)
    expect(screen.getByText(/no playlists yet/i)).toBeTruthy()
  })

  it('renders one card per playlist with the track-count + duration', () => {
    loaderData = {
      kind: 'ready',
      profileId: 1,
      playlists: [
        makePlaylist(11, { name: 'Morning' }),
        makePlaylist(12, { name: 'Focus', track_count: 5, total_duration_ms: 90_000 }),
      ],
    }
    render(<PlaylistsView />)
    const list = screen.getByRole('list')
    const cards = within(list).getAllByRole('listitem')
    expect(cards).toHaveLength(2)
    expect(screen.getByText('Morning')).toBeTruthy()
    expect(screen.getByText('Focus')).toBeTruthy()
    expect(screen.getByText(/5 tracks/i)).toBeTruthy()
    // 90_000 ms = 1:30
    expect(screen.getByText(/1:30/)).toBeTruthy()
  })

  it('tags smart playlists with a Smart badge', () => {
    loaderData = {
      kind: 'ready',
      profileId: 1,
      playlists: [makePlaylist(33, { name: 'Daily Mix 1', is_smart: 1 })],
    }
    render(<PlaylistsView />)
    expect(screen.getByText(/smart/i)).toBeTruthy()
  })

  it('renders the loader error path', () => {
    loaderData = {
      kind: 'error',
      message: 'Not found.',
    }
    render(<PlaylistsView />)
    const alert = screen.getByRole('alert')
    expect(alert.textContent).toMatch(/not found/i)
  })
})
