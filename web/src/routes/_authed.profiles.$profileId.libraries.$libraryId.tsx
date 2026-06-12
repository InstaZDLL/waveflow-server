import { useMemo, useState } from 'react'
import { Link, createFileRoute } from '@tanstack/react-router'
import { PlayableTrackList } from '@/components/PlayableTrackList'
import {
  TrackFilterBar,
  applyFilters,
  initialTrackFilters,
  type TrackFilters,
} from '@/components/TrackFilterBar'
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
  // Raw route params — TanStack keeps the SAME component instance
  // when only params change (e.g. user navigates from
  // /libraries/2 → /libraries/3), so the `filters` state below
  // would otherwise persist the previous library's query into
  // the new view and surface a misleading "No tracks match…"
  // banner. We use `Route.useParams()` (reference-stable per
  // resolved value) as the reset signal.
  const { profileId, libraryId } = Route.useParams()
  const tenantKey = `${profileId}/${libraryId}`
  // Filter state lives in the route so a re-render from the player
  // (which propagates through `usePlayer` inside `PlayableTrackList`)
  // doesn't reset the user's query. Initial value is the shared
  // sensible-defaults object so the first paint matches the server
  // ordering exactly (recent first, no search, no codec filter).
  const [filters, setFilters] = useState<TrackFilters>(initialTrackFilters)
  // Adjust-state-on-prop-change (the documented codebase pattern,
  // see CLAUDE.md). Resetting `filters` from inside a `useEffect`
  // would schedule an extra render with the stale filter still
  // applied, briefly flashing "No tracks match…" on the new
  // library; doing the reset during render fixes the state before
  // the first paint AND sidesteps the
  // react-hooks/set-state-in-effect lint.
  const [lastTenantKey, setLastTenantKey] = useState(tenantKey)
  if (lastTenantKey !== tenantKey) {
    setLastTenantKey(tenantKey)
    setFilters(initialTrackFilters)
  }
  // Depend on `data` directly because TanStack's `useLoaderData`
  // returns a reference-stable snapshot per loader resolution — the
  // player tick lives in a separate context and doesn't churn it.
  // Reading `data.kind` / `data.tracks` inside the callback (rather
  // than a hoisted `const`) keeps the react-hooks/exhaustive-deps
  // rule happy. The error branch returns `[]` once and reuses it
  // since `data` is stable, so the empty-state case has no extra
  // allocation cost.
  const filteredTracks = useMemo(
    () => (data.kind === 'ready' ? applyFilters(data.tracks, filters) : []),
    [data, filters],
  )

  return (
    <main className="page-wrap app-main px-4">
      <section className="panel panel-pad">
        <div className="section-header">
          <div>
            <p className="section-eyebrow mb-2">Library</p>
            <h1 className="display-title text-4xl font-bold text-[var(--sea-ink)]">Tracks</h1>
          </div>
          {data.kind === 'ready' && (
            <nav aria-label="Library browse" className="flex flex-wrap gap-2">
            <Link
              to="/profiles/$profileId/libraries/$libraryId/albums"
              params={{
                profileId: String(data.profileId),
                libraryId: String(data.libraryId),
              }}
              className="button button-ghost"
            >
              Albums
            </Link>
            <Link
              to="/profiles/$profileId/libraries/$libraryId/artists"
              params={{
                profileId: String(data.profileId),
                libraryId: String(data.libraryId),
              }}
              className="button button-ghost"
            >
              Artists
            </Link>
          </nav>
          )}
        </div>

        {data.kind === 'error' && (
          <p role="alert" className="error-card text-sm">
            {data.message}
          </p>
        )}

        {data.kind === 'ready' && data.tracks.length > 0 && (
          <TrackFilterBar tracks={data.tracks} filters={filters} onFiltersChange={setFilters} />
        )}

        {data.kind === 'ready' && (
          <PlayableTrackList
            profileId={data.profileId}
            libraryId={data.libraryId}
            tracks={filteredTracks}
            // Distinguish "library has no tracks" from "filter
            // hides every row" — the latter is the user's own
            // doing and the empty copy below points at what they
            // can change.
            emptyMessage={
              data.tracks.length === 0
                ? 'No tracks in this library yet.'
                : 'No tracks match the current filters.'
            }
            label="Library tracks"
          />
        )}

        {data.kind === 'ready' && (
          <p className="mt-6">
            <Link
              to="/profiles/$profileId"
              params={{ profileId: String(data.profileId) }}
              className="back-link"
            >
              ← Back to libraries
            </Link>
          </p>
        )}
      </section>
    </main>
  )
}
