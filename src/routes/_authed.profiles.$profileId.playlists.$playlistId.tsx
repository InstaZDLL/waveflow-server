import { Link, createFileRoute } from '@tanstack/react-router'

import { formatTime } from '@/lib/format-time'
import { getPlaylist, type Playlist } from '@/server-fns/playlists'

// Auth gating inherited from the `_authed` parent layout.
export const Route = createFileRoute('/_authed/profiles/$profileId/playlists/$playlistId')({
  loader: async ({ params }): Promise<LoaderData> => {
    const profileId = Number(params.profileId)
    const playlistId = Number(params.playlistId)
    if (![profileId, playlistId].every((id) => Number.isInteger(id) && id > 0)) {
      return { kind: 'error', message: 'Invalid profile or playlist id.' }
    }
    try {
      const playlist = await getPlaylist({ data: { profileId, playlistId } })
      return { kind: 'ready', profileId, playlist }
    } catch (err) {
      return {
        kind: 'error',
        message: err instanceof Error ? err.message : 'Failed to load playlist.',
      }
    }
  },
  component: PlaylistDetailView,
})

type LoaderData =
  | { kind: 'ready'; profileId: number; playlist: Playlist }
  | { kind: 'error'; message: string }

// Exported for direct unit-render — sidesteps the file-route + router
// shell so a vitest spec can mount the component without spinning up
// the full router.
export function PlaylistDetailView() {
  const data = Route.useLoaderData()

  return (
    <main className="page-wrap px-4 py-12 pb-32">
      <section className="island-shell rounded-2xl p-6 sm:p-8">
        {data.kind === 'error' && (
          <p role="alert" className="text-base text-red-600 dark:text-red-400">
            {data.message}
          </p>
        )}

        {data.kind === 'ready' && (
          <>
            <PlaylistHeader playlist={data.playlist} />
            <div className="mt-8 rounded-xl border border-dashed border-[var(--line)] bg-[var(--chip-bg)] p-6 text-sm text-[var(--sea-ink-soft)]">
              <p className="font-semibold text-[var(--sea-ink)]">Tracks coming soon</p>
              <p className="mt-2">
                The web client can read playlist metadata today, but the per-playlist track listing
                endpoint hasn&apos;t shipped on{' '}
                <a
                  href="https://github.com/InstaZDLL/waveflow-server"
                  target="_blank"
                  rel="noreferrer"
                  className="font-semibold text-[var(--sea-ink)] underline"
                >
                  waveflow-server
                </a>{' '}
                yet. Until it does, edit playlists from the desktop app.
              </p>
            </div>
            <p className="mt-6">
              <Link
                to="/profiles/$profileId/playlists"
                params={{ profileId: String(data.profileId) }}
                className="text-sm text-[var(--sea-ink-soft)] underline"
              >
                ← Back to playlists
              </Link>
            </p>
          </>
        )}
      </section>
    </main>
  )
}

function PlaylistHeader({ playlist }: { playlist: Playlist }) {
  const isSmart = playlist.is_smart === 1
  return (
    <div className="flex flex-col gap-3">
      <div className="flex items-center gap-2">
        <p className="island-kicker m-0">{isSmart ? 'Smart playlist' : 'Playlist'}</p>
        {isSmart && (
          <span
            className="rounded-full px-2 py-0.5 text-[10px] font-semibold uppercase tracking-wider"
            style={{ backgroundColor: 'var(--accent-100)', color: 'var(--accent-700)' }}
          >
            Auto
          </span>
        )}
      </div>
      <h1 className="display-title text-3xl font-bold text-[var(--sea-ink)] sm:text-4xl">
        {playlist.name}
      </h1>
      {playlist.description && (
        <p className="text-sm text-[var(--sea-ink-soft)]">{playlist.description}</p>
      )}
      <p className="text-xs text-[var(--sea-ink-soft)]">
        {playlist.track_count} {playlist.track_count === 1 ? 'track' : 'tracks'}
        {playlist.total_duration_ms > 0 && (
          <span> · {formatTime(playlist.total_duration_ms / 1000)}</span>
        )}
      </p>
    </div>
  )
}
