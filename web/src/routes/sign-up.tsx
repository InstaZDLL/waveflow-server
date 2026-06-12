import { useState } from 'react'
import { Link, createFileRoute, useNavigate } from '@tanstack/react-router'
import { authClient } from '@/lib/auth-client'
import { OAuthButtons, OAuthDivider } from '@/components/OAuthButtons'
import { getEnabledProviders, type EnabledProviders } from '@/server-fns/providers'

export const Route = createFileRoute('/sign-up')({
  // Same SSR loader as sign-in so the OAuth buttons land on the
  // first paint without a client-side roundtrip.
  loader: async (): Promise<EnabledProviders> => getEnabledProviders(),
  component: SignUp,
})

const MIN_PASSWORD = 12
const MAX_PASSWORD = 128

// Exported so tests can mount the component without spinning up the
// router. The file route above is what TanStack Start consumes at
// runtime.
export function SignUp() {
  const navigate = useNavigate()
  // `useLoaderData` returns the route's loader-data type when called
  // inside the component; same provider availability we render
  // OAuth buttons against on the sign-in side.
  const enabledProviders = Route.useLoaderData()
  const [name, setName] = useState('')
  const [email, setEmail] = useState('')
  const [password, setPassword] = useState('')
  const [error, setError] = useState<string | null>(null)
  const [loading, setLoading] = useState(false)

  function validate(trimmedEmail: string): string | null {
    if (!name.trim()) return 'Display name is required.'
    if (!trimmedEmail.includes('@')) return 'Enter a valid email address.'
    if (password.length < MIN_PASSWORD) {
      return `Password must be at least ${MIN_PASSWORD} characters.`
    }
    if (password.length > MAX_PASSWORD) {
      return `Password must be at most ${MAX_PASSWORD} characters.`
    }
    return null
  }

  async function onSubmit(event: React.FormEvent<HTMLFormElement>) {
    event.preventDefault()
    // Trim before validating AND before sending — otherwise a stray
    // leading space sneaks past the includes('@') check and Better
    // Auth bounces it back as "invalid email" a moment later.
    const trimmedEmail = email.trim()
    const local = validate(trimmedEmail)
    if (local) {
      setError(local)
      return
    }
    setError(null)
    setLoading(true)
    try {
      const { error: remote } = await authClient.signUp.email({
        email: trimmedEmail,
        password,
        name: name.trim(),
      })
      if (remote) {
        setError(remote.message ?? 'Sign-up failed. Please try again.')
        return
      }
      await navigate({ to: '/' })
    } catch (err) {
      // Better Auth resolves with `{ error }` for handled failures,
      // so a thrown exception here is a transport-level problem
      // (DNS, CORS, network down). Surface a generic retry hint
      // rather than leaving the button stuck on "Creating account…".
      setError(err instanceof Error ? err.message : 'Network error. Please try again.')
    } finally {
      setLoading(false)
    }
  }

  return (
    <main className="page-wrap app-main px-4">
      <section className="panel panel-pad mx-auto max-w-md">
        <p className="section-eyebrow mb-2">Create account</p>
        <h1 className="display-title mb-5 text-4xl font-bold text-[var(--sea-ink)]">Sign up</h1>
        {(enabledProviders.google || enabledProviders.apple) && (
          <div className="mb-4 flex flex-col gap-2">
            <OAuthButtons enabled={enabledProviders} callbackURL="/" />
            <OAuthDivider />
          </div>
        )}
        <form onSubmit={onSubmit} noValidate className="flex flex-col gap-4">
          <label className="flex flex-col gap-1 text-sm font-medium text-[var(--sea-ink)]">
            Display name
            <input
              type="text"
              name="name"
              autoComplete="name"
              required
              value={name}
              onChange={(e) => setName(e.target.value)}
              className="input text-base"
            />
          </label>

          <label className="flex flex-col gap-1 text-sm font-medium text-[var(--sea-ink)]">
            Email
            <input
              type="email"
              name="email"
              autoComplete="email"
              required
              value={email}
              onChange={(e) => setEmail(e.target.value)}
              className="input text-base"
            />
          </label>

          <label className="flex flex-col gap-1 text-sm font-medium text-[var(--sea-ink)]">
            Password
            <input
              type="password"
              name="password"
              autoComplete="new-password"
              required
              minLength={MIN_PASSWORD}
              maxLength={MAX_PASSWORD}
              value={password}
              onChange={(e) => setPassword(e.target.value)}
              className="input text-base"
            />
            <span className="text-xs font-normal text-[var(--sea-ink-soft)]">
              {MIN_PASSWORD}–{MAX_PASSWORD} characters.
            </span>
          </label>

          {error && (
            <p role="alert" className="error-card text-sm font-medium">
              {error}
            </p>
          )}

          <button
            type="submit"
            disabled={loading}
            className="button button-primary w-full"
          >
            {loading ? 'Creating account…' : 'Sign up'}
          </button>

          <p className="text-center text-sm text-[var(--sea-ink-soft)]">
            Already have an account?{' '}
            <Link to="/sign-in" className="font-semibold text-[var(--sea-ink)] underline">
              Sign in
            </Link>
          </p>
        </form>
      </section>
    </main>
  )
}
