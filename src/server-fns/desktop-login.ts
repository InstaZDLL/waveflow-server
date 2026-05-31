// Server function backing the `/desktop-login` route — kept in its
// own file so the route component (which gets bundled into the
// client) never imports `@tanstack/react-start/server`. The TanStack
// plugin would otherwise reject the route file at build time.

import { createServerFn } from '@tanstack/react-start'
import { getRequestHeaders } from '@tanstack/react-start/server'
import { APIError } from 'better-auth/api'
import { auth } from '@/lib/auth'

/**
 * Resolution of the desktop OAuth handshake. Discriminated union so
 * the caller (the route's `beforeLoad`) can either throw a redirect,
 * surface a friendly error page, or pivot the user through
 * `/sign-in`.
 *
 * - `redirect`: server minted a JWT. The route MUST throw the URL as
 *   a server-side redirect — never render it to the DOM.
 * - `no-session`: caller should redirect to `/sign-in?continue=…`.
 * - `invalid-callback`: the `cb` query param failed loopback
 *   validation; render the error page.
 * - `mint-failed`: a valid session existed but Better Auth refused
 *   to mint a JWT (config drift, etc.); render the error page.
 */
export type DesktopLoginResolution =
  | { kind: 'redirect'; url: string }
  | { kind: 'no-session' }
  | { kind: 'invalid-callback' }
  | { kind: 'mint-failed' }

interface DesktopLoginInput {
  cb: string
  state: string
}

/**
 * Strict loopback validator. Anything other than a plain-`http://`
 * loopback URL is rejected — accepting an arbitrary URL would turn
 * this route into a token-leak vector (a phishing link could pivot
 * the redirect at an attacker-controlled host).
 *
 * Rules:
 * - protocol = `http:` (the desktop's `tiny_http` listener is plain)
 * - hostname ∈ {`127.0.0.1`, `localhost`, `[::1]`}
 * - port ∈ [1024, 65535] (non-privileged)
 *
 * Exported solely so the route-level test can exercise every rejection
 * path against the actual implementation — the route consumes the
 * function through `resolveDesktopLogin`, not directly.
 */
export function parseLoopback(raw: string): URL | null {
  let url: URL
  try {
    url = new URL(raw)
  } catch {
    return null
  }
  if (url.protocol !== 'http:') return null
  const host = url.hostname.toLowerCase()
  if (host !== '127.0.0.1' && host !== 'localhost' && host !== '[::1]') {
    return null
  }
  const port = Number(url.port)
  if (!Number.isInteger(port) || port < 1024 || port > 65535) {
    return null
  }
  return url
}

export const resolveDesktopLogin = createServerFn({ method: 'GET' })
  .inputValidator((value: unknown): DesktopLoginInput => {
    const raw = value as Partial<DesktopLoginInput> | undefined
    return {
      cb: typeof raw?.cb === 'string' ? raw.cb : '',
      state: typeof raw?.state === 'string' ? raw.state.trim() : '',
    }
  })
  .handler(async ({ data }): Promise<DesktopLoginResolution> => {
    const cb = parseLoopback(data.cb)
    if (!cb || !data.state) {
      return { kind: 'invalid-callback' }
    }

    const headers = getRequestHeaders()
    let session
    try {
      session = await auth.api.getSession({
        headers: new Headers(headers as HeadersInit),
      })
    } catch (err) {
      if (err instanceof APIError && err.status === 'UNAUTHORIZED') {
        return { kind: 'no-session' }
      }
      throw err
    }
    if (!session?.user) {
      return { kind: 'no-session' }
    }

    let minted
    try {
      minted = await auth.api.getToken({
        headers: new Headers(headers as HeadersInit),
      })
    } catch (err) {
      if (err instanceof APIError && err.status === 'UNAUTHORIZED') {
        // Lost the session between checks — fall back to the sign-in
        // path rather than the generic mint-failed page so the user
        // can recover by re-entering credentials.
        return { kind: 'no-session' }
      }
      console.error('[desktop-login] getToken failed:', err)
      return { kind: 'mint-failed' }
    }
    if (!minted?.token) {
      return { kind: 'mint-failed' }
    }

    const redirectUrl = new URL(cb.toString())
    redirectUrl.searchParams.set('token', minted.token)
    redirectUrl.searchParams.set('state', data.state)
    return { kind: 'redirect', url: redirectUrl.toString() }
  })
