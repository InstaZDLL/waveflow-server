import { Link, createFileRoute } from '@tanstack/react-router'

// Sign-in paused for the 1.5.0 cut alongside sign-up. See sign-up.tsx
// for the full rationale. The desktop OAuth-loopback flow that hits
// this route from the system browser is also dormant for 1.5.0 —
// every sync surface on the desktop is hidden until 1.6.0.

interface SignInSearch {
  /**
   * Kept on the route so the desktop loopback handshake doesn't 404
   * on a `?continue=...` query string left from a previous release.
   * The full validate-redirect gate (`safeContinueTarget`) is still
   * exported below — the test suite locks it down so the open-redirect
   * invariants don't drift while sign-in is dormant.
   */
  continue?: string
}

/**
 * Open-redirect gate for the post-sign-in navigate. The naive
 * `startsWith('/desktop-login')` guard let `/desktop-login/../admin`
 * slip past because the browser normalises that to `/admin` after
 * navigation. Exported (and exercised by `-sign-in.test.ts`) even
 * while the sign-in form is hidden, so the hardened behaviour stays
 * pinned for the 1.6.0 restoration.
 */
export function safeContinueTarget(raw: string | undefined): string {
  if (!raw) return '/'
  let parsed: URL
  try {
    parsed = new URL(raw, 'http://localhost')
  } catch {
    return '/'
  }
  if (parsed.origin !== 'http://localhost') return '/'
  if (parsed.pathname !== '/desktop-login' && !parsed.pathname.startsWith('/desktop-login/'))
    return '/'
  return parsed.pathname + parsed.search
}

export const Route = createFileRoute('/sign-in')({
  validateSearch: (raw: Record<string, unknown>): SignInSearch =>
    typeof raw.continue === 'string' ? { continue: raw.continue } : {},
  component: SignIn,
})

export function SignIn() {
  return (
    <main className="page-wrap app-main px-4">
      <section className="panel panel-pad mx-auto max-w-md text-center">
        <p className="section-eyebrow mb-2">Sign-in paused</p>
        <h1 className="display-title mb-4 text-3xl font-bold text-(--sea-ink)">
          WaveFlow accounts are temporarily disabled
        </h1>
        <p className="mb-6 text-sm text-(--sea-ink-soft)">
          Multi-device sync is being polished for 1.6.0. The desktop
          1.5.0 release ships in local-only mode — there is nothing to
          sign in to right now. Sign-in re-opens alongside the sync
          feature.
        </p>
        <Link to="/" className="button button-primary inline-block">
          Back to home
        </Link>
      </section>
    </main>
  )
}
