import { useState } from 'react'
import { Link, createFileRoute, useNavigate, useRouter } from '@tanstack/react-router'

import { DeletePlaylistDialog } from '@/components/DeletePlaylistDialog'
import { PlaylistFormDialog } from '@/components/PlaylistFormDialog'
import { formatTime } from '@/lib/format-time'
import { deletePlaylist, getPlaylist, updatePlaylist, type Playlist } from '@/server-fns/playlists'

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
  const router = useRouter()
  const navigate = useNavigate()
  const [editOpen, setEditOpen] = useState(false)
  const [deleteOpen, setDeleteOpen] = useState(false)

  const isReady = data.kind === 'ready'
  // Smart playlists are read-only on the web in v1 — the server's
  // repo refuses to PATCH them anyway, and the editor for smart
  // rules lives on the desktop. Hide the action buttons for them
  // so the user doesn't get a "not found" surprise after a click.
  const canMutate = isReady && data.playlist.is_smart === 0

  function onEdited(_updated: Playlist) {
    setEditOpen(false)
    router.invalidate().catch((err) => {
      console.warn('[playlists] post-edit invalidate failed:', err)
    })
  }

  async function onDeleted() {
    setDeleteOpen(false)
    if (!isReady) return
    const profileId = data.profileId
    router.invalidate().catch((err) => {
      console.warn('[playlists] post-delete invalidate failed:', err)
    })
    try {
      await navigate({
        to: '/profiles/$profileId/playlists',
        params: { profileId: String(profileId) },
      })
    } catch (err) {
      console.warn('[playlists] post-delete navigation failed:', err)
    }
  }

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
            <div className="flex flex-wrap items-start justify-between gap-3">
              <PlaylistHeader playlist={data.playlist} />
              {canMutate && (
                <div className="flex flex-shrink-0 items-center gap-2">
                  <button
                    type="button"
                    onClick={() => setEditOpen(true)}
                    className="rounded-xl border border-[var(--line)] bg-[var(--chip-bg)] px-3 py-1.5 text-sm font-semibold text-[var(--sea-ink)] transition hover:opacity-90"
                  >
                    Edit
                  </button>
                  <button
                    type="button"
                    onClick={() => setDeleteOpen(true)}
                    className="rounded-xl border border-red-200 bg-red-50 px-3 py-1.5 text-sm font-semibold text-red-700 transition hover:bg-red-100 dark:border-red-900 dark:bg-red-950/40 dark:text-red-300 dark:hover:bg-red-900/40"
                  >
                    Delete
                  </button>
                </div>
              )}
            </div>
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

      {canMutate && (
        <>
          <PlaylistFormDialog
            open={editOpen}
            mode="edit"
            initial={{
              name: data.playlist.name,
              description: data.playlist.description ?? '',
            }}
            onClose={() => setEditOpen(false)}
            submit={(values) =>
              updatePlaylist({
                data: {
                  profileId: data.profileId,
                  playlistId: data.playlist.id,
                  name: values.name,
                  // PlaylistFormDialog passes through the cleared
                  // description as an empty string in edit mode so the
                  // server overwrites the existing value.
                  ...(values.description !== undefined ? { description: values.description } : {}),
                },
              })
            }
            onSubmitted={onEdited}
          />
          <DeletePlaylistDialog
            open={deleteOpen}
            playlistName={data.playlist.name}
            onClose={() => setDeleteOpen(false)}
            submit={() =>
              deletePlaylist({
                data: { profileId: data.profileId, playlistId: data.playlist.id },
              })
            }
            onDeleted={onDeleted}
          />
        </>
      )}
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
            Smart
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
