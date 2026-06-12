// QueuePanel — right-side drawer that lists the now-playing
// track, the upcoming queue, and the history of recently played
// items. Clicking a queue row jumps playback to it (skipped
// items roll into history so `previous()` still walks back
// through them). History rows are read-only in v1 — re-queueing
// from history lands in 4.b.2 alongside the Now Playing overlay.
//
// Mounted at root next to `<PlayerBar>`. Renders nothing when
// closed.

import { useEffect, useRef } from 'react'

import WaveflowLogo from './WaveflowLogo'
import { useFocusTrap } from '@/lib/use-focus-trap'
import { usePlayer, type PlayingTrack, type QueueEntry } from '@/lib/player-context'

export interface QueuePanelProps {
  open: boolean
  onClose: () => void
}

function formatDuration(ms: number): string {
  const total = Math.floor(ms / 1000)
  const m = Math.floor(total / 60)
  const s = total % 60
  return `${m}:${s.toString().padStart(2, '0')}`
}

export function QueuePanel({ open, onClose }: QueuePanelProps) {
  const player = usePlayer()
  const dialogRef = useRef<HTMLElement | null>(null)
  const closeButtonRef = useRef<HTMLButtonElement | null>(null)

  useFocusTrap(open, dialogRef)

  // Move focus to the close button on open so keyboard users land
  // inside the drawer. Restoring focus to the trigger on close is
  // the responsibility of the caller (PlayerBar holds the
  // imperative ref to the toggle button).
  useEffect(() => {
    if (open) closeButtonRef.current?.focus()
  }, [open])

  // ESC closes the drawer when it has focus.
  useEffect(() => {
    if (!open) return
    function onKeyDown(event: KeyboardEvent) {
      if (event.key === 'Escape') onClose()
    }
    window.addEventListener('keydown', onKeyDown)
    return () => window.removeEventListener('keydown', onKeyDown)
  }, [open, onClose])

  if (!open) return null

  return (
    <>
      {/* Backdrop — click-to-close, dimmed but not opaque so the
          PlayerBar at the bottom stays visible. aria-hidden because
          the dialog itself owns the modal role. */}
      <div
        aria-hidden="true"
        onClick={onClose}
        className="fixed inset-0 z-40 bg-black/30 backdrop-blur-sm"
      />
      <aside
        ref={dialogRef}
        id="player-queue-panel"
        role="dialog"
        aria-modal="true"
        aria-label="Playback queue"
        tabIndex={-1}
        className="fixed inset-y-0 right-0 z-50 flex w-full max-w-md flex-col border-l border-(--line) bg-(--header-bg) backdrop-blur-lg"
      >
        <header className="flex flex-shrink-0 items-center justify-between border-b border-(--line) px-4 py-3">
          <h2 className="text-base font-semibold text-(--sea-ink)">Queue</h2>
          <button
            ref={closeButtonRef}
            type="button"
            onClick={onClose}
            aria-label="Close queue"
            className="flex h-9 w-9 items-center justify-center rounded-full text-(--sea-ink) transition hover:bg-(--link-bg-hover)"
          >
            <svg viewBox="0 0 24 24" width="20" height="20" aria-hidden="true">
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

        <div className="flex-1 overflow-y-auto px-4 py-4">
          {player.current && (
            <section className="mb-6">
              <h3 className="mb-2 text-xs font-semibold uppercase tracking-wider text-(--sea-ink-soft)">
                Now playing
              </h3>
              <NowPlayingRow current={player.current} />
            </section>
          )}

          <section className="mb-6">
            <h3 className="mb-2 text-xs font-semibold uppercase tracking-wider text-(--sea-ink-soft)">
              Next up
            </h3>
            {player.queue.length === 0 ? (
              <p className="text-sm text-(--sea-ink-soft)">The queue is empty.</p>
            ) : (
              <ul className="flex flex-col gap-1">
                {player.queue.map((entry, index) => (
                  <QueueRow
                    key={`${entry.trackId}-${index}`}
                    entry={entry}
                    onJump={() => {
                      void player.playQueueAt(index)
                    }}
                  />
                ))}
              </ul>
            )}
          </section>

          {player.history.length > 0 && (
            <section>
              <h3 className="mb-2 text-xs font-semibold uppercase tracking-wider text-(--sea-ink-soft)">
                Recently played
              </h3>
              <ul className="flex flex-col gap-1">
                {player.history.map((entry, index) => (
                  <HistoryRow key={`${entry.trackId}-${index}`} entry={entry} />
                ))}
              </ul>
            </section>
          )}
        </div>
      </aside>
    </>
  )
}

function NowPlayingRow({ current }: { current: PlayingTrack }) {
  return (
    <div className="flex items-center gap-3 rounded-xl border border-(--line) bg-(--chip-bg) px-3 py-2.5">
      <div
        className="flex h-10 w-10 flex-shrink-0 items-center justify-center rounded-lg"
        style={{ backgroundColor: 'var(--accent-100)', color: 'var(--accent-700)' }}
      >
        <WaveflowLogo size={20} label={null} />
      </div>
      <div className="min-w-0 flex-1">
        <p className="truncate text-sm font-semibold text-(--sea-ink)">{current.title}</p>
        {current.artist && (
          <p className="truncate text-xs text-(--sea-ink-soft)">{current.artist}</p>
        )}
      </div>
      <span className="text-xs tabular-nums text-(--sea-ink-soft)">
        {formatDuration(current.durationMs)}
      </span>
    </div>
  )
}

function QueueRow({ entry, onJump }: { entry: QueueEntry; onJump: () => void }) {
  return (
    <li>
      <button
        type="button"
        onClick={onJump}
        aria-label={`Jump to ${entry.title}`}
        className="flex w-full items-center gap-3 rounded-lg px-2 py-1.5 text-left transition hover:bg-(--link-bg-hover)"
      >
        <span
          className="flex h-8 w-8 flex-shrink-0 items-center justify-center rounded-md"
          style={{ backgroundColor: 'var(--accent-100)', color: 'var(--accent-700)' }}
        >
          <WaveflowLogo size={16} label={null} />
        </span>
        <span className="min-w-0 flex-1">
          <span className="block truncate text-sm text-(--sea-ink)">{entry.title}</span>
          {entry.artist && (
            <span className="block truncate text-xs text-(--sea-ink-soft)">{entry.artist}</span>
          )}
        </span>
        <span className="text-xs tabular-nums text-(--sea-ink-soft)">
          {formatDuration(entry.durationMs)}
        </span>
      </button>
    </li>
  )
}

function HistoryRow({ entry }: { entry: QueueEntry }) {
  return (
    <li className="flex items-center gap-3 rounded-lg px-2 py-1.5 opacity-70">
      <span
        className="flex h-8 w-8 flex-shrink-0 items-center justify-center rounded-md"
        style={{ backgroundColor: 'var(--accent-100)', color: 'var(--accent-700)' }}
      >
        <WaveflowLogo size={16} label={null} />
      </span>
      <span className="min-w-0 flex-1">
        <span className="block truncate text-sm text-(--sea-ink)">{entry.title}</span>
        {entry.artist && (
          <span className="block truncate text-xs text-(--sea-ink-soft)">{entry.artist}</span>
        )}
      </span>
      <span className="text-xs tabular-nums text-(--sea-ink-soft)">
        {formatDuration(entry.durationMs)}
      </span>
    </li>
  )
}
