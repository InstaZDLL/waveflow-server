import { createFileRoute, useNavigate } from '@tanstack/react-router'

import { ThemePicker } from '@/components/ThemePicker'
import { authClient, useSession } from '@/lib/auth-client'

export const Route = createFileRoute('/_authed/settings')({
  component: SettingsPage,
})

// Exported for direct unit-render — sidesteps the file-route + router
// shell so a vitest spec can mount the picker in isolation.
export function SettingsPage() {
  return (
    <main className="page-wrap app-main px-4">
      <section className="panel panel-pad mx-auto max-w-3xl">
        <p className="section-eyebrow mb-2">Settings</p>
        <h1 className="display-title mb-6 text-4xl font-bold text-(--sea-ink)">Account</h1>
        <AccountCard />

        <hr className="my-8 border-t border-(--line)" />

        <h2 className="display-title mb-4 text-2xl font-bold text-(--sea-ink) sm:text-3xl">
          Appearance
        </h2>
        <p className="mb-6 max-w-[62ch] text-sm leading-6 text-(--sea-ink-soft)">
          Pick a palette for this browser. The cookie-backed choice is applied before React
          hydrates, so navigation keeps the same colour system from the first paint.
        </p>
        <ThemePicker />
      </section>
    </main>
  )
}

// Exported so a unit test can mount the card without standing up
// the full Settings page (and without needing the ThemePicker's
// theme-context plumbing).
export function AccountCard() {
  const { data: session, isPending } = useSession()
  const navigate = useNavigate()
  // `useSession()` flips `isPending` to true on the very first
  // render even when the parent `_authed` layout has already
  // resolved the session server-side. The skeleton keeps the page
  // from flickering "Loading…" for that single render.
  if (isPending) {
    return (
      <div className="status-card text-sm" aria-live="polite">
        Loading account details…
      </div>
    )
  }
  if (!session?.user) {
    // The `_authed` layout's `beforeLoad` guard normally redirects
    // an unauthenticated visitor to `/sign-in` before this view
    // renders — but a session that expires while the user is
    // sitting on the Settings page can leave `useSession` returning
    // `null` without a navigation. Render an explicit "signed out"
    // fallback so the page stays informative rather than crashing
    // on `session.user.email`.
    return (
      <div className="error-card text-sm" role="alert">
        Your session expired. Please sign in again to manage your account.
      </div>
    )
  }
  async function onSignOut() {
    try {
      await authClient.signOut()
    } catch (err) {
      // Same rationale as the Header's sign-out: network failure
      // leaves the local cookie behind, but the server has almost
      // certainly cleared its side already. Log + navigate anyway
      // so the user lands somewhere sensible.
      console.error('[settings] sign-out failed:', err)
    }
    try {
      await navigate({ to: '/sign-in' })
    } catch (err) {
      console.warn('[settings] post-sign-out navigation failed:', err)
    }
  }
  return (
    <dl className="quiet-panel grid gap-x-6 gap-y-3 p-4 sm:grid-cols-[auto_1fr]">
      <dt className="text-sm font-semibold text-(--sea-ink-soft)">Name</dt>
      <dd className="text-sm text-(--sea-ink)">
        {session.user.name || <span className="italic text-(--sea-ink-soft)">Not set</span>}
      </dd>
      <dt className="text-sm font-semibold text-(--sea-ink-soft)">Email</dt>
      <dd className="text-sm text-(--sea-ink)">{session.user.email}</dd>
      <dt className="text-sm font-semibold text-(--sea-ink-soft)">Member since</dt>
      <dd className="text-sm text-(--sea-ink-soft)">{formatCreatedAt(session.user.createdAt)}</dd>
      <dd className="col-span-full mt-2">
        <button type="button" onClick={onSignOut} className="button button-ghost">
          Sign out
        </button>
      </dd>
    </dl>
  )
}

/**
 * Format Better Auth's `createdAt` (a `Date` over the wire on the
 * react-query payload, but tolerated as a string for resilience to
 * future shape changes) for display. Falls back to a dash for the
 * rare row where the column is somehow missing — better than
 * "Invalid Date".
 */
function formatCreatedAt(value: Date | string | null | undefined): string {
  if (!value) return '—'
  const date = value instanceof Date ? value : new Date(value)
  if (Number.isNaN(date.getTime())) return '—'
  // `dateStyle: 'medium'` gives "Jun 8, 2026" in en-US, "8 juin 2026"
  // in fr-FR, etc. — the browser's locale wins, matching the rest
  // of the app's date rendering convention.
  return new Intl.DateTimeFormat(undefined, { dateStyle: 'medium' }).format(date)
}
