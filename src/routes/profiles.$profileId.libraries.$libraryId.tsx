import { useState } from 'react'
import { Link, createFileRoute, redirect } from '@tanstack/react-router'
import { Player, type PlayingTrack } from '@/components/Player'
import { getStreamUrl } from '@/server-fns/stream'
import { listTracks, type Track } from '@/server-fns/tracks'
import { getCurrentSession } from '@/server-fns/session'

export const Route = createFileRoute('/profiles/$profileId/libraries/$libraryId')({
  beforeLoad: async () => {
    const session = await getCurrentSession()
    if (!session) {
      throw redirect({ to: '/sign-in' })
    }
  },
  loader: async ({ params }): Promise<LoaderData> => {
    const profileId = Number(params.profileId)
    const libraryId = Number(params.libraryId)
    if (![profileId, libraryId].every((id) => Number.isInteger(id) && id > 0)) {
      return { kind: 'error', message: 'Invalid profile or library id.' }
    }
    try {
      const tracks = await listTracks({ data: { profileId, libraryId } })
      return { kind: 'ready', profileId, libraryId, tracks }
    } catch (err) {
      return {
        kind: 'error',
        message: err instanceof Error ? err.message : 'Failed to load tracks.',
      }
    }
  },
  component: TracksView,
})

type LoaderData =
  | { kind: 'ready'; profileId: number; libraryId: number; tracks: Track[] }
  | { kind: 'error'; message: string }

function TracksView() {
  const data = Route.useLoaderData()
  const [current, setCurrent] = useState<PlayingTrack | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [pendingTrackId, setPendingTrackId] = useState<number | null>(null)

  async function play(track: Track) {
    if (data.kind !== 'ready') return
    setError(null)
    setPendingTrackId(track.id)
    try {
      const { url } = await getStreamUrl({
        data: {
          profileId: data.profileId,
          libraryId: data.libraryId,
          trackId: track.id,
        },
      })
      setCurrent({
        id: track.id,
        title: track.title,
        url,
        durationMs: track.duration_ms,
      })
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Could not start playback.')
    } finally {
      setPendingTrackId(null)
    }
  }

  return (
    <>
      <main className="page-wrap px-4 py-12 pb-32">
        <section className="island-shell rounded-2xl p-6 sm:p-8">
          <p className="island-kicker mb-2">Library</p>
          <h1 className="display-title mb-4 text-3xl font-bold text-[var(--sea-ink)] sm:text-4xl">
            Tracks
          </h1>

          {data.kind === 'error' && (
            <p role="alert" className="text-base text-red-600 dark:text-red-400">
              {data.message}
            </p>
          )}

          {error && (
            <p
              role="alert"
              className="mb-4 rounded-lg border border-red-200 bg-red-50 px-3 py-2 text-sm text-red-700 dark:border-red-900 dark:bg-red-950/40 dark:text-red-300"
            >
              {error}
            </p>
          )}

          {data.kind === 'ready' && data.tracks.length === 0 && (
            <p className="text-base text-[var(--sea-ink-soft)]">No tracks in this library yet.</p>
          )}

          {data.kind === 'ready' && data.tracks.length > 0 && (
            <ul className="divide-y divide-[var(--line)]">
              {data.tracks.map((track) => {
                const isCurrent = current?.id === track.id
                const isPending = pendingTrackId === track.id
                return (
                  <li key={track.id} className="flex items-center gap-3 py-2">
                    <button
                      type="button"
                      onClick={() => play(track)}
                      disabled={isPending}
                      aria-label={`Play ${track.title}`}
                      className="flex h-9 w-9 flex-shrink-0 items-center justify-center rounded-full border border-[var(--line)] bg-[var(--chip-bg)] transition hover:opacity-90 disabled:opacity-50"
                    >
                      {isPending ? '…' : '▶'}
                    </button>
                    <div className="min-w-0 flex-1">
                      <p
                        className={`truncate text-sm ${
                          isCurrent
                            ? 'font-semibold text-[var(--sea-ink)]'
                            : 'text-[var(--sea-ink)]'
                        }`}
                      >
                        {track.title}
                      </p>
                      {track.codec && (
                        <p className="truncate text-xs text-[var(--sea-ink-soft)]">{track.codec}</p>
                      )}
                    </div>
                    <span className="text-xs tabular-nums text-[var(--sea-ink-soft)]">
                      {formatDuration(track.duration_ms)}
                    </span>
                  </li>
                )
              })}
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
        </section>
      </main>

      {/*
        Keying the Player by the current track id (or `idle` when
        nothing's playing) remounts the component on every track
        swap. That resets its internal `playing` / `position` state
        without a `setState`-in-effect, which the React hooks lint
        rule (v7) flags.
      */}
      <Player key={current?.id ?? 'idle'} current={current} />
    </>
  )
}

function formatDuration(ms: number): string {
  const total = Math.floor(ms / 1000)
  const m = Math.floor(total / 60)
  const s = total % 60
  return `${m}:${s.toString().padStart(2, '0')}`
}
