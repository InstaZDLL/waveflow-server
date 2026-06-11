// NowPlayingOverlay — fullscreen "expanded" view of the current
// track. Opens when the user clicks the cover thumbnail on the
// PlayerBar; closes via the close button, the backdrop, or ESC.
// Same controls as the PlayerBar (prev / play-pause / next +
// seek + volume) at a larger size, plus a big accent-tinted
// cover panel that retints with the active theme.
//
// Focus-trapped via `useFocusTrap` so `aria-modal="true"` is an
// honest promise to assistive tech.

import { useEffect, useRef, useState } from 'react'

import WaveflowLogo from './WaveflowLogo'
import { formatTime } from '@/lib/format-time'
import { useFocusTrap } from '@/lib/use-focus-trap'
import { usePlayer } from '@/lib/player-context'

export interface NowPlayingOverlayProps {
  open: boolean
  onClose: () => void
}

export function NowPlayingOverlay({ open, onClose }: NowPlayingOverlayProps) {
  const player = usePlayer()
  const dialogRef = useRef<HTMLDivElement | null>(null)
  const closeButtonRef = useRef<HTMLButtonElement | null>(null)
  // Local scrub state — same pattern as PlayerBar: the slider
  // mirrors player.position when idle and tracks the thumb during
  // a drag, committing through player.seek() once on release.
  const [seekScrub, setSeekScrub] = useState<number | null>(null)

  useFocusTrap(open, dialogRef)

  // Move focus to the close button on open so keyboard users land
  // inside the dialog immediately. Depends on `hasCurrent` too
  // because the dialog body renders nothing until the URL mints —
  // a `[open]`-only dependency would fire before the close
  // button exists in the DOM and silently no-op.
  const hasCurrent = !!player.current
  useEffect(() => {
    if (open && hasCurrent) closeButtonRef.current?.focus()
  }, [open, hasCurrent])

  // ESC closes.
  useEffect(() => {
    if (!open) return
    function onKeyDown(event: KeyboardEvent) {
      if (event.key === 'Escape') onClose()
    }
    window.addEventListener('keydown', onKeyDown)
    return () => window.removeEventListener('keydown', onKeyDown)
  }, [open, onClose])

  // Reset the scrub when the track changes mid-overlay so the
  // displayed time doesn't freeze on the previous track.
  const currentTrackId = player.current?.trackId
  const [lastTrackId, setLastTrackId] = useState(currentTrackId)
  if (lastTrackId !== currentTrackId) {
    setLastTrackId(currentTrackId)
    setSeekScrub(null)
  }

  if (!open || !player.current) return null

  const current = player.current
  const durationSec = current.durationMs / 1000

  return (
    <>
      <div
        aria-hidden="true"
        onClick={onClose}
        className="fixed inset-0 z-40 bg-black/50 backdrop-blur-sm"
      />
      <div
        ref={dialogRef}
        id="player-now-playing"
        role="dialog"
        aria-modal="true"
        aria-label="Now playing"
        className="fixed inset-x-0 top-0 z-50 mx-auto flex h-screen w-full max-w-3xl flex-col items-center justify-between p-6 sm:p-10"
        // Container is itself focusable so the trap has somewhere
        // to land when nothing inside is interactable.
        tabIndex={-1}
      >
        <header className="flex w-full items-center justify-end">
          <button
            ref={closeButtonRef}
            type="button"
            onClick={onClose}
            aria-label="Close now playing"
            className="flex h-10 w-10 items-center justify-center rounded-full text-[var(--sea-ink)] transition hover:bg-[var(--link-bg-hover)]"
          >
            <svg viewBox="0 0 24 24" width="22" height="22" aria-hidden="true">
              <path
                d="M6 6l12 12M18 6L6 18"
                stroke="currentColor"
                strokeWidth="2"
                strokeLinecap="round"
                fill="none"
              />
            </svg>
          </button>
        </header>

        <div className="flex flex-col items-center gap-6 text-center">
          <div
            className="flex h-52 w-52 items-center justify-center rounded-2xl shadow-[0_20px_60px_rgba(0,0,0,0.18)] sm:h-72 sm:w-72"
            style={{ backgroundColor: 'var(--accent-100)', color: 'var(--accent-700)' }}
          >
            <WaveflowLogo size={120} label={null} />
          </div>
          <div className="flex flex-col gap-1">
            <h2 className="text-2xl font-bold text-[var(--sea-ink)] sm:text-3xl">
              {current.title}
            </h2>
            {current.artist && (
              <p className="text-base text-[var(--sea-ink-soft)]">{current.artist}</p>
            )}
          </div>
        </div>

        <div className="flex w-full max-w-xl flex-col gap-4">
          <div className="flex items-center gap-3">
            <span className="w-12 text-right text-xs tabular-nums text-[var(--sea-ink-soft)]">
              {formatTime(seekScrub ?? player.position)}
            </span>
            <input
              type="range"
              min={0}
              max={durationSec || 0}
              step={1}
              value={seekScrub ?? player.position}
              onChange={(e) => setSeekScrub(Number(e.target.value))}
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
              className="min-w-0 flex-1"
              style={{ accentColor: 'var(--accent-600)' }}
            />
            <span className="w-12 text-left text-xs tabular-nums text-[var(--sea-ink-soft)]">
              {formatTime(durationSec)}
            </span>
          </div>

          <div className="flex items-center justify-center gap-4">
            <button
              type="button"
              onClick={player.previous}
              aria-label="Previous track"
              disabled={player.isLoading}
              className="flex h-12 w-12 items-center justify-center rounded-full text-[var(--sea-ink)] transition hover:bg-[var(--link-bg-hover)] disabled:opacity-40"
            >
              <svg viewBox="0 0 24 24" width="26" height="26" aria-hidden="true">
                <path d="M6 6h2v12H6zM9.5 12l8.5-6v12z" fill="currentColor" />
              </svg>
            </button>
            <button
              type="button"
              onClick={player.togglePlayPause}
              aria-label={player.isPlaying ? 'Pause' : 'Play'}
              disabled={player.isLoading}
              className="flex h-16 w-16 items-center justify-center rounded-full text-white transition hover:opacity-90 disabled:opacity-50"
              style={{ backgroundColor: 'var(--sea-ink)' }}
            >
              {player.isPlaying ? (
                <svg viewBox="0 0 24 24" width="28" height="28" aria-hidden="true">
                  <rect x="6" y="5" width="4" height="14" rx="1" fill="currentColor" />
                  <rect x="14" y="5" width="4" height="14" rx="1" fill="currentColor" />
                </svg>
              ) : (
                <svg viewBox="0 0 24 24" width="28" height="28" aria-hidden="true">
                  <path d="M8 5v14l11-7z" fill="currentColor" />
                </svg>
              )}
            </button>
            <button
              type="button"
              onClick={player.next}
              aria-label="Next track"
              disabled={player.isLoading || player.queue.length === 0}
              className="flex h-12 w-12 items-center justify-center rounded-full text-[var(--sea-ink)] transition hover:bg-[var(--link-bg-hover)] disabled:opacity-40"
            >
              <svg viewBox="0 0 24 24" width="26" height="26" aria-hidden="true">
                <path d="M16 6h2v12h-2zM6 6l8.5 6L6 18z" fill="currentColor" />
              </svg>
            </button>
          </div>

          <div className="flex items-center gap-3">
            <svg
              viewBox="0 0 24 24"
              width="18"
              height="18"
              aria-hidden="true"
              className="text-[var(--sea-ink-soft)]"
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
              className="flex-1"
              style={{ accentColor: 'var(--accent-600)' }}
            />
          </div>
        </div>
      </div>
    </>
  )
}
