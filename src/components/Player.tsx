import { useEffect, useRef, useState } from 'react'

export interface PlayingTrack {
  /** Track id, used as the React key on the audio element so a swap remounts the player cleanly. */
  id: number
  title: string
  /** Absolute, signed streaming URL — the same string the server fn returned. */
  url: string
  /** Optional artist line for the now-playing label. */
  artist?: string
  /** Total duration in ms, for the seek bar denominator. */
  durationMs: number
}

interface PlayerProps {
  current: PlayingTrack | null
  /** Called when the audio element reports EOF, so the parent can advance the queue. */
  onEnded?: () => void
}

/**
 * Sticky player bar pinned to the bottom of the viewport. Renders
 * nothing when no track is selected. Callers MUST pass
 * `key={current?.id ?? 'idle'}` (or similar) on the component so a
 * track swap remounts the player — that's how we reset `playing` /
 * `position` without a `setState` in an effect (cf. React's
 * "you might not need an effect" guidance).
 */
export function Player({ current, onEnded }: PlayerProps) {
  const audioRef = useRef<HTMLAudioElement | null>(null)
  const [playing, setPlaying] = useState(false)
  const [position, setPosition] = useState(0)

  // Autoplay the new track once the metadata is ready. `<audio>`
  // emits `canplay` when enough data has buffered to start playback;
  // calling `.play()` earlier rejects the promise in Chrome.
  //
  // `readyState >= HAVE_FUTURE_DATA` (= 3) means `canplay` already
  // fired before we attached — happens when the browser cache
  // serves the response synchronously, or after a fast remount.
  // Call the handler immediately in that case so we don't sit on
  // an event that will never fire again.
  useEffect(() => {
    const el = audioRef.current
    if (!el || !current) return
    const tryPlay = () => {
      el.play()
        .then(() => setPlaying(true))
        .catch((err) => {
          // Autoplay policies block playback without a recent user
          // gesture in some browsers. The user can hit the play
          // button to start; we just log for visibility.
          console.warn('[player] autoplay rejected:', err)
        })
    }
    if (el.readyState >= HTMLMediaElement.HAVE_FUTURE_DATA) {
      tryPlay()
      return
    }
    el.addEventListener('canplay', tryPlay)
    return () => el.removeEventListener('canplay', tryPlay)
  }, [current])

  if (!current) return null

  function toggle() {
    const el = audioRef.current
    if (!el) return
    if (el.paused) {
      el.play().then(
        () => setPlaying(true),
        (err) => console.warn('[player] play() rejected:', err),
      )
    } else {
      el.pause()
      setPlaying(false)
    }
  }

  function onSeek(event: React.ChangeEvent<HTMLInputElement>) {
    const el = audioRef.current
    if (!el) return
    const next = Number(event.target.value)
    el.currentTime = next
    setPosition(next)
  }

  const durationSec = current.durationMs / 1000

  return (
    <div
      role="region"
      aria-label="Player"
      className="fixed inset-x-0 bottom-0 z-40 border-t border-[var(--line)] bg-[var(--header-bg)] backdrop-blur-lg"
    >
      <div className="page-wrap flex items-center gap-4 px-4 py-3 sm:py-4">
        <div className="min-w-0 flex-1">
          <p className="truncate text-sm font-semibold text-[var(--sea-ink)]">{current.title}</p>
          {current.artist && (
            <p className="truncate text-xs text-[var(--sea-ink-soft)]">{current.artist}</p>
          )}
        </div>

        <button
          type="button"
          onClick={toggle}
          aria-label={playing ? 'Pause' : 'Play'}
          className="flex h-10 w-10 flex-shrink-0 items-center justify-center rounded-full bg-[var(--sea-ink)] text-white transition hover:opacity-90"
        >
          {playing ? (
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

        <input
          type="range"
          min={0}
          max={durationSec}
          step={1}
          value={position}
          onChange={onSeek}
          aria-label="Seek"
          className="hidden flex-1 accent-[var(--sea-ink)] sm:block"
        />
        <span className="hidden w-20 text-right text-xs tabular-nums text-[var(--sea-ink-soft)] sm:inline">
          {formatTime(position)} / {formatTime(durationSec)}
        </span>
      </div>

      {/*
        Keyed by track id so a new track replaces this element rather
        than mutating an existing one. The browser then fetches the new
        URL fresh — no stale buffer carry-over from the previous track.
      */}
      <audio
        key={current.id}
        ref={audioRef}
        src={current.url}
        preload="metadata"
        onTimeUpdate={(e) => setPosition(e.currentTarget.currentTime)}
        onEnded={() => {
          setPlaying(false)
          onEnded?.()
        }}
        className="hidden"
      />
    </div>
  )
}

function formatTime(seconds: number): string {
  if (!Number.isFinite(seconds) || seconds < 0) return '0:00'
  const total = Math.floor(seconds)
  const m = Math.floor(total / 60)
  const s = total % 60
  return `${m}:${s.toString().padStart(2, '0')}`
}
