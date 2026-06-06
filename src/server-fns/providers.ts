// Surface "which OAuth providers does this deployment have
// credentials for?" to the sign-in / sign-up routes so the UI can
// hide buttons that would otherwise lead to a `/api/auth/sign-in/social`
// endpoint Better Auth won't have registered. Mirrors the
// per-provider env check in `lib/auth.ts` — single source of truth
// for the conditional, so a misconfigured deploy that sets only
// the public client id without the secret behaves identically on
// both sides.

import { createServerFn } from '@tanstack/react-start'

/**
 * Whether each auth provider is wired up at boot. `email` is
 * always true (Better Auth is initialised with
 * `emailAndPassword.enabled: true` unconditionally); OAuth flips
 * to `true` only when BOTH the client id and secret are set, so a
 * half-configured deploy stays hidden from the user.
 */
export interface EnabledProviders {
  email: boolean
  google: boolean
  apple: boolean
}

/**
 * Read the per-provider env at request time so a deploy that adds
 * credentials without a restart still picks them up on the next
 * sign-in render. Server-fn → runs SSR-side, `process.env` is
 * available; the result is part of the route's loader data so the
 * markup never leaks "this provider is unconfigured" into a user-
 * visible 500.
 */
export const getEnabledProviders = createServerFn({ method: 'GET' }).handler(
  async (): Promise<EnabledProviders> => ({
    email: true,
    google: !!process.env.GOOGLE_CLIENT_ID && !!process.env.GOOGLE_CLIENT_SECRET,
    apple: !!process.env.APPLE_CLIENT_ID && !!process.env.APPLE_CLIENT_SECRET,
  }),
)
