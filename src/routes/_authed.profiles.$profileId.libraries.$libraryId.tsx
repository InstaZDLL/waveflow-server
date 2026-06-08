import { Link, createFileRoute } from '@tanstack/react-router'
import { PlayableTrackList } from '@/components/PlayableTrackList'
import { listTracks, type Track } from '@/server-fns/tracks'

// Auth gating inherited from the `_authed` parent layout.
export const Route = createFileRoute('/_authed/profiles/$profileId/libraries/$libraryId')({
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

  return (
    <main className="page-wrap px-4 py-12 pb-32">
      <section className="island-shell rounded-2xl p-6 sm:p-8">
        <p className="island-kicker mb-2">Library</p>
        <h1 className="display-title mb-4 text-3xl font-bold text-[var(--sea-ink)] sm:text-4xl">
          Tracks
        </h1>

        {data.kind === 'ready' && (
          <nav aria-label="Library browse" className="mb-6 flex flex-wrap gap-2">
            <Link
              to="/profiles/$profileId/libraries/$libraryId/albums"
              params={{
                profileId: String(data.profileId),
                libraryId: String(data.libraryId),
              }}
              className="rounded-xl border border-[var(--line)] bg-[var(--chip-bg)] px-3 py-1.5 text-sm font-semibold text-[var(--sea-ink)] no-underline transition hover:opacity-90"
            >
              Browse albums →
            </Link>
            <Link
              to="/profiles/$profileId/libraries/$libraryId/artists"
              params={{
                profileId: String(data.profileId),
                libraryId: String(data.libraryId),
              }}
              className="rounded-xl border border-[var(--line)] bg-[var(--chip-bg)] px-3 py-1.5 text-sm font-semibold text-[var(--sea-ink)] no-underline transition hover:opacity-90"
            >
              Browse artists →
            </Link>
          </nav>
        )}

        {data.kind === 'error' && (
          <p role="alert" className="text-base text-red-600 dark:text-red-400">
            {data.message}
          </p>
        )}

        {data.kind === 'ready' && (
          <PlayableTrackList
            profileId={data.profileId}
            libraryId={data.libraryId}
            tracks={data.tracks}
            emptyMessage="No tracks in this library yet."
            label="Library tracks"
          />
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
  )
}
