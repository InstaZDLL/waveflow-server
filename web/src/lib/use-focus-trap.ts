// useFocusTrap — keep Tab navigation inside a container so a modal
// dialog doesn't let focus leak to the page underneath. WAI-ARIA
// Authoring Practices requires the trap for `aria-modal="true"`
// to be honest about its semantics.
//
// The hook is intentionally small: it queries the container for
// focusable descendants every time Tab is pressed (cheap — there
// are usually <20 of them) so a dynamic re-render of the dialog
// doesn't desync the trap from the current DOM. The listener
// lives on `document` (capture phase) — a container-scoped
// listener would miss Tab events fired outside the dialog, which
// is exactly the case the "pulled back to first" branch exists
// to handle. Reading `containerRef.current` inside the handler
// also makes the trap robust against a consumer that swaps the
// underlying node mid-life (e.g. a portal re-target) without a
// re-attach cycle.
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

function focusables(container: HTMLElement): HTMLElement[] {
  // `Element.checkVisibility()` is the standards-track replacement
  // for the old `offsetParent !== null` trick — it returns false
  // for `display: none`, `visibility: hidden`, and dimensionless
  // elements in a real browser; jsdom doesn't implement it,
  // returning `undefined`, which we OR-with-true to fall through.
  // Works in Chromium 105+ / Firefox 125+ / Safari 17.4+; older
  // browsers see the fallback (no filter) which is the same
  // posture this hook took before the method shipped.
  return Array.from(container.querySelectorAll<HTMLElement>(FOCUSABLE_SELECTOR)).filter(
    (el) =>
      !el.hasAttribute('aria-hidden') &&
      (el.checkVisibility?.({ checkOpacity: false, checkVisibilityCSS: true }) ?? true),
  )
}

/**
 * @param active   — turn the trap on/off (typically the dialog's
 *                   open state).
 * @param containerRef — ref to the element whose subtree forms
 *                       the trap's boundary.
 */
export function useFocusTrap(active: boolean, containerRef: RefObject<HTMLElement | null>): void {
  useEffect(() => {
    if (!active) return

    function onKeyDown(event: KeyboardEvent) {
      if (event.key !== 'Tab') return
      const container = containerRef.current
      if (!container) return
      const items = focusables(container)
      if (items.length === 0) {
        // Nothing focusable inside — keep focus pinned to the
        // container itself so it doesn't leak to the page behind.
        // `HTMLElement.focus()` silently no-ops when the target
        // has no `tabindex`, which would let Tab fall through to
        // the page underneath. Stamp a transient `tabindex="-1"`
        // (programmatic-only, not Tab-reachable), focus, then
        // restore the prior tabindex state so the DOM contract
        // stays intact for next renders.
        event.preventDefault()
        const priorTabIndex = container.getAttribute('tabindex')
        if (priorTabIndex === null) {
          container.setAttribute('tabindex', '-1')
        }
        container.focus()
        if (priorTabIndex === null) {
          container.removeAttribute('tabindex')
        }
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
      } else if (activeEl && !container.contains(activeEl)) {
        // Focus drifted outside (clicked something or programmatic
        // focus jumped) — pull it back to the first focusable.
        event.preventDefault()
        first.focus()
      }
    }

    document.addEventListener('keydown', onKeyDown, true)
    return () => document.removeEventListener('keydown', onKeyDown, true)
  }, [active, containerRef])
}
