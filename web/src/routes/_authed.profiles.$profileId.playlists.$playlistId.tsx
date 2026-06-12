import { useState } from 'react'
import { Link, createFileRoute, useNavigate, useRouter } from '@tanstack/react-router'

import { DeletePlaylistDialog } from '@/components/DeletePlaylistDialog'
import { PlaylistFormDialog } from '@/components/PlaylistFormDialog'
import { formatTime } from '@/lib/format-time'
import {
  deletePlaylist,
  getPlaylist,
  getPlaylistTracks,
  updatePlaylist,
  type Playlist,
  type PlaylistTrack,
} from '@/server-fns/playlists'

// Auth gating inherited from the `_authed` parent layout.
export const Route = createFileRoute('/_authed/profiles/$profileId/playlists/$playlistId')({
  loader: async ({ params }): Promise<LoaderData> => {
    const profileId = Number(params.profileId)
    const playlistId = Number(params.playlistId)
    if (![profileId, playlistId].every((id) => Number.isInteger(id) && id > 0)) {
      return { kind: 'error', message: 'Invalid profile or playlist id.' }
    }
    try {
      // Fetch the metadata + tracks in parallel — both target the
      // same tenant chain so the server validates the ownership
      // twice (cheap) but the wall-clock is the slower of the two.
      // The track read is guarded against its own failure: if the
      // metadata resolved but the track list errored, we still
      // render the playlist (the user can edit / delete / navigate
      // out) and surface a non-blocking alert on the empty rail.
      const [playlist, tracksResult] = await Promise.all([
        getPlaylist({ data: { profileId, playlistId } }),
        getPlaylistTracks({ data: { profileId, playlistId } })
          .then((tracks): TrackFetchResult => ({ ok: true, tracks }))
          .catch(
            (err: unknown): TrackFetchResult => ({
              ok: false,
              error: err instanceof Error ? err.message : 'Failed to load tracks.',
            }),
          ),
      ])
      return { kind: 'ready', profileId, playlist, tracksResult }
    } catch (err) {
      return {
        kind: 'error',
        message: err instanceof Error ? err.message : 'Failed to load playlist.',
      }
    }
  },
  component: PlaylistDetailView,
})

export type TrackFetchResult = { ok: true; tracks: PlaylistTrack[] } | { ok: false; error: string }

export type LoaderData =
  | {
      kind: 'ready'
      profileId: number
      playlist: Playlist
      tracksResult: TrackFetchResult
    }
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
    <main className="page-wrap app-main px-4">
      <section className="panel panel-pad">
        {data.kind === 'error' && (
          <p role="alert" className="error-card text-sm">
            {data.message}
          </p>
        )}

        {data.kind === 'ready' && (
          <>
            <div className="section-header">
              <PlaylistHeader playlist={data.playlist} />
              {canMutate && (
                <div className="flex flex-shrink-0 items-center gap-2">
                  <button
                    type="button"
                    onClick={() => setEditOpen(true)}
                    className="button button-ghost min-h-0 px-3 py-2"
                  >
                    Edit
                  </button>
                  <button
                    type="button"
                    onClick={() => setDeleteOpen(true)}
                    className="button button-danger min-h-0 px-3 py-2"
                  >
                    Delete
                  </button>
                </div>
              )}
            </div>
            <TrackList result={data.tracksResult} />
            <p className="mt-6">
              <Link
                to="/profiles/$profileId/playlists"
                params={{ profileId: String(data.profileId) }}
                className="back-link"
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
        <p className="section-eyebrow m-0">{isSmart ? 'Smart playlist' : 'Playlist'}</p>
        {isSmart && (
          <span
            className="rounded-md px-2 py-0.5 text-[10px] font-semibold uppercase tracking-wider"
            style={{ backgroundColor: 'var(--accent-100)', color: 'var(--accent-700)' }}
          >
            Smart
          </span>
        )}
      </div>
      <h1 className="display-title text-4xl font-bold text-[var(--sea-ink)]">{playlist.name}</h1>
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

function TrackList({ result }: { result: TrackFetchResult }) {
  if (!result.ok) {
    return (
      <div role="alert" className="error-card mt-8 text-sm">
        Could not load tracks: {result.error}
      </div>
    )
  }
  if (result.tracks.length === 0) {
    return (
      <div className="status-card mt-8 text-sm">
        <p className="font-semibold text-[var(--sea-ink)]">No tracks yet</p>
        <p className="mt-2">
          Add tracks to this playlist from the desktop app — they&apos;ll sync over and show up
          here.
        </p>
      </div>
    )
  }
  // Ordinals are 1..N over the rendered array order — mainstream
  // player convention (Spotify / Apple Music / YouTube Music all
  // do this). We deliberately do NOT render `track.position` to
  // the user: it's the desktop's storage column and goes sparse
  // after deletes, so showing it would surface internal gaps
  // (e.g. "1, 2, 4, 7") that mean nothing on this surface. The
  // server still ORDER BY `position`, so the array order IS the
  // user's intended sequence.
  return (
    <ul aria-label="Playlist tracks" className="media-list mt-8">
      {result.tracks.map((track, index) => (
        <TrackRow key={track.track_id} track={track} ordinal={index + 1} />
      ))}
    </ul>
  )
}

function TrackRow({ track, ordinal }: { track: PlaylistTrack; ordinal: number }) {
  // Pre-1.j.b desktops emitted tracks ops without snapshots — the
  // owner is allowed to see those rows on this surface, so render
  // a placeholder rather than hiding them. We display the rendered
  // ordinal rather than the wire `track_id` so the placeholder
  // doesn't leak the desktop's local i64 row id (which means
  // nothing on another device's view and changes per-desktop).
  const title = track.snapshot_title ?? `Track ${ordinal}`
  const hasMetadata = track.snapshot_title !== null
  const durationSec =
    track.snapshot_duration_ms !== null && track.snapshot_duration_ms > 0
      ? track.snapshot_duration_ms / 1000
      : null
  return (
    <li className="media-row">
      <span
        aria-hidden="true"
        className="w-6 flex-shrink-0 text-right text-xs tabular-nums text-[var(--sea-ink-soft)]"
      >
        {ordinal}
      </span>
      <div className="min-w-0 flex-1">
        <p
          className={`truncate text-sm font-semibold ${
            hasMetadata ? 'text-[var(--sea-ink)]' : 'text-[var(--sea-ink-soft)] italic'
          }`}
        >
          {title}
        </p>
        {track.snapshot_artist && (
          <p className="truncate text-xs text-[var(--sea-ink-soft)]">{track.snapshot_artist}</p>
        )}
      </div>
      {durationSec !== null && (
        <span className="flex-shrink-0 text-xs tabular-nums text-[var(--sea-ink-soft)]">
          {formatTime(durationSec)}
        </span>
      )}
    </li>
  )
}
