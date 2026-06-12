// `/desktop-login` — bridge route for the WaveFlow desktop OAuth-style
// loopback handshake (Phase 1.f.desktop.1b).
//
// Flow:
//
//   1. The desktop binds a `tiny_http` listener on
//      `127.0.0.1:<port>/cb`, generates a random `state`, and opens
//      the user's default browser to:
//
//        /desktop-login?cb=http://127.0.0.1:PORT/cb&state=<state>
//
//   2. `resolveDesktopLogin` (server function) validates `cb` +
//      `state`, checks for an active Better Auth session, and mints
//      a fresh JWT via `auth.api.getToken`.
//
//   3. The browser is server-side-redirected (302) to
//      `<cb>?token=<jwt>&state=<state>`. The desktop listener
//      validates `state` matches what it generated and stores the
//      JWT.
//
//   4. If the user isn't signed in, `beforeLoad` redirects them to
//      `/sign-in?continue=…` so the existing form lands them back
//      here on success.
//
// The actual server logic lives in `@/server-fns/desktop-login` —
// route files get bundled into the client too, so importing
// `getRequestHeaders` (server-only) here directly would fail the
// TanStack import-protection plugin at build time.

import { createFileRoute, redirect } from '@tanstack/react-router'
import { resolveDesktopLogin } from '@/server-fns/desktop-login'

interface DesktopLoginSearch {
  cb?: string
  state?: string
}

interface DesktopLoginContext {
  status: 'invalid-callback' | 'mint-failed'
}

export const Route = createFileRoute('/desktop-login')({
  validateSearch: (raw: Record<string, unknown>): DesktopLoginSearch => ({
    cb: typeof raw.cb === 'string' ? raw.cb : undefined,
    state: typeof raw.state === 'string' ? raw.state : undefined,
  }),
  beforeLoad: async ({ search }): Promise<DesktopLoginContext> => {
    const result = await resolveDesktopLogin({
      data: { cb: search.cb ?? '', state: search.state ?? '' },
    })

    if (result.kind === 'redirect') {
      // Hard redirect — `throw redirect({ href })` lets TanStack
      // emit a 302 response with the loopback URL as `Location`. The
      // JWT never reaches a rendered DOM and the localhost endpoint
      // captures it the same way an OAuth callback works.
      throw redirect({ href: result.url })
    }

    if (result.kind === 'no-session') {
      // Encode the *current* URL (search included) so post-sign-in
      // navigate resumes the OAuth flow without losing `cb` /
      // `state`. `cb` was already validated server-side, so we know
      // it's a non-malicious loopback URL by the time we round-trip.
      const continueTo = `/desktop-login?cb=${encodeURIComponent(search.cb ?? '')}&state=${encodeURIComponent(search.state ?? '')}`
      throw redirect({ to: '/sign-in', search: { continue: continueTo } })
    }

    return { status: result.kind }
  },
  component: DesktopLoginPage,
})

function DesktopLoginPage() {
  const ctx = Route.useRouteContext()

  if (ctx.status === 'invalid-callback') {
    return (
      <main className="page-wrap app-main px-4">
        <section className="panel panel-pad mx-auto max-w-md">
          <p className="island-kicker mb-2">WaveFlow desktop</p>
          <h1 className="display-title mb-4 text-2xl font-bold text-(--sea-ink)">
            Invalid sign-in link
          </h1>
          <p className="text-sm text-(--sea-ink-soft)">
            The desktop sent us a callback URL we can&apos;t use. The link must point at the local
            handshake listener on
            <code className="mx-1 rounded bg-black/5 px-1 py-0.5 text-xs">http://127.0.0.1</code>
            and include a non-empty <code>state</code> parameter. Re-launch the sign-in from the
            desktop&apos;s Settings page.
          </p>
        </section>
      </main>
    )
  }

  return (
    <main className="page-wrap app-main px-4">
      <section className="panel panel-pad mx-auto max-w-md">
        <p className="island-kicker mb-2">WaveFlow desktop</p>
        <h1 className="display-title mb-4 text-2xl font-bold text-(--sea-ink)">
          Couldn&apos;t issue a desktop token
        </h1>
        <p className="text-sm text-(--sea-ink-soft)">
          Better Auth wouldn&apos;t mint a JWT for this session. Try signing out and back in, or
          contact your administrator if the problem persists.
        </p>
      </section>
    </main>
  )
}
