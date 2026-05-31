import { Link, createFileRoute, redirect } from '@tanstack/react-router'
import { listProfiles, type Profile } from '@/server-fns/profiles'
import { getCurrentSession } from '@/server-fns/session'

// `beforeLoad` runs server-side on SSR and client-side on
// navigation — both paths reach the same auth gate. Throwing
// `redirect()` aborts the load and pushes the user at /sign-in
// before the component mounts, so there's no flash of empty
// content for an unauthenticated visitor.
//
// `loader` then fetches the profile list. Any error becomes a
// loader rejection that the component sees via `useLoaderData` —
// we wrap to keep the message client-safe (the server fn already
// maps statuses, this is the catch-all envelope).
export const Route = createFileRoute('/profiles')({
  beforeLoad: async () => {
    const session = await getCurrentSession()
    if (!session) {
      throw redirect({ to: '/sign-in' })
    }
  },
  loader: async (): Promise<LoaderData> => {
    try {
      const profiles = await listProfiles()
      return { kind: 'ready', profiles }
    } catch (err) {
      return {
        kind: 'error',
        message: err instanceof Error ? err.message : 'Failed to load profiles.',
      }
    }
  },
  component: ProfilesView,
})

type LoaderData = { kind: 'ready'; profiles: Profile[] } | { kind: 'error'; message: string }

function ProfilesView() {
  const data = Route.useLoaderData()

  return (
    <main className="page-wrap px-4 py-12">
      <section className="island-shell rounded-2xl p-6 sm:p-8">
        <p className="island-kicker mb-2">Profiles</p>
        <h1 className="display-title mb-4 text-3xl font-bold text-[var(--sea-ink)] sm:text-4xl">
          Your profiles
        </h1>

        {data.kind === 'error' && (
          <p role="alert" className="text-base text-red-600 dark:text-red-400">
            {data.message}
          </p>
        )}

        {data.kind === 'ready' && data.profiles.length === 0 && (
          <p className="text-base text-[var(--sea-ink-soft)]">
            No profiles yet. The desktop app or the API will create the first one for you.
          </p>
        )}

        {data.kind === 'ready' && data.profiles.length > 0 && (
          <ul className="grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
            {data.profiles.map((p) => (
              <li key={p.id}>
                <Link
                  to="/profiles/$profileId"
                  params={{ profileId: String(p.id) }}
                  className="block rounded-xl border border-[var(--line)] bg-[var(--chip-bg)] p-4 no-underline transition hover:opacity-90"
                >
                  <p className="text-base font-semibold text-[var(--sea-ink)]">{p.name}</p>
                  <p className="mt-1 text-xs text-[var(--sea-ink-soft)]">
                    Last used {new Date(p.last_used_at).toLocaleDateString()}
                  </p>
                </Link>
              </li>
            ))}
          </ul>
        )}
      </section>
    </main>
  )
}
