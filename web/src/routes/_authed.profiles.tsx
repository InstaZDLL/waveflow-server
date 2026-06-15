import { Link, createFileRoute } from '@tanstack/react-router'
import { listProfiles, type Profile } from '@/server-fns/profiles'

interface ProfileWithFormatted extends Profile {
  last_used_at_formatted: string
}

// Pre-format dates server-side with a fixed locale + timezone so
// the SSR output matches what the client would render. `new
// Date(...).toLocaleDateString()` defaults to the runtime locale
// + timezone, which diverges between Node (server) and the
// browser (client) — React then logs a hydration mismatch and
// briefly flickers the wrong format. `en-US` + `UTC` is a stable
// choice we can revisit when we wire user-locale selection.
const DATE_FORMATTER = new Intl.DateTimeFormat('en-US', {
  timeZone: 'UTC',
  year: 'numeric',
  month: 'short',
  day: 'numeric',
})

// Auth gating lives in the `_authed` parent layout — every route
// in this folder inherits the session check, no per-route
// `beforeLoad` to duplicate. The loader only handles data.
export const Route = createFileRoute('/_authed/profiles')({
  loader: async (): Promise<LoaderData> => {
    try {
      const profiles = await listProfiles()
      const formatted: ProfileWithFormatted[] = profiles.map((p) => ({
        ...p,
        last_used_at_formatted: DATE_FORMATTER.format(new Date(p.last_used_at)),
      }))
      return { kind: 'ready', profiles: formatted }
    } catch (err) {
      // Log the raw error server-side; surface a stable generic
      // message to the UI (see `artists.tsx` loader for the
      // rationale).
      console.error('[profiles.loader] listProfiles failed', err)
      return {
        kind: 'error',
        message: 'Failed to load profiles.',
      }
    }
  },
  component: ProfilesView,
})

type LoaderData =
  | { kind: 'ready'; profiles: ProfileWithFormatted[] }
  | { kind: 'error'; message: string }

function ProfilesView() {
  const data = Route.useLoaderData()

  return (
    <main className="page-wrap app-main px-4">
      <section className="panel panel-pad">
        <div className="section-header">
          <div>
            <p className="section-eyebrow mb-2">Profiles</p>
            <h1 className="display-title text-4xl font-bold text-(--sea-ink)">
              Choose a workspace
            </h1>
          </div>
        </div>

        {data.kind === 'error' && (
          <p role="alert" className="error-card text-sm">
            {data.message}
          </p>
        )}

        {data.kind === 'ready' && data.profiles.length === 0 && (
          <div className="status-card">
            No profiles yet. The desktop app or the API will create the first one for you.
          </div>
        )}

        {data.kind === 'ready' && data.profiles.length > 0 && (
          <ul className="grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
            {data.profiles.map((p) => (
              <li key={p.id}>
                <Link
                  to="/profiles/$profileId"
                  params={{ profileId: String(p.id) }}
                  className="card-link"
                >
                  <div className="mb-4 art-tile h-12 w-12 text-lg font-bold">
                    {p.name.trim()[0]?.toUpperCase() ?? 'P'}
                  </div>
                  <p className="text-base font-bold text-(--sea-ink)">{p.name}</p>
                  <p className="mt-1 text-xs text-(--sea-ink-soft)">
                    Last used {p.last_used_at_formatted}
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
