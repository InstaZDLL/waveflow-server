import { Link, createFileRoute } from '@tanstack/react-router'

// WaveFlow account creation is paused for the 1.5.0 cut. The desktop
// app does not surface server account binding in 1.5.0 — sync, public
// share, and multi-device features re-enable together in 1.6.0 once
// the upgrade-bootstrap (pre-v2 row heal, server-side library
// auto-provision) lands. Until then this route returns a stub so the
// URL stays valid (auto-updaters / bookmarks don't 404) but does not
// invoke Better Auth's signUp.email path.

export const Route = createFileRoute('/sign-up')({
  component: SignUp,
})

export function SignUp() {
  return (
    <main className="page-wrap app-main px-4">
      <section className="panel panel-pad mx-auto max-w-md text-center">
        <p className="section-eyebrow mb-2">Account creation paused</p>
        <h1 className="display-title mb-4 text-3xl font-bold text-(--sea-ink)">
          Sign-ups are temporarily disabled
        </h1>
        <p className="mb-6 text-sm text-(--sea-ink-soft)">
          WaveFlow multi-device sync is being polished for 1.6.0. The
          desktop 1.5.0 release ships in local-only mode — there is
          nothing to bind to right now. We&rsquo;ll re-open sign-ups
          alongside the sync feature.
        </p>
        <Link to="/" className="button button-primary inline-block">
          Back to home
        </Link>
      </section>
    </main>
  )
}
