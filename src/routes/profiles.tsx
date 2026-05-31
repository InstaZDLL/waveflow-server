import { useEffect, useState } from 'react'
import { createFileRoute, useNavigate } from '@tanstack/react-router'
import { listProfiles, type Profile } from '@/server-fns/profiles'
import { useSession } from '@/lib/auth-client'

export const Route = createFileRoute('/profiles')({
  component: ProfilesView,
})

type State =
  | { kind: 'loading' }
  | { kind: 'ready'; profiles: Profile[] }
  | { kind: 'error'; message: string }

function ProfilesView() {
  const navigate = useNavigate()
  const { data: session, isPending } = useSession()
  const [state, setState] = useState<State>({ kind: 'loading' })

  // Redirect to sign-in once the session has resolved and there's no
  // user — Better Auth's session-cookie lookup runs on mount, so
  // gating before that resolves would force an unnecessary trip
  // through /sign-in even for an already-signed-in user.
  useEffect(() => {
    if (!isPending && !session?.user) {
      void navigate({ to: '/sign-in' })
    }
  }, [isPending, navigate, session])

  useEffect(() => {
    if (isPending || !session?.user) return
    let cancelled = false
    listProfiles()
      .then((profiles) => {
        if (cancelled) return
        setState({ kind: 'ready', profiles })
      })
      .catch((err: unknown) => {
        if (cancelled) return
        setState({
          kind: 'error',
          message: err instanceof Error ? err.message : 'Failed to load profiles.',
        })
      })
    return () => {
      cancelled = true
    }
  }, [isPending, session])

  return (
    <main className="page-wrap px-4 py-12">
      <section className="island-shell rounded-2xl p-6 sm:p-8">
        <p className="island-kicker mb-2">Profiles</p>
        <h1 className="display-title mb-4 text-3xl font-bold text-[var(--sea-ink)] sm:text-4xl">
          Your profiles
        </h1>

        {state.kind === 'loading' && (
          <p className="text-base text-[var(--sea-ink-soft)]">Loading…</p>
        )}

        {state.kind === 'error' && (
          <p role="alert" className="text-base text-red-600 dark:text-red-400">
            {state.message}
          </p>
        )}

        {state.kind === 'ready' && state.profiles.length === 0 && (
          <p className="text-base text-[var(--sea-ink-soft)]">
            No profiles yet. The desktop app or the API will create the first one for you.
          </p>
        )}

        {state.kind === 'ready' && state.profiles.length > 0 && (
          <ul className="grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
            {state.profiles.map((p) => (
              <li
                key={p.id}
                className="rounded-xl border border-[var(--line)] bg-[var(--chip-bg)] p-4"
              >
                <p className="text-base font-semibold text-[var(--sea-ink)]">{p.name}</p>
                <p className="mt-1 text-xs text-[var(--sea-ink-soft)]">
                  Last used {new Date(p.last_used_at).toLocaleDateString()}
                </p>
              </li>
            ))}
          </ul>
        )}
      </section>
    </main>
  )
}
