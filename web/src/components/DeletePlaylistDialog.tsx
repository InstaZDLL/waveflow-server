// DeletePlaylistDialog — confirmation modal for the destructive
// path on the playlist detail page. Same dialog plumbing as the
// form dialog (role=dialog + aria-modal + focus trap + ESC +
// backdrop), but no inputs — just a name echo and a Delete /
// Cancel pair. Autofocus lands on the Cancel button so the
// destructive choice isn't a single-keystroke confirmation.

import { useEffect, useId, useRef, useState } from 'react'

import { useFocusTrap } from '@/lib/use-focus-trap'

export interface DeletePlaylistDialogProps {
  open: boolean
  /** Display name shown in the confirmation copy. */
  playlistName: string
  onClose: () => void
  /**
   * The actual destructive write. Resolved → the dialog fires
   * `onDeleted`; rejected → the dialog surfaces the error inline
   * and stays open so the user can retry or cancel.
   */
  submit: () => Promise<void>
  /** Fires once `submit` resolves. */
  onDeleted: () => void
}

export function DeletePlaylistDialog({
  open,
  playlistName,
  onClose,
  submit,
  onDeleted,
}: DeletePlaylistDialogProps) {
  const dialogRef = useRef<HTMLDivElement | null>(null)
  const cancelButtonRef = useRef<HTMLButtonElement | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [submitting, setSubmitting] = useState(false)
  const headingId = useId()

  useFocusTrap(open, dialogRef)

  // Reset transient state (error + busy) on the rising edge of
  // `open` so a previous attempt's failure copy doesn't leak into
  // a fresh open.
  const [lastOpen, setLastOpen] = useState(open)
  if (lastOpen !== open) {
    setLastOpen(open)
    if (open) {
      setError(null)
      setSubmitting(false)
    }
  }

  // Cancel is the autofocused button — a destructive confirm
  // shouldn't be one Enter-keystroke away.
  useEffect(() => {
    if (open) cancelButtonRef.current?.focus()
  }, [open])

  // ESC + backdrop dismissals MUST NOT close while the request
  // is in flight. The Cancel button is already gated via its
  // `disabled` prop; this read-the-current-submitting-via-ref
  // pattern keeps the keydown listener stable (no re-attach
  // every flip of `submitting`). Sync via effect because
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

  async function onConfirm() {
    setError(null)
    setSubmitting(true)
    try {
      await submit()
      onDeleted()
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Could not delete playlist.')
    } finally {
      setSubmitting(false)
    }
  }

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
            Delete playlist
          </h2>
          <p className="mt-3 text-sm text-[var(--sea-ink-soft)]">
            <span className="font-semibold text-[var(--sea-ink)]">{playlistName}</span> will be
            removed permanently. The tracks themselves are not deleted; they stay in your library.
          </p>

          {error && (
            <p role="alert" className="error-card mt-4 text-sm font-medium">
              {error}
            </p>
          )}

          <div className="mt-5 flex items-center justify-end gap-2">
            <button
              ref={cancelButtonRef}
              type="button"
              onClick={onClose}
              disabled={submitting}
              className="button button-ghost"
            >
              Cancel
            </button>
            <button
              type="button"
              onClick={onConfirm}
              disabled={submitting}
              className="button button-danger"
            >
              {submitting ? 'Deleting…' : 'Delete playlist'}
            </button>
          </div>
        </div>
      </div>
    </>
  )
}
