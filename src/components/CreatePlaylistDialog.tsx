// CreatePlaylistDialog — modal form for the create flow on the
// listing page. Mirrors the QueuePanel / NowPlayingOverlay pattern:
// role="dialog" + aria-modal + focus trap + ESC + backdrop. Submits
// through `createPlaylist`; on success calls `onCreated` with the
// new playlist so the parent can navigate to its detail page.
//
// Smart-playlist creation isn't supported (server hardcodes
// is_smart=0 on insert) — the dialog deliberately doesn't expose
// it; the desktop app remains the editor for smart rules.

import { useEffect, useId, useRef, useState } from 'react'

import { useFocusTrap } from '@/lib/use-focus-trap'
import { createPlaylist, type Playlist } from '@/server-fns/playlists'

export interface CreatePlaylistDialogProps {
  open: boolean
  profileId: number
  onClose: () => void
  /** Fires on a 201 with the new playlist row from the server. */
  onCreated: (playlist: Playlist) => void
}

const NAME_MAX = 200
const DESCRIPTION_MAX = 1000

export function CreatePlaylistDialog({
  open,
  profileId,
  onClose,
  onCreated,
}: CreatePlaylistDialogProps) {
  const dialogRef = useRef<HTMLDivElement | null>(null)
  const nameInputRef = useRef<HTMLInputElement | null>(null)
  const [name, setName] = useState('')
  const [description, setDescription] = useState('')
  const [error, setError] = useState<string | null>(null)
  const [submitting, setSubmitting] = useState(false)
  const headingId = useId()

  useFocusTrap(open, dialogRef)

  // Reset form fields each time the dialog opens. The "adjust state
  // on prop change" pattern from the React docs — keeps the reset
  // visible-before-paint instead of after a render cycle.
  const [lastOpen, setLastOpen] = useState(open)
  if (lastOpen !== open) {
    setLastOpen(open)
    if (open) {
      setName('')
      setDescription('')
      setError(null)
      setSubmitting(false)
    }
  }

  useEffect(() => {
    if (open) nameInputRef.current?.focus()
  }, [open])

  useEffect(() => {
    if (!open) return
    function onKeyDown(event: KeyboardEvent) {
      if (event.key === 'Escape') onClose()
    }
    window.addEventListener('keydown', onKeyDown)
    return () => window.removeEventListener('keydown', onKeyDown)
  }, [open, onClose])

  if (!open) return null

  async function onSubmit(event: React.FormEvent<HTMLFormElement>) {
    event.preventDefault()
    const trimmed = name.trim()
    if (!trimmed) {
      setError('Name is required.')
      return
    }
    setError(null)
    setSubmitting(true)
    try {
      const playlist = await createPlaylist({
        data: {
          profileId,
          name: trimmed,
          ...(description.trim() ? { description: description.trim() } : {}),
        },
      })
      onCreated(playlist)
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Could not create playlist.')
    } finally {
      setSubmitting(false)
    }
  }

  return (
    <>
      <div
        aria-hidden="true"
        onClick={onClose}
        className="fixed inset-0 z-40 bg-black/40 backdrop-blur-sm"
      />
      <div
        ref={dialogRef}
        role="dialog"
        aria-modal="true"
        aria-labelledby={headingId}
        tabIndex={-1}
        className="fixed inset-x-0 top-1/2 z-50 mx-auto -translate-y-1/2 w-full max-w-md px-4"
      >
        <div className="rounded-2xl border border-[var(--line)] bg-[var(--header-bg)] p-6 shadow-[0_20px_60px_rgba(0,0,0,0.2)] backdrop-blur-lg">
          <h2 id={headingId} className="text-xl font-bold text-[var(--sea-ink)]">
            Create playlist
          </h2>
          <form onSubmit={onSubmit} noValidate className="mt-4 flex flex-col gap-3">
            <label className="flex flex-col gap-1 text-sm font-medium text-[var(--sea-ink)]">
              Name
              <input
                ref={nameInputRef}
                type="text"
                name="name"
                autoComplete="off"
                required
                maxLength={NAME_MAX}
                value={name}
                onChange={(e) => setName(e.target.value)}
                className="rounded-xl border border-[var(--line)] bg-white/80 px-3 py-2 text-base text-[var(--sea-ink)] outline-none transition focus:border-[var(--sea-ink)] focus:ring-2 dark:bg-black/30"
                style={{
                  // The slider/picker accent already retints with the
                  // theme; keep the focus ring colour consistent so
                  // the form doesn't read brand-Sea-ink on a Lavender
                  // theme.
                  ['--tw-ring-color' as string]:
                    'color-mix(in oklab, var(--accent-500) 30%, transparent)',
                }}
              />
            </label>
            <label className="flex flex-col gap-1 text-sm font-medium text-[var(--sea-ink)]">
              Description <span className="font-normal text-[var(--sea-ink-soft)]">(optional)</span>
              <textarea
                name="description"
                rows={3}
                maxLength={DESCRIPTION_MAX}
                value={description}
                onChange={(e) => setDescription(e.target.value)}
                className="resize-none rounded-xl border border-[var(--line)] bg-white/80 px-3 py-2 text-sm text-[var(--sea-ink)] outline-none transition focus:border-[var(--sea-ink)] dark:bg-black/30"
              />
            </label>

            {error && (
              <p role="alert" className="text-sm font-medium text-red-600 dark:text-red-400">
                {error}
              </p>
            )}

            <div className="mt-2 flex items-center justify-end gap-2">
              <button
                type="button"
                onClick={onClose}
                disabled={submitting}
                className="rounded-xl border border-[var(--line)] bg-[var(--chip-bg)] px-4 py-2 text-sm font-semibold text-[var(--sea-ink)] transition hover:opacity-90 disabled:cursor-not-allowed disabled:opacity-60"
              >
                Cancel
              </button>
              <button
                type="submit"
                disabled={submitting}
                className="rounded-xl px-4 py-2 text-sm font-semibold text-white transition hover:opacity-90 disabled:cursor-not-allowed disabled:opacity-60"
                style={{ backgroundColor: 'var(--accent-600)' }}
              >
                {submitting ? 'Creating…' : 'Create playlist'}
              </button>
            </div>
          </form>
        </div>
      </div>
    </>
  )
}
