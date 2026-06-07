// useFocusTrap — keep Tab navigation inside a container so a modal
// dialog doesn't let focus leak to the page underneath. WAI-ARIA
// Authoring Practices requires the trap for `aria-modal="true"`
// to be honest about its semantics.
//
// The hook is intentionally small: it queries the container for
// focusable descendants every time Tab is pressed (cheap — there
// are usually <20 of them) so a dynamic re-render of the dialog
// doesn't desync the trap from the current DOM.
//
// Restoration of focus on close is the PARENT's responsibility —
// every dialog in this app already wires `setOpen(false)` next to
// `triggerRef.current?.focus()` in its close handler, and pushing
// the same concern into the hook would yank focus inappropriately
// when the user themselves clicked outside (e.g. closed the
// dialog by clicking the toggle button that owns the trigger).

import { useEffect, type RefObject } from 'react'

const FOCUSABLE_SELECTOR = [
  'a[href]',
  'button:not([disabled])',
  'textarea:not([disabled])',
  'input:not([disabled]):not([type="hidden"])',
  'select:not([disabled])',
  '[tabindex]:not([tabindex="-1"])',
].join(',')

/**
 * @param active   — turn the trap on/off (typically the dialog's
 *                   open state).
 * @param containerRef — ref to the element whose subtree forms
 *                       the trap's boundary.
 */
export function useFocusTrap(active: boolean, containerRef: RefObject<HTMLElement | null>): void {
  useEffect(() => {
    if (!active) return
    const container = containerRef.current
    if (!container) return

    function focusables(): HTMLElement[] {
      if (!container) return []
      // `Element.checkVisibility()` is the standards-track replacement
      // for the old `offsetParent !== null` trick — it returns
      // false for `display: none`, `visibility: hidden`, and
      // dimensionless elements in a real browser; jsdom doesn't
      // implement it, returning `undefined`, which we OR-with-true
      // to fall through. Works in Chromium 105+ / Firefox 125+ /
      // Safari 17.4+; older browsers see the fallback (no filter)
      // which is the same posture this hook took before the
      // method shipped.
      return Array.from(container.querySelectorAll<HTMLElement>(FOCUSABLE_SELECTOR)).filter(
        (el) =>
          !el.hasAttribute('aria-hidden') &&
          (el.checkVisibility?.({ checkOpacity: false, checkVisibilityCSS: true }) ?? true),
      )
    }

    function onKeyDown(event: KeyboardEvent) {
      if (event.key !== 'Tab') return
      const items = focusables()
      if (items.length === 0) {
        // Nothing focusable inside — keep focus pinned to the
        // container itself so it doesn't leak to the page behind.
        event.preventDefault()
        container?.focus()
        return
      }
      const first = items[0]
      const last = items[items.length - 1]
      const activeEl = document.activeElement as HTMLElement | null
      if (event.shiftKey && activeEl === first) {
        event.preventDefault()
        last.focus()
      } else if (!event.shiftKey && activeEl === last) {
        event.preventDefault()
        first.focus()
      } else if (activeEl && !container?.contains(activeEl)) {
        // Focus drifted outside (clicked something or programmatic
        // focus jumped) — pull it back to the first focusable.
        event.preventDefault()
        first.focus()
      }
    }

    container.addEventListener('keydown', onKeyDown)
    return () => container.removeEventListener('keydown', onKeyDown)
  }, [active, containerRef])
}
