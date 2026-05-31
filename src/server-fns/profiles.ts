// TanStack server functions for the `/api/v1/profiles` surface on
// waveflow-server. Each function lives on the Nitro side: it pulls
// the request headers, mints a JWT off the active Better Auth
// session, and forwards to waveflow-server with the bearer attached.
//
// Browser callers invoke these like normal RPC (`await listProfiles()`)
// — TanStack Start handles the wire serialization.

import { createServerFn } from '@tanstack/react-start'
import { getRequestHeaders } from '@tanstack/react-start/server'
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
 * Mint a fresh JWT for the current Better Auth session. Throws if
 * the request isn't authenticated — every server fn here calls this
 * first, so the caller bubbles a 401-equivalent up to the UI.
 */
async function mintToken(): Promise<string> {
  const headers = getRequestHeaders()
  const result = await auth.api.getToken({ headers: new Headers(headers as HeadersInit) })
  if (!result?.token) {
    throw new Error('Not signed in')
  }
  return result.token
}

export const listProfiles = createServerFn({ method: 'GET' }).handler(async () => {
  const token = await mintToken()
  try {
    return await waveflowFetch<Profile[]>('/api/v1/profiles', { token })
  } catch (err) {
    // Map the raw backend error to a client-safe message so
    // diagnostic details (SQL text, internal trace fragments) never
    // reach the browser. Full error stays logged server-side so an
    // operator can correlate.
    if (err instanceof WaveflowServerError) {
      console.error(`[server-fn] listProfiles → waveflow-server ${err.status}:`, err.message)
      throw new Error(safeMessageForStatus(err.status))
    }
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
