// PlaylistFormDialog — shared modal form for both create + edit
// flows. Same dialog plumbing as QueuePanel / NowPlayingOverlay
// (role="dialog" + aria-modal + focus trap + ESC + backdrop).
// Submission is delegated to a `submit` callback prop that the
// parent wires to either `createPlaylist` or `updatePlaylist`,
// so the dialog doesn't itself know which server-fn to call —
// keeps it reusable and easy to test.
//
// Smart-playlist creation / editing isn't supported (server
// hardcodes is_smart=0 on insert, refuses to PATCH a smart row);
// the dialog deliberately doesn't expose the toggle, the desktop
// app remains the editor for smart rules.

import { useEffect, useId, useRef, useState } from 'react'

import { useFocusTrap } from '@/lib/use-focus-trap'
import type { Playlist } from '@/server-fns/playlists'

const NAME_MAX = 200
const DESCRIPTION_MAX = 1000

export interface PlaylistFormValues {
  name: string
  description?: string
}

export interface PlaylistFormDialogProps {
  open: boolean
  /** `'create'` shows empty fields + a "Create playlist" submit. */
  mode: 'create' | 'edit'
  /**
   * Initial form values. Always read on the rising edge of `open`
   * (false → true) so closing then reopening the dialog reverts
   * any unsubmitted changes.
   */
  initial?: PlaylistFormValues
  onClose: () => void
  /**
   * The actual write. Receives the trimmed form values; returns
   * the up-to-date playlist row on success. The dialog's submit
   * button reflects `'Saving…' / 'Creating…'` while the promise
   * is pending and surfaces a rejection as an inline alert.
   */
  submit: (values: PlaylistFormValues) => Promise<Playlist>
  /** Fires with the row `submit` resolved to. */
  onSubmitted: (playlist: Playlist) => void
}

export function PlaylistFormDialog({
  open,
  mode,
  initial,
  onClose,
  submit,
  onSubmitted,
}: PlaylistFormDialogProps) {
  const dialogRef = useRef<HTMLDivElement | null>(null)
  const nameInputRef = useRef<HTMLInputElement | null>(null)
  const [name, setName] = useState(initial?.name ?? '')
  const [description, setDescription] = useState(initial?.description ?? '')
  const [error, setError] = useState<string | null>(null)
  const [submitting, setSubmitting] = useState(false)
  const headingId = useId()

  useFocusTrap(open, dialogRef)

  // Reset on the rising edge of `open` using the React-docs
  // "adjust state on prop change" pattern. Reads the LATEST
  // `initial` value at that moment so a parent that swaps the
  // edited playlist mid-life paints the new defaults next time
  // the user reopens the dialog.
  const [lastOpen, setLastOpen] = useState(open)
  if (lastOpen !== open) {
    setLastOpen(open)
    if (open) {
      setName(initial?.name ?? '')
      setDescription(initial?.description ?? '')
      setError(null)
      setSubmitting(false)
    }
  }

  useEffect(() => {
    if (open) nameInputRef.current?.focus()
  }, [open])

  // ESC + backdrop dismissals MUST NOT close while the submit
  // is in flight. The Cancel button is already gated via its
  // `disabled` prop. Sync the ref via effect because
  // react-hooks/refs rejects a write at render time.
  const submittingRef = useRef(submitting)
  useEffect(() => {
    submittingRef.current = submitting
  }, [submitting])
  function attemptClose() {
    if (submittingRef.current) return
    onClose()
  }

  useEffect(() => {
    if (!open) return
    function onKeyDown(event: KeyboardEvent) {
      if (event.key !== 'Escape') return
      if (submittingRef.current) return
      onClose()
    }
    window.addEventListener('keydown', onKeyDown)
    return () => window.removeEventListener('keydown', onKeyDown)
  }, [open, onClose])

  if (!open) return null

  async function onFormSubmit(event: React.FormEvent<HTMLFormElement>) {
    event.preventDefault()
    const trimmedName = name.trim()
    if (!trimmedName) {
      setError('Name is required.')
      return
    }
    setError(null)
    setSubmitting(true)
    const trimmedDescription = description.trim()
    try {
      const playlist = await submit({
        name: trimmedName,
        // For edit mode we send an explicit empty string when the
        // user cleared the description, because the server treats
        // the key's presence as "set to this value". For create
        // mode the parent's `submit` may choose to drop the empty
        // string; that's outside the dialog's concern.
        ...(trimmedDescription || mode === 'edit' ? { description: trimmedDescription } : {}),
      })
      onSubmitted(playlist)
    } catch (err) {
      const fallback = mode === 'create' ? 'Could not create playlist.' : 'Could not save changes.'
      setError(err instanceof Error ? err.message : fallback)
    } finally {
      setSubmitting(false)
    }
  }

  const heading = mode === 'create' ? 'Create playlist' : 'Edit playlist'
  const idleLabel = mode === 'create' ? 'Create playlist' : 'Save changes'
  const busyLabel = mode === 'create' ? 'Creating…' : 'Saving…'

  return (
    <>
      <div
        aria-hidden="true"
        onClick={attemptClose}
        className="fixed inset-0 z-40 bg-black/40 backdrop-blur-sm"
      />
      <div
        ref={dialogRef}
        role="dialog"
        aria-modal="true"
        aria-labelledby={headingId}
        tabIndex={-1}
        className="fixed inset-x-0 top-1/2 z-50 mx-auto w-full max-w-md -translate-y-1/2 px-4"
      >
        <div className="panel panel-pad">
          <h2 id={headingId} className="text-xl font-bold text-[var(--sea-ink)]">
            {heading}
          </h2>
          <form onSubmit={onFormSubmit} noValidate className="mt-4 flex flex-col gap-3">
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
                className="input text-base"
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
                className="textarea resize-none text-sm"
              />
            </label>

            {error && (
              <p role="alert" className="error-card text-sm font-medium">
                {error}
              </p>
            )}

            <div className="mt-2 flex items-center justify-end gap-2">
              <button
                type="button"
                onClick={onClose}
                disabled={submitting}
                className="button button-ghost"
              >
                Cancel
              </button>
              <button type="submit" disabled={submitting} className="button button-primary">
                {submitting ? busyLabel : idleLabel}
              </button>
            </div>
          </form>
        </div>
      </div>
    </>
  )
}
