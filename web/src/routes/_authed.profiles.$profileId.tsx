import { Link, createFileRoute } from '@tanstack/react-router'
import { listLibraries, type Library } from '@/server-fns/libraries'

// Auth gating inherited from the `_authed` parent layout.
export const Route = createFileRoute('/_authed/profiles/$profileId')({
  loader: async ({ params }): Promise<LoaderData> => {
    const profileId = Number(params.profileId)
    if (!Number.isInteger(profileId) || profileId <= 0) {
      return { kind: 'error', message: 'Invalid profile id.' }
    }
    try {
      const libraries = await listLibraries({ data: profileId })
      return { kind: 'ready', profileId, libraries }
    } catch (err) {
      return {
        kind: 'error',
        message: err instanceof Error ? err.message : 'Failed to load libraries.',
      }
    }
  },
  component: LibrariesView,
})

type LoaderData =
  | { kind: 'ready'; profileId: number; libraries: Library[] }
  | { kind: 'error'; message: string }

function LibrariesView() {
  const data = Route.useLoaderData()

  return (
    <main className="page-wrap app-main px-4">
      <section className="panel panel-pad">
        <div className="section-header">
          <div>
            <p className="section-eyebrow mb-2">Libraries</p>
            <h1 className="display-title text-4xl font-bold text-(--sea-ink)">Choose a library</h1>
          </div>
          {data.kind === 'ready' && (
            <Link
              to="/profiles/$profileId/playlists"
              params={{ profileId: String(data.profileId) }}
              className="button button-ghost"
            >
              Playlists
            </Link>
          )}
        </div>

        {data.kind === 'error' && (
          <p role="alert" className="error-card text-sm">
            {data.message}
          </p>
        )}

        {data.kind === 'ready' && data.libraries.length === 0 && (
          <div className="status-card">
            No libraries under this profile yet. The desktop app or the API will create the first
            one for you.
          </div>
        )}

        {data.kind === 'ready' && data.libraries.length > 0 && (
          <ul className="grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
            {data.libraries.map((lib) => (
              <li key={lib.id}>
                <Link
                  to="/profiles/$profileId/libraries/$libraryId"
                  params={{
                    profileId: String(data.profileId),
                    libraryId: String(lib.id),
                  }}
                  className="card-link"
                >
                  <div className="mb-4 art-tile h-12 w-12 text-lg font-bold">
                    {Array.from(lib.name.trim())[0]?.toUpperCase() ?? 'L'}
                  </div>
                  <p className="text-base font-bold text-(--sea-ink)">{lib.name}</p>
                  <p className="mt-1 text-xs font-semibold text-(--sea-ink-soft)">
                    {lib.track_count} {lib.track_count === 1 ? 'track' : 'tracks'}
                  </p>
                </Link>
              </li>
            ))}
          </ul>
        )}

        <p className="mt-3">
          <Link to="/profiles" className="back-link">
            ← Back to profiles
          </Link>
        </p>
      </section>
    </main>
  )
}
