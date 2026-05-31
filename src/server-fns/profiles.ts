// TanStack server functions for the `/api/v1/profiles` surface on
// waveflow-server. Each function lives on the Nitro side: it pulls
// the request headers, mints a JWT off the active Better Auth
// session, and forwards to waveflow-server with the bearer attached.
//
// Browser callers invoke these like normal RPC (`await listProfiles()`)
// — TanStack Start handles the wire serialization.

import { createServerFn } from '@tanstack/react-start'
import { getRequestHeaders } from '@tanstack/react-start/server'
import { APIError } from 'better-auth/api'
import { auth } from '@/lib/auth'
import { waveflowFetch, WaveflowServerError } from '@/lib/server/waveflow-server'

export interface Profile {
  id: number
  name: string
  color_id: string
  avatar_hash: string | null
  created_at: number
  last_used_at: number
}

/**
 * Marker error for "no Better Auth session on this request" — thrown
 * by [`mintToken`] so the caller can map it to a client-safe message
 * inside its own try/catch envelope (vs. leaking the raw
 * `auth.api.getToken` failure shape).
 */
class NotSignedInError extends Error {
  constructor() {
    super('Not signed in')
    this.name = 'NotSignedInError'
  }
}

/**
 * Mint a fresh JWT for the current Better Auth session. Throws
 * [`NotSignedInError`] if there is no active session — Better Auth
 * surfaces that as an `APIError` with `status: 'UNAUTHORIZED'`, NOT
 * as a `{ token: undefined }` payload, so we have to intercept the
 * throw rather than null-check the return.
 *
 * Anything else (DB unreachable, JWKS broken) bubbles up unchanged
 * — the caller's catch envelope sanitizes it.
 */
async function mintToken(): Promise<string> {
  const headers = getRequestHeaders()
  try {
    const result = await auth.api.getToken({
      headers: new Headers(headers as HeadersInit),
    })
    // Defensive null-check: kept in case a future Better Auth release
    // changes the contract to "return null on no-session" instead of
    // throwing — the marker error stays the right outcome either way.
    if (!result?.token) {
      throw new NotSignedInError()
    }
    return result.token
  } catch (err) {
    if (err instanceof APIError && err.status === 'UNAUTHORIZED') {
      throw new NotSignedInError()
    }
    throw err
  }
}

export const listProfiles = createServerFn({ method: 'GET' }).handler(async () => {
  try {
    const token = await mintToken()
    return await waveflowFetch<Profile[]>('/api/v1/profiles', { token })
  } catch (err) {
    // Map every failure mode to a client-safe message before
    // re-throwing. Diagnostic details (SQL text, internal trace
    // fragments, JWKS errors) never reach the browser; the full
    // error stays logged server-side so an operator can correlate.
    if (err instanceof NotSignedInError) {
      throw new Error('Session expired. Please sign in again.')
    }
    if (err instanceof WaveflowServerError) {
      console.error(`[server-fn] listProfiles → waveflow-server ${err.status}:`, err.message)
      throw new Error(safeMessageForStatus(err.status))
    }
    // Either Better Auth threw (DB / JWKS / unexpected) or the
    // fetch rejected before getting a status (DNS, TLS, timeout).
    console.error('[server-fn] listProfiles failed:', err)
    throw new Error('Could not reach waveflow-server. Please try again.')
  }
})

/** Map a waveflow-server status to a message safe to render in the UI. */
function safeMessageForStatus(status: number): string {
  if (status === 401) return 'Session expired. Please sign in again.'
  if (status === 403) return 'Access denied.'
  if (status >= 500) return 'waveflow-server is unavailable. Please try again.'
  return 'Request failed. Please try again.'
}
