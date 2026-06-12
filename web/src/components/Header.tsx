import { Link, useNavigate } from '@tanstack/react-router'
import { authClient, useSession } from '@/lib/auth-client'
import ThemeToggle from './ThemeToggle'
import WaveflowLogo from './WaveflowLogo'

export default function Header() {
  const navigate = useNavigate()
  const { data: session, isPending } = useSession()

  async function onSignOut() {
    try {
      await authClient.signOut()
    } catch (err) {
      // Network failure leaves the local cookie behind, but the
      // server has almost certainly cleared its side of the
      // session already. Logging keeps the trace for diagnostics;
      // we still redirect so the next page load re-evaluates auth
      // state from scratch instead of stranding the user on the
      // current view.
      console.error('[auth] sign-out failed:', err)
    }
    // Mirror the Settings page's `try/catch` around `navigate` —
    // TanStack Router can reject if a `beforeLoad` guard throws
    // or the user unmounts mid-await. Without the guard the
    // rejection lands as an unhandled promise in the console
    // while the user is stuck on the authed page with a cleared
    // cookie. Logging keeps it diagnosable; we don't surface to
    // the user because the cookie clear already happened.
    try {
      await navigate({ to: '/sign-in' })
    } catch (err) {
      console.warn('[auth] post-sign-out navigation failed:', err)
    }
  }

  return (
    <header className="sticky top-0 z-50 border-b border-(--line) bg-(--header-bg) px-4 backdrop-blur-xl">
      <nav className="page-wrap flex flex-wrap items-center gap-x-3 gap-y-2 py-3">
        <h2 className="m-0 flex-shrink-0 text-base font-semibold tracking-tight">
          <Link
            to="/"
            className="inline-flex items-center gap-2 rounded-xl border border-(--chip-line) bg-(--chip-bg) px-3 py-2 text-sm text-(--sea-ink) no-underline shadow-[0_1px_0_var(--inset-glint)_inset]"
          >
            <span style={{ color: 'var(--accent-600)' }}>
              <WaveflowLogo size={18} label={null} />
            </span>
            WaveFlow
          </Link>
        </h2>

        <div className="order-3 flex w-full flex-wrap items-center gap-x-1 gap-y-1 pb-1 text-sm font-semibold sm:order-none sm:w-auto sm:flex-nowrap sm:pb-0">
          <Link to="/" className="nav-link" activeProps={{ className: 'nav-link is-active' }}>
            Home
          </Link>
          <Link to="/about" className="nav-link" activeProps={{ className: 'nav-link is-active' }}>
            About
          </Link>
          {session?.user && (
            <Link
              to="/profiles"
              className="nav-link"
              activeProps={{ className: 'nav-link is-active' }}
            >
              Library
            </Link>
          )}
          {session?.user && (
            <Link
              to="/settings"
              className="nav-link"
              activeProps={{ className: 'nav-link is-active' }}
            >
              Settings
            </Link>
          )}
        </div>

        <div className="ml-auto flex items-center gap-1.5 sm:gap-2">
          <a
            href="https://github.com/InstaZDLL/WaveFlow"
            target="_blank"
            rel="noreferrer"
            className="hidden rounded-xl p-2 text-(--sea-ink-soft) transition hover:bg-(--link-bg-hover) hover:text-(--sea-ink) sm:block"
          >
            <span className="sr-only">WaveFlow on GitHub</span>
            <svg viewBox="0 0 16 16" aria-hidden="true" width="24" height="24">
              <path
                fill="currentColor"
                d="M8 0C3.58 0 0 3.58 0 8c0 3.54 2.29 6.53 5.47 7.59.4.07.55-.17.55-.38 0-.19-.01-.82-.01-1.49-2.01.37-2.53-.49-2.69-.94-.09-.23-.48-.94-.82-1.13-.28-.15-.68-.52-.01-.53.63-.01 1.08.58 1.23.82.72 1.21 1.87.87 2.33.66.07-.52.28-.87.51-1.07-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82.64-.18 1.32-.27 2-.27.68 0 1.36.09 2 .27 1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48 0 1.07-.01 1.93-.01 2.2 0 .21.15.46.55.38A8.012 8.012 0 0 0 16 8c0-4.42-3.58-8-8-8z"
              />
            </svg>
          </a>

          {/*
            While `useSession()` is still resolving on the first
            render, render nothing in this slot — otherwise the
            sign-in / sign-up links flash for a beat before swapping
            to the authed chip, which reads as "you're signed out"
            to a signed-in user.
          */}
          {isPending ? null : session?.user ? (
            <>
              <span
                className="hidden text-sm text-(--sea-ink-soft) sm:inline"
                title={session.user.email}
              >
                {session.user.name || session.user.email}
              </span>
              <button
                type="button"
                onClick={onSignOut}
                className="button button-ghost min-h-0 px-3 py-2"
              >
                Sign out
              </button>
            </>
          ) : (
            <>
              <Link
                to="/sign-in"
                className="nav-link"
                activeProps={{ className: 'nav-link is-active' }}
              >
                Sign in
              </Link>
              <Link to="/sign-up" className="button button-primary min-h-0 px-3 py-2">
                Sign up
              </Link>
            </>
          )}

          <ThemeToggle />
        </div>
      </nav>
    </header>
  )
}
