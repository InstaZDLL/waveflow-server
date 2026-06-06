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
