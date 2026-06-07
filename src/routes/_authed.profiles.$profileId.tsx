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
    <main className="page-wrap px-4 py-12">
      <section className="island-shell rounded-2xl p-6 sm:p-8">
        <p className="island-kicker mb-2">Libraries</p>
        <h1 className="display-title mb-4 text-3xl font-bold text-[var(--sea-ink)] sm:text-4xl">
          Choose a library
        </h1>

        {data.kind === 'error' && (
          <p role="alert" className="text-base text-red-600 dark:text-red-400">
            {data.message}
          </p>
        )}

        {data.kind === 'ready' && data.libraries.length === 0 && (
          <p className="text-base text-[var(--sea-ink-soft)]">
            No libraries under this profile yet. The desktop app or the API will create the first
            one for you.
          </p>
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
                  className="block rounded-xl border border-[var(--line)] bg-[var(--chip-bg)] p-4 no-underline transition hover:opacity-90"
                >
                  <p className="text-base font-semibold text-[var(--sea-ink)]">{lib.name}</p>
                  <p className="mt-1 text-xs text-[var(--sea-ink-soft)]">
                    {lib.track_count} {lib.track_count === 1 ? 'track' : 'tracks'}
                  </p>
                </Link>
              </li>
            ))}
          </ul>
        )}

        {data.kind === 'ready' && (
          <p className="mt-6">
            <Link
              to="/profiles/$profileId/playlists"
              params={{ profileId: String(data.profileId) }}
              className="text-sm font-semibold text-[var(--sea-ink)] underline"
            >
              View playlists →
            </Link>
          </p>
        )}

        <p className="mt-3">
          <Link to="/profiles" className="text-sm text-[var(--sea-ink-soft)] underline">
            ← Back to profiles
          </Link>
        </p>
      </section>
    </main>
  )
}
