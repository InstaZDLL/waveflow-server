// PlayerBar — persistent sticky strip at the bottom of every page.
// Reads the shared player state from `PlayerContext` and owns the
// `<audio>` element. Audio events flow back into the context via
// `setIsPlaying` / `setPosition` / `next` so a system-level pause
// (media keys, OS notification center, "another tab took focus")
// stays in sync with the UI.
//
// Renders nothing when `current` is null — the UI doesn't reserve
// space for an empty bar.

import { useEffect, useRef, useState } from 'react'

import { NowPlayingOverlay } from './NowPlayingOverlay'
import { QueuePanel } from './QueuePanel'
import WaveflowLogo from './WaveflowLogo'
import { formatTime } from '@/lib/format-time'
import { usePlayer } from '@/lib/player-context'

export function PlayerBar() {
  const player = usePlayer()
  const audioRef = useRef<HTMLAudioElement | null>(null)
  const queueToggleRef = useRef<HTMLButtonElement | null>(null)
  const nowPlayingTriggerRef = useRef<HTMLButtonElement | null>(null)
  // Local seek-scrub state — the slider mirrors `player.position`
  // when idle but tracks the user's thumb during a drag. We only
  // call `player.seek()` on pointer release so a single drag
  // produces one seek event instead of one per pixel (which would
  // hammer the seekVersion counter + audio.currentTime).
  const [seekScrub, setSeekScrub] = useState<number | null>(null)
  // Drawer state. Queue + Now Playing are mounted as siblings,
  // but their open/closed signals live here so the toggle buttons
  // read the same source of truth. Closing each restores focus to
  // its trigger so keyboard navigation isn't stranded.
  const [queueOpen, setQueueOpen] = useState(false)
  const [nowPlayingOpen, setNowPlayingOpen] = useState(false)
  const closeQueue = () => {
    setQueueOpen(false)
    queueToggleRef.current?.focus()
  }
  const closeNowPlaying = () => {
    setNowPlayingOpen(false)
    nowPlayingTriggerRef.current?.focus()
  }

  // Apply the latest volume to the element whenever it changes.
  // The element's own state isn't reactive — we sync it via effect.
  useEffect(() => {
    if (audioRef.current) audioRef.current.volume = player.volume
  }, [player.volume])

  // Reflect the desired play/pause state on the element. Skips the
  // call when the element is already in the requested state so a
  // re-render doesn't double-fire `.play()`.
  const { isPlaying, setIsPlaying, current } = player
  const currentTrackId = current?.trackId

  // Clear any in-progress scrub when the track changes — auto-
  // advance, next(), and previous() all flip currentTrackId while
  // a user may still be mid-drag on the old track's slider.
  // Without this, formatTime(seekScrub ?? player.position) would
  // freeze on the previous track's timestamp until the user
  // touches the slider again. Uses the "adjust state on prop
  // change" pattern from the React docs so the reset lands BEFORE
  // the render commits — the lint rule rejects setState-inside-
  // effect, an additional render cycle, OR a setSeekScrub here
  // during a track-unchanged render is a cheap bail-out.
  const [lastTrackId, setLastTrackId] = useState(currentTrackId)
  if (lastTrackId !== currentTrackId) {
    setLastTrackId(currentTrackId)
    setSeekScrub(null)
  }

  useEffect(() => {
    const el = audioRef.current
    if (!el) return
    if (isPlaying && el.paused) {
      el.play().catch((err) => {
        // Autoplay policies + transient stream errors land here.
        // Roll the context state back to paused so the UI doesn't
        // sit on a `Pause` icon while audio is silent.
        console.warn('[player] play() rejected:', err)
        setIsPlaying(false)
      })
    } else if (!isPlaying && !el.paused) {
      el.pause()
    }
  }, [isPlaying, setIsPlaying, currentTrackId])

  // Honour a seek() request — the context bumps `seekVersion` each
  // time the user asks for a new position; this effect mirrors the
  // target onto the audio element. Keyed on `seekVersion` so the
  // every-250ms `position` ticks from `onTimeUpdate` don't re-fire
  // it (which would clobber the playhead between updates).
  useEffect(() => {
    const el = audioRef.current
    if (!el) return
    el.currentTime = player.seekTargetSec
    // `player.seekTargetSec` intentionally excluded from the dep
    // array — the seek "event" is the `seekVersion` bump; the
    // target is the value that bump carries.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [player.seekVersion])

  if (!player.current) return null

  const durationSec = player.current.durationMs / 1000

  return (
    <div
      role="region"
      aria-label="Now playing"
      className="fixed inset-x-0 bottom-0 z-40 border-t border-(--line) bg-(--header-bg) shadow-[0_-20px_60px_rgba(0,0,0,0.12)] backdrop-blur-xl"
    >
      <div className="page-wrap flex items-center gap-3 px-3 py-2.5 sm:gap-4 sm:px-4 sm:py-3">
        {/* Cover placeholder doubles as the Now Playing trigger —
            accent-tinted brand mark until the scanner pipes cover
            URLs through the listing payload. */}
        <button
          ref={nowPlayingTriggerRef}
          type="button"
          onClick={() => setNowPlayingOpen(true)}
          aria-label="Open now playing"
          aria-haspopup="dialog"
          aria-expanded={nowPlayingOpen}
          aria-controls="player-now-playing"
          className="art-tile flex h-12 w-12 flex-shrink-0 items-center justify-center transition hover:scale-105 sm:h-14 sm:w-14"
          style={{ backgroundColor: 'var(--accent-100)', color: 'var(--accent-700)' }}
        >
          <WaveflowLogo size={28} label={null} />
        </button>

        <div className="min-w-0 flex-1">
          <p className="truncate text-sm font-bold text-(--sea-ink)">{player.current.title}</p>
          {player.current.artist && (
            <p className="truncate text-xs text-(--sea-ink-soft)">{player.current.artist}</p>
          )}
        </div>

        <div className="flex flex-shrink-0 items-center gap-1.5 sm:gap-2">
          <button
            type="button"
            onClick={player.previous}
            aria-label="Previous track"
            className="flex h-9 w-9 items-center justify-center rounded-xl text-(--sea-ink) transition hover:bg-(--link-bg-hover) disabled:opacity-40"
            disabled={player.isLoading}
          >
            <svg viewBox="0 0 24 24" width="20" height="20" aria-hidden="true">
              <path d="M6 6h2v12H6zM9.5 12l8.5-6v12z" fill="currentColor" />
            </svg>
          </button>

          <button
            type="button"
            onClick={player.togglePlayPause}
            aria-label={player.isPlaying ? 'Pause' : 'Play'}
            disabled={player.isLoading}
            className="flex h-10 w-10 items-center justify-center rounded-xl text-white transition hover:-translate-y-0.5 disabled:opacity-50"
            style={{ backgroundColor: 'var(--sea-ink)' }}
          >
            {player.isLoading ? (
              <svg
                viewBox="0 0 24 24"
                width="20"
                height="20"
                aria-hidden="true"
                className="animate-spin"
              >
                <circle
                  cx="12"
                  cy="12"
                  r="9"
                  stroke="currentColor"
                  strokeWidth="2.5"
                  fill="none"
                  opacity="0.25"
                />
                <path
                  d="M12 3a9 9 0 0 1 9 9"
                  stroke="currentColor"
                  strokeWidth="2.5"
                  fill="none"
                  strokeLinecap="round"
                />
              </svg>
            ) : player.isPlaying ? (
              <svg viewBox="0 0 24 24" width="20" height="20" aria-hidden="true">
                <rect x="6" y="5" width="4" height="14" rx="1" fill="currentColor" />
                <rect x="14" y="5" width="4" height="14" rx="1" fill="currentColor" />
              </svg>
            ) : (
              <svg viewBox="0 0 24 24" width="20" height="20" aria-hidden="true">
                <path d="M8 5v14l11-7z" fill="currentColor" />
              </svg>
            )}
          </button>

          <button
            type="button"
            onClick={player.next}
            aria-label="Next track"
            className="flex h-9 w-9 items-center justify-center rounded-xl text-(--sea-ink) transition hover:bg-(--link-bg-hover) disabled:opacity-40"
            disabled={player.isLoading || player.queue.length === 0}
          >
            <svg viewBox="0 0 24 24" width="20" height="20" aria-hidden="true">
              <path d="M16 6h2v12h-2zM6 6l8.5 6L6 18z" fill="currentColor" />
            </svg>
          </button>
        </div>

        <input
          type="range"
          min={0}
          max={durationSec || 0}
          step={1}
          value={seekScrub ?? player.position}
          onChange={(e) => setSeekScrub(Number(e.target.value))}
          // Pointer events cover mouse + touch + pen in one API.
          // Commit the scrub on release (lift) OR on cancel
          // (pointer leaves the viewport mid-drag). Keyboard arrow
          // adjustments don't fire pointer events — they go through
          // `onKeyUp` so the same commit path runs.
          onPointerUp={() => {
            if (seekScrub !== null) {
              player.seek(seekScrub)
              setSeekScrub(null)
            }
          }}
          onPointerCancel={() => setSeekScrub(null)}
          onKeyUp={(e) => {
            if (
              seekScrub !== null &&
              (e.key === 'ArrowLeft' ||
                e.key === 'ArrowRight' ||
                e.key === 'Home' ||
                e.key === 'End' ||
                e.key === 'PageUp' ||
                e.key === 'PageDown')
            ) {
              player.seek(seekScrub)
              setSeekScrub(null)
            }
          }}
          aria-label="Seek"
          className="hidden min-w-0 flex-1 sm:block"
          style={{ accentColor: 'var(--accent-600)' }}
        />
        <span className="hidden w-20 text-right text-xs tabular-nums text-(--sea-ink-soft) sm:inline">
          {formatTime(seekScrub ?? player.position)} / {formatTime(durationSec)}
        </span>

        <div className="hidden items-center gap-2 lg:flex">
          <svg
            viewBox="0 0 24 24"
            width="18"
            height="18"
            aria-hidden="true"
            className="text-(--sea-ink-soft)"
          >
            <path
              fill="currentColor"
              d="M5 9v6h4l5 4V5L9 9H5zm11.5 3a4.5 4.5 0 0 0-2.5-4.03v8.06A4.5 4.5 0 0 0 16.5 12z"
            />
          </svg>
          <input
            type="range"
            min={0}
            max={1}
            step={0.01}
            value={player.volume}
            onChange={(e) => player.setVolume(Number(e.target.value))}
            aria-label="Volume"
            className="w-24"
            style={{ accentColor: 'var(--accent-600)' }}
          />
        </div>

        <button
          ref={queueToggleRef}
          type="button"
          onClick={() => setQueueOpen((open) => !open)}
          aria-label={queueOpen ? 'Close queue' : 'Open queue'}
          aria-expanded={queueOpen}
          aria-controls="player-queue-panel"
          className="flex h-9 w-9 flex-shrink-0 items-center justify-center rounded-xl text-(--sea-ink) transition hover:bg-(--link-bg-hover)"
        >
          <svg viewBox="0 0 24 24" width="20" height="20" aria-hidden="true">
            <path fill="currentColor" d="M4 6h12v2H4zm0 5h12v2H4zm0 5h8v2H4zm10 0l6-3.5L14 9z" />
          </svg>
        </button>
      </div>

      {/*
        Keyed by track id so a new track replaces this element
        rather than mutating an existing one. The browser then
        fetches the new URL fresh — no stale buffer carry-over from
        the previous track.
      */}
      <audio
        key={player.current.trackId}
        ref={audioRef}
        src={player.current.url}
        preload="metadata"
        onLoadedMetadata={() => {
          // The element's volume defaults to 1 on mount; re-apply
          // the user's choice before the first sample plays.
          if (audioRef.current) audioRef.current.volume = player.volume
        }}
        onPlay={() => player.setIsPlaying(true)}
        onPause={() => player.setIsPlaying(false)}
        onTimeUpdate={(e) => player.setPosition(e.currentTarget.currentTime)}
        onEnded={() => {
          player.setIsPlaying(false)
          // Fire-and-forget. If the queue is empty the context's
          // `next()` is a no-op; if it isn't, the new current's
          // `<audio>` remount triggers autoplay through the
          // play/pause effect above.
          void player.next()
        }}
        className="hidden"
      />
      <QueuePanel open={queueOpen} onClose={closeQueue} />
      <NowPlayingOverlay open={nowPlayingOpen} onClose={closeNowPlaying} />
    </div>
  )
}
