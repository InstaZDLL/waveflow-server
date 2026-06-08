// PlayableTrackList — shared track-list renderer for every browse
// surface that lists `Track` rows (library tracks, album drill-down,
// artist drill-down). Centralises the per-row click → playTrack
// flow, the pending-spinner pattern, and the queue-seeding logic
// (everything AFTER the clicked row becomes the auto-advance
// context) so the three callsites stay in lock-step.
//
// The component owns local UI state only (current pending track id +
// error). The playback truth lives in `usePlayer` from
// `@/lib/player-context`. A parent that already has its own error
// surface can drive the inline error via `error` / `onError` and
// hide the rendered banner.

import { useRef, useState } from 'react'
import { usePlayer, type QueueEntry } from '@/lib/player-context'
import { formatTime } from '@/lib/format-time'
import type { Track } from '@/server-fns/tracks'

interface PlayableTrackListProps {
  profileId: number
  libraryId: number
  tracks: Track[]
  /**
   * Renders when the list is empty. Each browse surface ships its
   * own copy ("No tracks in this library yet." vs "No tracks under
   * this album yet."); centralising the wording would require an
   * i18n round-trip we don't have yet.
   */
  emptyMessage: string
  /**
   * Accessible label for the list container. Required because
   * three browse surfaces (library tracks / album tracks / artist
   * tracks) share this component — a screen-reader user navigating
   * by landmarks needs a way to distinguish them when the page
   * heading alone isn't audible in the current navigation mode.
   */
  label: string
}

export function PlayableTrackList({
  profileId,
  libraryId,
  tracks,
  emptyMessage,
  label,
}: PlayableTrackListProps) {
  const player = usePlayer()
  const [error, setError] = useState<string | null>(null)
  // `player.isLoading` flips on for ANY in-flight URL mint
  // (playTrack / next / previous). To show the spinner on the row
  // the user actually clicked, we track which trackId is pending
  // locally and pair it with the global isLoading.
  const [pendingTrackId, setPendingTrackId] = useState<number | null>(null)
  // Mirror of `pendingTrackId` we can read synchronously from the
  // catch block — `useState` reads always see the captured-render
  // value, not the latest, so a stale `playTrack` failure that
  // resolves AFTER the user clicked a different row would otherwise
  // call `setError` with the old track's failure message while the
  // new track is happily playing. The ref tells us "is THIS click
  // still the active one?" at the moment the error lands.
  const pendingTrackIdRef = useRef<number | null>(null)

  function toQueueEntry(track: Track): QueueEntry {
    return {
      profileId,
      libraryId,
      trackId: track.id,
      title: track.title,
      durationMs: track.duration_ms,
    }
  }

  async function play(track: Track) {
    setError(null)
    // Seed the queue with the surrounding tracks AFTER the clicked
    // one so `next()` auto-advances down the list. URLs aren't
    // minted upfront — `next()` resolves each on demand.
    // `findIndex` would return -1 for a track that's not in the
    // list (impossible from the current callsites — each row was
    // rendered from `tracks` itself — but we guard against it so
    // a future caller that hand-builds a wider play set doesn't
    // accidentally seed the queue with the clicked track at
    // position 0 and replay it on the first `next()` press).
    const startIndex = tracks.findIndex((t) => t.id === track.id)
    const contextQueue = startIndex >= 0 ? tracks.slice(startIndex + 1).map(toQueueEntry) : []
    // Capture the id this invocation owns so a concurrent click
    // (track A in flight, user clicks track B before A resolves)
    // can't have the older call's finally wipe the newer pending
    // marker. The functional setter compares against the LATEST
    // state, not the captured closure value, so the stale closure
    // hazard is gone too.
    const myPending = track.id
    setPendingTrackId(myPending)
    pendingTrackIdRef.current = myPending
    try {
      await player.playTrack(toQueueEntry(track), contextQueue)
    } catch (err) {
      // Only surface the error if THIS click is still the active
      // one — a later click that preempted us already owns the UI
      // and a stale "Could not start playback." would attach to
      // the wrong track.
      if (pendingTrackIdRef.current === myPending) {
        setError(err instanceof Error ? err.message : 'Could not start playback.')
      }
    } finally {
      setPendingTrackId((current) => (current === myPending ? null : current))
      if (pendingTrackIdRef.current === myPending) {
        pendingTrackIdRef.current = null
      }
    }
  }

  if (tracks.length === 0) {
    return <p className="text-base text-[var(--sea-ink-soft)]">{emptyMessage}</p>
  }

  return (
    <>
      {error && (
        <p
          role="alert"
          className="mb-4 rounded-lg border border-red-200 bg-red-50 px-3 py-2 text-sm text-red-700 dark:border-red-900 dark:bg-red-950/40 dark:text-red-300"
        >
          {error}
        </p>
      )}
      <ul aria-label={label} className="divide-y divide-[var(--line)]">
        {tracks.map((track) => {
          const isCurrent = player.current?.trackId === track.id
          const isPending = pendingTrackId === track.id
          return (
            <li key={track.id} className="flex items-center gap-3 py-2">
              <button
                type="button"
                onClick={() => play(track)}
                disabled={isPending}
                aria-label={`Play ${track.title}`}
                className="flex h-9 w-9 flex-shrink-0 items-center justify-center rounded-full border border-[var(--line)] bg-[var(--chip-bg)] transition hover:opacity-90 disabled:opacity-50"
              >
                {isPending ? '…' : '▶'}
              </button>
              <div className="min-w-0 flex-1">
                <p
                  className={`truncate text-sm ${
                    isCurrent ? 'font-semibold text-[var(--sea-ink)]' : 'text-[var(--sea-ink)]'
                  }`}
                >
                  {track.title}
                </p>
                {track.codec && (
                  <p className="truncate text-xs text-[var(--sea-ink-soft)]">{track.codec}</p>
                )}
              </div>
              <span className="text-xs tabular-nums text-[var(--sea-ink-soft)]">
                {formatTime(track.duration_ms / 1000)}
              </span>
            </li>
          )
        })}
      </ul>
    </>
  )
}
