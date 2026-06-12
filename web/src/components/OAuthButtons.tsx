// "Continue with Google / Apple" buttons for the sign-in and
// sign-up routes. Renders only the providers Better Auth has
// credentials for (per the route's loader, see
// `server-fns/providers.ts`). Reused across both routes so adding
// a provider lands in one place.
//
// Click handler delegates to `authClient.signIn.social({
// provider, callbackURL })`. Better Auth's redirect flow takes
// over from there — the user lands on the provider's consent
// screen, then back at `callbackURL` once the session cookie is
// minted. We DON'T do anything fancy with the result on this side
// because the redirect navigates the entire document.

import { useState } from 'react'
import { authClient } from '@/lib/auth-client'
import type { EnabledProviders } from '@/server-fns/providers'

/**
 * Provider metadata pinned at module scope so adding a row to
 * `EnabledProviders` requires touching this list — keeps a future
 * `microsoft` flag from rendering nothing because the lookup
 * silently falls through.
 */
const PROVIDER_META = [
  {
    id: 'google' as const,
    label: 'Continue with Google',
    // Multi-color "G" mark — same shapes Google publishes in their
    // branding pack. Kept inline so the bundle doesn't ship a
    // dependency for two SVG paths.
    icon: (
      <svg viewBox="0 0 18 18" aria-hidden="true" className="h-4 w-4 shrink-0">
        <path
          d="M17.64 9.2c0-.637-.057-1.251-.164-1.84H9v3.481h4.844a4.14 4.14 0 0 1-1.796 2.716v2.258h2.908c1.702-1.567 2.684-3.875 2.684-6.615z"
          fill="#4285F4"
        />
        <path
          d="M9 18c2.43 0 4.467-.806 5.956-2.184l-2.908-2.259c-.806.54-1.837.86-3.048.86-2.344 0-4.328-1.584-5.036-3.711H.957v2.332A8.997 8.997 0 0 0 9 18z"
          fill="#34A853"
        />
        <path
          d="M3.964 10.706A5.41 5.41 0 0 1 3.682 9c0-.593.102-1.17.282-1.706V4.962H.957A8.996 8.996 0 0 0 0 9c0 1.452.348 2.827.957 4.038l3.007-2.332z"
          fill="#FBBC05"
        />
        <path
          d="M9 3.58c1.321 0 2.508.454 3.44 1.345l2.582-2.58C13.463.891 11.426 0 9 0A8.997 8.997 0 0 0 .957 4.962L3.964 7.294C4.672 5.167 6.656 3.58 9 3.58z"
          fill="#EA4335"
        />
      </svg>
    ),
  },
  {
    id: 'apple' as const,
    label: 'Continue with Apple',
    // Solid silhouette of Apple's wordmark. `currentColor` adapts
    // to dark / light themes via the parent button's text colour.
    icon: (
      <svg viewBox="0 0 24 24" aria-hidden="true" className="h-4 w-4 shrink-0" fill="currentColor">
        <path d="M16.365 1.43c0 1.14-.43 2.222-1.286 3.19-.917 1.03-2.024 1.629-3.215 1.532a3.65 3.65 0 0 1-.022-.385c0-1.11.498-2.281 1.387-3.225C13.658 2.077 14.345 1.6 15.15 1.347c.401-.127.79-.18 1.171-.18.027.087.044.175.044.263zM20.5 17.2c-.59 1.365-.873 1.974-1.634 3.183-1.062 1.69-2.56 3.798-4.415 3.815-1.65.015-2.073-1.075-4.31-1.064-2.235.011-2.7 1.082-4.35 1.067-1.857-.017-3.275-1.92-4.337-3.611C-1.5 16.92-1.81 11.42 1.06 8.49c2.034-2.084 4.27-2.205 5.86-2.205 1.61 0 2.62.892 4.273.892 1.6 0 2.57-.893 4.43-.893 1.396 0 2.875.762 3.93 2.077-3.452 1.892-2.89 6.823.947 8.84z" />
      </svg>
    ),
  },
]

interface Props {
  enabled: EnabledProviders
  /**
   * Where Better Auth should land the user after a successful
   * social sign-in. The sign-in route passes through its
   * `safeContinueTarget` here so the desktop handshake's
   * `?continue=` survives the OAuth round-trip.
   */
  callbackURL: string
}

export function OAuthButtons({ enabled, callbackURL }: Props) {
  const [pending, setPending] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)

  const visible = PROVIDER_META.filter((p) => enabled[p.id])
  if (visible.length === 0) return null

  async function onClick(providerId: 'google' | 'apple') {
    setError(null)
    setPending(providerId)
    try {
      await authClient.signIn.social({
        provider: providerId,
        callbackURL,
      })
      // Better Auth's `signIn.social` navigates the entire document
      // to the provider, so reaching this line means the redirect
      // didn't take — surface a generic error (same wording as the
      // catch branch since the user can't tell which path failed)
      // and let the user retry. Without `setError` the button just
      // re-enables silently and the user has no signal that the
      // click did nothing.
      setError('Sign-in redirect failed.')
      setPending(null)
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Sign-in redirect failed.')
      setPending(null)
    }
  }

  return (
    <div className="flex flex-col gap-2">
      {visible.map((provider) => (
        <button
          key={provider.id}
          type="button"
          onClick={() => onClick(provider.id)}
          disabled={pending !== null}
          className="button button-ghost w-full"
        >
          {provider.icon}
          <span>{pending === provider.id ? 'Redirecting…' : provider.label}</span>
        </button>
      ))}
      {error && (
        <p role="alert" className="error-card text-sm font-medium">
          {error}
        </p>
      )}
    </div>
  )
}

/**
 * Horizontal divider with a "OR" label centred in the middle —
 * separates the OAuth buttons from the email/password form below.
 * Exported alongside the buttons so the consuming route doesn't
 * have to re-derive the markup; rendered only when at least one
 * OAuth provider is enabled.
 */
export function OAuthDivider() {
  return (
    <div className="my-2 flex items-center gap-3 text-xs uppercase tracking-wider text-[var(--sea-ink-soft)]">
      <span className="h-px flex-1 bg-[var(--line)]" />
      <span>or</span>
      <span className="h-px flex-1 bg-[var(--line)]" />
    </div>
  )
}
