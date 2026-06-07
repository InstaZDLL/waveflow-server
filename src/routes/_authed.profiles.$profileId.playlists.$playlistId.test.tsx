// PlaylistDetailView render tests. Mocks the file-route so we
// can mount the component without spinning up the router.

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

// The route file imports `getPlaylist` at the module top, which
// transitively pulls in the server-fn internals (DB driver, JWT
// mint) — none of which is appropriate for a unit-test render.
vi.mock('@/server-fns/playlists', () => ({
  getPlaylist: vi.fn(),
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
    expect(screen.getByText(/smart playlist/i)).toBeTruthy()
    expect(screen.getByText(/auto/i)).toBeTruthy()
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
})
