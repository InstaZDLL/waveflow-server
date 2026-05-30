import { useState } from 'react'
import { Link, createFileRoute, useNavigate } from '@tanstack/react-router'
import { authClient } from '@/lib/auth-client'

export const Route = createFileRoute('/sign-up')({
  component: SignUp,
})

const MIN_PASSWORD = 12
const MAX_PASSWORD = 128

// Exported so tests can mount the component without spinning up the
// router. The file route above is what TanStack Start consumes at
// runtime.
export function SignUp() {
  const navigate = useNavigate()
  const [name, setName] = useState('')
  const [email, setEmail] = useState('')
  const [password, setPassword] = useState('')
  const [error, setError] = useState<string | null>(null)
  const [loading, setLoading] = useState(false)

  function validate(): string | null {
    if (!name.trim()) return 'Display name is required.'
    if (!email.includes('@')) return 'Enter a valid email address.'
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
    const local = validate()
    if (local) {
      setError(local)
      return
    }
    setError(null)
    setLoading(true)
    const { error: remote } = await authClient.signUp.email({
      email,
      password,
      name: name.trim(),
    })
    setLoading(false)
    if (remote) {
      setError(remote.message ?? 'Sign-up failed. Please try again.')
      return
    }
    await navigate({ to: '/' })
  }

  return (
    <main className="page-wrap px-4 py-12">
      <section className="island-shell mx-auto max-w-md rounded-2xl p-6 sm:p-8">
        <p className="island-kicker mb-2">Create account</p>
        <h1 className="display-title mb-4 text-3xl font-bold text-[var(--sea-ink)] sm:text-4xl">
          Sign up
        </h1>
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
              className="rounded-xl border border-[var(--line)] bg-white/80 px-3 py-2 text-base text-[var(--sea-ink)] outline-none transition focus:border-[var(--sea)] focus:ring-2 focus:ring-[var(--sea)]/30 dark:bg-black/30"
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
              className="rounded-xl border border-[var(--line)] bg-white/80 px-3 py-2 text-base text-[var(--sea-ink)] outline-none transition focus:border-[var(--sea)] focus:ring-2 focus:ring-[var(--sea)]/30 dark:bg-black/30"
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
              className="rounded-xl border border-[var(--line)] bg-white/80 px-3 py-2 text-base text-[var(--sea-ink)] outline-none transition focus:border-[var(--sea)] focus:ring-2 focus:ring-[var(--sea)]/30 dark:bg-black/30"
            />
            <span className="text-xs font-normal text-[var(--sea-ink-soft)]">
              {MIN_PASSWORD}–{MAX_PASSWORD} characters.
            </span>
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
