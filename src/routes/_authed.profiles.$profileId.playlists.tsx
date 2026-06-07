import { useState } from 'react'
import { Link, createFileRoute, useNavigate, useRouter } from '@tanstack/react-router'

import { CreatePlaylistDialog } from '@/components/CreatePlaylistDialog'
import { formatTime } from '@/lib/format-time'
import { listPlaylists, type Playlist } from '@/server-fns/playlists'

// Auth gating inherited from the `_authed` parent layout.
export const Route = createFileRoute('/_authed/profiles/$profileId/playlists')({
  loader: async ({ params }): Promise<LoaderData> => {
    const profileId = Number(params.profileId)
    if (!Number.isInteger(profileId) || profileId <= 0) {
      return { kind: 'error', message: 'Invalid profile id.' }
    }
    try {
      const playlists = await listPlaylists({ data: profileId })
      return { kind: 'ready', profileId, playlists }
    } catch (err) {
      return {
        kind: 'error',
        message: err instanceof Error ? err.message : 'Failed to load playlists.',
      }
    }
  },
  component: PlaylistsView,
})

type LoaderData =
  | { kind: 'ready'; profileId: number; playlists: Playlist[] }
  | { kind: 'error'; message: string }

// Exported for direct unit-render — sidesteps the file-route + router
// shell so a vitest spec can mount the component without spinning up
// the full router.
export function PlaylistsView() {
  const data = Route.useLoaderData()
  const router = useRouter()
  const navigate = useNavigate()
  const [createOpen, setCreateOpen] = useState(false)
  const canCreate = data.kind === 'ready'

  async function onCreated(playlist: Playlist) {
    setCreateOpen(false)
    if (data.kind !== 'ready') return
    const profileId = data.profileId
    // Navigate first so the user lands on the detail page
    // immediately — the create call already succeeded, the new
    // playlist exists, the listing's freshness can wait. Firing
    // invalidate() afterwards keeps the grid up to date in the
    // background; a rejection here is logged but doesn't bubble
    // into a failure path the user has to acknowledge for an
    // operation that's already done.
    await navigate({
      to: '/profiles/$profileId/playlists/$playlistId',
      params: { profileId: String(profileId), playlistId: String(playlist.id) },
    })
    router.invalidate().catch((err) => {
      console.warn('[playlists] post-create invalidate failed:', err)
    })
  }

  return (
    <main className="page-wrap px-4 py-12">
      <section className="island-shell rounded-2xl p-6 sm:p-8">
        <div className="mb-4 flex flex-wrap items-center justify-between gap-3">
          <div>
            <p className="island-kicker mb-2">Playlists</p>
            <h1 className="display-title text-3xl font-bold text-[var(--sea-ink)] sm:text-4xl">
              Your playlists
            </h1>
          </div>
          {canCreate && (
            <button
              type="button"
              onClick={() => setCreateOpen(true)}
              className="rounded-full px-4 py-2 text-sm font-semibold text-white transition hover:-translate-y-0.5 hover:opacity-95"
              style={{ backgroundColor: 'var(--accent-600)' }}
            >
              + Create playlist
            </button>
          )}
        </div>

        {data.kind === 'error' && (
          <p role="alert" className="text-base text-red-600 dark:text-red-400">
            {data.message}
          </p>
        )}

        {data.kind === 'ready' && data.playlists.length === 0 && (
          <p className="text-base text-[var(--sea-ink-soft)]">
            No playlists yet. Create one from the desktop app — it&apos;ll sync here next time you
            open the page.
          </p>
        )}

        {data.kind === 'ready' && data.playlists.length > 0 && (
          <ul className="grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
            {data.playlists.map((playlist) => (
              <PlaylistCard key={playlist.id} profileId={data.profileId} playlist={playlist} />
            ))}
          </ul>
        )}

        {data.kind === 'ready' && (
          <p className="mt-6">
            <Link
              to="/profiles/$profileId"
              params={{ profileId: String(data.profileId) }}
              className="text-sm text-[var(--sea-ink-soft)] underline"
            >
              ← Back to libraries
            </Link>
          </p>
        )}

        {data.kind === 'error' && (
          <p className="mt-6">
            <Link to="/profiles" className="text-sm text-[var(--sea-ink-soft)] underline">
              ← Back to profiles
            </Link>
          </p>
        )}
      </section>

      {canCreate && (
        <CreatePlaylistDialog
          open={createOpen}
          profileId={data.profileId}
          onClose={() => setCreateOpen(false)}
          onCreated={onCreated}
        />
      )}
    </main>
  )
}

interface PlaylistCardProps {
  profileId: number
  playlist: Playlist
}

function PlaylistCard({ profileId, playlist }: PlaylistCardProps) {
  const isSmart = playlist.is_smart === 1
  return (
    <li>
      <Link
        to="/profiles/$profileId/playlists/$playlistId"
        params={{
          profileId: String(profileId),
          playlistId: String(playlist.id),
        }}
        className="block rounded-xl border border-[var(--line)] bg-[var(--chip-bg)] p-4 no-underline transition hover:-translate-y-0.5 hover:opacity-95"
      >
        <div className="flex items-start justify-between gap-3">
          <p className="flex-1 truncate text-base font-semibold text-[var(--sea-ink)]">
            {playlist.name}
          </p>
          {isSmart && (
            <span
              className="flex-shrink-0 rounded-full px-2 py-0.5 text-[10px] font-semibold uppercase tracking-wider"
              style={{ backgroundColor: 'var(--accent-100)', color: 'var(--accent-700)' }}
            >
              Smart
            </span>
          )}
        </div>
        {playlist.description && (
          <p className="mt-1 line-clamp-2 text-xs text-[var(--sea-ink-soft)]">
            {playlist.description}
          </p>
        )}
        <p className="mt-2 text-xs text-[var(--sea-ink-soft)]">
          {playlist.track_count} {playlist.track_count === 1 ? 'track' : 'tracks'}
          {playlist.total_duration_ms > 0 && (
            <span> · {formatTime(playlist.total_duration_ms / 1000)}</span>
          )}
        </p>
      </Link>
    </li>
  )
}
