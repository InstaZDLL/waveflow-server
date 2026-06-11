// Shared internals for the server functions in this folder. Lives on
// the Nitro side only — never bundled into the browser.
//
// `mintToken` resolves a Better Auth session into a fresh JWT.
// `withSafeErrors` wraps a server-fn body so every failure mode maps
// to a client-safe message before re-throwing, keeping diagnostic
// detail (SQL text, JWKS errors) server-side via `console.error`.

import { getRequestHeaders } from '@tanstack/react-start/server'
import { APIError } from 'better-auth/api'
import { auth } from '@/lib/auth'
import { WaveflowServerError } from '@/lib/server/waveflow-server'

/**
 * Marker error for "no Better Auth session on this request". Thrown
 * by [`mintToken`] so the caller's catch can distinguish it from a
 * transport-level failure (DB unreachable, JWKS broken) and map it
 * to a "session expired" hint rather than the generic message.
 */
export class NotSignedInError extends Error {
  constructor() {
    super('Not signed in')
    this.name = 'NotSignedInError'
  }
}

/**
 * Mint a fresh JWT for the current Better Auth session. Throws
 * [`NotSignedInError`] when no session is present — Better Auth
 * surfaces that as an `APIError` with `status: 'UNAUTHORIZED'`, NOT
 * a `{ token: undefined }` payload, so we intercept the throw rather
 * than null-check the return.
 *
 * Anything else (DB unreachable, JWKS broken) bubbles up unchanged
 * — the caller's [`withSafeErrors`] envelope sanitises it.
 */
export async function mintToken(): Promise<string> {
  const headers = getRequestHeaders()
  try {
    const result = await auth.api.getToken({
      headers: new Headers(headers as HeadersInit),
    })
    // Defensive null-check in case a future Better Auth release
    // flips to "return null on no-session" semantics — the marker
    // error stays the right outcome either way.
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

/**
 * Wrap a server-fn body so every failure resolves to a client-safe
 * Error. Maps:
 * - [`NotSignedInError`] → "Session expired. Please sign in again."
 * - [`WaveflowServerError`] → safe message keyed on status
 * - anything else → generic "Could not reach waveflow-server"
 *
 * `label` lands in the server-side `console.error` so an operator
 * can correlate the sanitised client message back to a specific
 * call site.
 */
export async function withSafeErrors<T>(label: string, body: () => Promise<T>): Promise<T> {
  try {
    return await body()
  } catch (err) {
    if (err instanceof NotSignedInError) {
      throw new Error('Session expired. Please sign in again.', { cause: err })
    }
    if (err instanceof WaveflowServerError) {
      console.error(`[server-fn] ${label} → waveflow-server ${err.status}:`, err.message)
      throw new Error(safeMessageForStatus(err.status), { cause: err })
    }
    console.error(`[server-fn] ${label} failed:`, err)
    throw new Error('Could not reach waveflow-server. Please try again.', { cause: err })
  }
}

/** Map a waveflow-server status to a message safe to render in the UI. */
function safeMessageForStatus(status: number): string {
  if (status === 401) return 'Session expired. Please sign in again.'
  if (status === 403) return 'Access denied.'
  if (status === 404) return 'Not found.'
  if (status >= 500) return 'waveflow-server is unavailable. Please try again.'
  return 'Request failed. Please try again.'
}

/**
 * Coerce + validate a path-parameter integer. The id segments end
 * up interpolated into URLs hitting waveflow-server, so anything
 * the validator lets through could (in the worst case) twist the
 * request path. TanStack Start deserialises server-fn payloads as
 * JSON, but a malicious caller can still hand-craft a string
 * payload — keeping the parse + range check on the server side
 * makes the guarantee explicit instead of relying on the type
 * system at a process boundary.
 */
export function asPathId(value: unknown, label: string): number {
  const n = typeof value === 'number' ? value : Number(value)
  if (!Number.isInteger(n) || n <= 0 || n > Number.MAX_SAFE_INTEGER) {
    throw new Error(`${label} must be a positive integer`)
  }
  return n
}
