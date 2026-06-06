// Render helpers for the public share preview (Phase 1.j.c).
//
// Lives in `lib/` rather than next to the route so the formatters
// can stay covered by a unit test without exposing extra exports
// from a TanStack file-route module (where every export is
// interpreted by the router). The route at `/p/$token` imports
// from here; the test at `share-format.test.ts` exercises both
// helpers directly.

import type { PublicTrack } from '@/server-fns/share'

/**
 * Default canonical origin for the public share routes. Used as the
 * fallback when `BETTER_AUTH_URL` is unset at SSR — matches the
 * hosted deployment so a misconfigured preview env still hands
 * crawlers something resolvable instead of `undefined/p/<token>`.
 *
 * Exported for the unit test to compare against without re-mocking
 * `process.env` per assertion.
 */
export const DEFAULT_CANONICAL_ORIGIN = 'https://waveflow.app'

/**
 * Resolve the canonical origin for share URLs (issue #21). Reads
 * `BETTER_AUTH_URL` since that's already the source of truth for
 * the deployment's public web origin (Better Auth needs it to mint
 * cookies + sign JWTs scoped to this host). A trailing slash is
 * stripped so a downstream `${origin}/p/${token}` template doesn't
 * produce a double slash.
 *
 * Returns the [`DEFAULT_CANONICAL_ORIGIN`] when the env is unset.
 * Empty strings are treated as unset — `process.env` exposes
 * `Ok("")` for exported-but-empty vars on some shells, and an
 * empty origin would produce a bare-slash share URL no scraper
 * would respect.
 */
export function getCanonicalOrigin(): string {
  // `head()` runs SSR-side at the moment a crawler hits the page,
  // so `process.env` is available. The `typeof process` guard keeps
  // a client-side re-render (eg. an in-app navigation that re-runs
  // `head`) from throwing on a ReferenceError.
  const raw =
    typeof process !== 'undefined' && process.env?.BETTER_AUTH_URL
      ? process.env.BETTER_AUTH_URL
      : ''
  const trimmed = raw.replace(/\/+$/, '')
  return trimmed.length > 0 ? trimmed : DEFAULT_CANONICAL_ORIGIN
}

/**
 * Format a `duration_ms` integer as `mm:ss` (or `h:mm:ss` when the
 * track crosses an hour). Returns an empty string when the value is
 * non-positive — pre-1.j.b desktops emit `0` for the snapshot's
 * duration field, and showing `0:00` next to those rows would
 * mislead the reader.
 *
 * `Math.round` rather than `Math.floor`: a 320500 ms track displays
 * as `5:21` (321 s) instead of `5:20` (320 s), matching the desktop
 * UI's rounding. Invisible past one decimal but worth the
 * consistency.
 */
export function formatDuration(durationMs: number): string {
  if (!Number.isFinite(durationMs) || durationMs <= 0) return ''
  const totalSeconds = Math.round(durationMs / 1000)
  const hours = Math.floor(totalSeconds / 3600)
  const minutes = Math.floor((totalSeconds % 3600) / 60)
  const seconds = totalSeconds % 60
  const pad = (n: number) => n.toString().padStart(2, '0')
  return hours > 0 ? `${hours}:${pad(minutes)}:${pad(seconds)}` : `${minutes}:${pad(seconds)}`
}

/**
 * Render a human "X tracks · 32 min" header. Skips the runtime
 * fragment when the cumulative `duration_ms` is non-positive
 * (pre-1.j.b snapshot — duration field absent on the wire).
 *
 * Singular vs plural on `track`: 1 → "1 track", 0 / 2+ → "N tracks".
 * Hour formatting clamps to `Xh YY` (two-digit minute pad) when the
 * total crosses 60 minutes so a long playlist doesn't render as
 * "147 min" — the eye reads "2h 27" faster.
 */
export function formatTrackCountAndRuntime(tracks: PublicTrack[]): string {
  const total = tracks.reduce((acc, t) => acc + Math.max(0, t.duration_ms), 0)
  const count = `${tracks.length} ${tracks.length === 1 ? 'track' : 'tracks'}`
  if (total <= 0) return count
  const minutes = Math.round(total / 60000)
  if (minutes < 60) return `${count} · ${minutes} min`
  const hours = Math.floor(minutes / 60)
  const remainder = minutes % 60
  return `${count} · ${hours} h ${remainder.toString().padStart(2, '0')}`
}
