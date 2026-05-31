import { useState } from 'react'
import { Link, createFileRoute, useNavigate } from '@tanstack/react-router'
import { authClient } from '@/lib/auth-client'

interface SignInSearch {
  /**
   * Optional return path the desktop OAuth handshake (Phase
   * 1.f.desktop.1b) sets when redirecting an unsigned user through
   * here. Validated as same-origin + restricted to a known prefix to
   * keep this from becoming an open-redirect vector.
   */
  continue?: string
}

/**
 * Restrict the post-sign-in redirect to internal routes we
 * intentionally hand off to. Today only `/desktop-login` qualifies —
 * anything else falls back to the default `/` landing.
 */
function safeContinueTarget(raw: string | undefined): string {
  if (!raw) return '/'
  if (!raw.startsWith('/desktop-login')) return '/'
  return raw
}

export const Route = createFileRoute('/sign-in')({
  validateSearch: (raw: Record<string, unknown>): SignInSearch => ({
    continue: typeof raw.continue === 'string' ? raw.continue : undefined,
  }),
  component: SignIn,
})

function SignIn() {
  const navigate = useNavigate()
  const search = Route.useSearch()
  const continueTo = safeContinueTarget(search.continue)
  const [email, setEmail] = useState('')
  const [password, setPassword] = useState('')
  const [error, setError] = useState<string | null>(null)
  const [loading, setLoading] = useState(false)

  async function onSubmit(event: React.FormEvent<HTMLFormElement>) {
    event.preventDefault()
    // Trim before both the local check AND the network call so a
    // user who pasted in their email with a stray leading space
    // doesn't see "enter your email" client-side and then "no such
    // user" server-side a moment later.
    const trimmedEmail = email.trim()
    if (!trimmedEmail.includes('@') || !password) {
      setError('Enter your email and password.')
      return
    }
    setError(null)
    setLoading(true)
    try {
      const { error: remote } = await authClient.signIn.email({
        email: trimmedEmail,
        password,
      })
      if (remote) {
        setError(remote.message ?? 'Sign-in failed. Check your credentials.')
        return
      }
      // Same-origin only — `safeContinueTarget` restricts to a
      // known prefix so a crafted link can't pivot the post-login
      // navigate at an external host.
      await navigate({ href: continueTo })
    } catch (err) {
      // Better Auth resolves with `{ error }` on auth failures, so
      // a thrown exception here is a transport-level problem (DNS,
      // CORS, network down). Surface a generic message and let the
      // user retry.
      setError(err instanceof Error ? err.message : 'Network error. Please try again.')
    } finally {
      setLoading(false)
    }
  }

  return (
    <main className="page-wrap px-4 py-12">
      <section className="island-shell mx-auto max-w-md rounded-2xl p-6 sm:p-8">
        <p className="island-kicker mb-2">Welcome back</p>
        <h1 className="display-title mb-4 text-3xl font-bold text-[var(--sea-ink)] sm:text-4xl">
          Sign in
        </h1>
        <form onSubmit={onSubmit} noValidate className="flex flex-col gap-4">
          <label className="flex flex-col gap-1 text-sm font-medium text-[var(--sea-ink)]">
            Email
            <input
              type="email"
              name="email"
              autoComplete="email"
              required
              value={email}
              onChange={(e) => setEmail(e.target.value)}
              className="rounded-xl border border-[var(--line)] bg-white/80 px-3 py-2 text-base text-[var(--sea-ink)] outline-none transition focus:border-[var(--sea)] focus:ring-2 focus:ring-[var(--sea)]/30 dark:bg-black/30"
            />
          </label>

          <label className="flex flex-col gap-1 text-sm font-medium text-[var(--sea-ink)]">
            Password
            <input
              type="password"
              name="password"
              autoComplete="current-password"
              required
              value={password}
              onChange={(e) => setPassword(e.target.value)}
              className="rounded-xl border border-[var(--line)] bg-white/80 px-3 py-2 text-base text-[var(--sea-ink)] outline-none transition focus:border-[var(--sea)] focus:ring-2 focus:ring-[var(--sea)]/30 dark:bg-black/30"
            />
          </label>

          {error && (
            <p role="alert" className="text-sm font-medium text-red-600 dark:text-red-400">
              {error}
            </p>
          )}

          <button
            type="submit"
            disabled={loading}
            className="rounded-xl bg-[var(--sea-ink)] px-4 py-2.5 text-sm font-semibold text-white transition hover:opacity-90 disabled:cursor-not-allowed disabled:opacity-60"
          >
            {loading ? 'Signing in…' : 'Sign in'}
          </button>

          <p className="text-center text-sm text-[var(--sea-ink-soft)]">
            Don&apos;t have an account?{' '}
            <Link to="/sign-up" className="font-semibold text-[var(--sea-ink)] underline">
              Sign up
            </Link>
          </p>
        </form>
      </section>
    </main>
  )
}
