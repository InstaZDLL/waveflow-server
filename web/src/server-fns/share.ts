// Server functions for the public `/api/v1/share/*` surface (Phase
// 1.g.2 — the read side of playlist sharing). Mint + revoke stay on
// the desktop / authed callers; the web only consumes the read
// endpoint that powers the `/p/$token` route.

import { notFound } from '@tanstack/react-router'
import { createServerFn } from '@tanstack/react-start'
import { waveflowFetchPublic, WaveflowServerError } from '@/lib/server/waveflow-server'

/**
 * Minimal placeholder track shape mirroring the server's
 * `PublicTrack` DTO. Empty for every playlist today (server-side
 * `playlist_track` materialisation is a separate Phase 1.g.x); the
 * field exists so a future server release can populate it without
 * a wire-break.
 */
export interface PublicTrack {
  title: string
  artist: string | null
  duration_ms: number
}

/**
 * Public preview of a shared playlist. Mirrors the server's
 * `PublicPlaylistResponse` — name + description + cover + brand
 * tokens + timestamps + the tracks list (populated by Phase 1.j
 * desktop emitters; older clients leave it empty and the route
 * renders the "no preview yet" fallback). Notably absent:
 * `profile_id` and any other tenant-identifying fields. Same DTO
 * drives the `<meta>` tag rendering on `/p/$token`.
 */
export interface PublicPlaylist {
  id: number
  name: string
  description: string | null
  color_id: string
  icon_id: string
  cover_hash: string | null
  created_at: number
  updated_at: number
  tracks: PublicTrack[]
}

/**
 * The server-fn returns a discriminated union for the "reached
 * waveflow-server" path. "Playlist not found" is signalled out-
 * of-band via `throw notFound()` — the router catches it,
 * renders the route's `notFoundComponent` and emits a real HTTP
 * 404. Going through the union for the 404 case would force the
 * TanStack RPC client to swallow a non-OK HTTP response, which
 * it treats as a thrown error (see `serverFnFetcher.ts:188`).
 */
export type PublicPlaylistResult =
  | { kind: 'ok'; playlist: PublicPlaylist }
  | { kind: 'error'; message: string }

/**
 * Reject obviously malformed tokens at the boundary so a hand-
 * crafted server-fn payload can't trick us into proxying a path
 * like `/api/v1/share/playlists/../whatever` to waveflow-server.
 *
 * The server mints exactly 32 alphanumeric characters
 * (`rand::distributions::Alphanumeric`); we mirror that exactly
 * on the client. Tight enough to block any URL-meaningful
 * character (slash, dot, query, hash, percent) and any deviation
 * from the server's mint format.
 *
 * The check is consulted by the handler — not the input validator
 * — so a malformed token resolves to `{ kind: 'not_found' }` and
 * the route renders the friendly empty state instead of crashing
 * into the framework's error boundary.
 */
export function isWellShapedToken(value: unknown): value is string {
  return typeof value === 'string' && /^[A-Za-z0-9]{32}$/.test(value)
}

export const getPublicPlaylist = createServerFn({ method: 'GET' })
  .inputValidator((value: unknown): string => {
    // Pass through as a string — shape validation happens in the
    // handler so a bad token can surface as a 'not_found' result
    // rather than a thrown error the route can't render.
    return typeof value === 'string' ? value : ''
  })
  .handler(async ({ data: token }): Promise<PublicPlaylistResult> => {
    // Throw `notFound()` for the 404 cases — TanStack serialises
    // it through the RPC layer as a recognised payload (vs.
    // throwing every non-OK HTTP as an Error), so the client
    // loader sees a real `NotFoundError` and the router routes
    // to `notFoundComponent` with a proper SSR 404 status.
    if (!isWellShapedToken(token)) {
      throw notFound()
    }
    try {
      const playlist = await waveflowFetchPublic<PublicPlaylist>(
        `/api/v1/share/playlists/${encodeURIComponent(token)}`,
      )
      return { kind: 'ok', playlist }
    } catch (err) {
      if (err instanceof WaveflowServerError && err.status === 404) {
        throw notFound()
      }
      console.error('[server-fn] getPublicPlaylist failed:', err)
      return {
        kind: 'error',
        message: 'Could not reach waveflow-server. Please try again.',
      }
    }
  })
