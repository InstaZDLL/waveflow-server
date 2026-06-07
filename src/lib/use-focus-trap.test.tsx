// useFocusTrap tests — exercise the Tab/Shift+Tab wrap behaviour
// and the no-focusable fallback (pin to container). jsdom doesn't
// move focus on Tab natively, so we simulate by manually setting
// focus to the first / last button, dispatching a Tab keydown on
// the container, and asserting the hook moved focus to the other
// end of the chain.

import { useRef, useState } from 'react'
import { describe, expect, it } from 'vitest'
import { fireEvent, render, screen } from '@testing-library/react'

import { useFocusTrap } from './use-focus-trap'

function Harness({
  initialActive = true,
  withButtons = true,
}: {
  initialActive?: boolean
  withButtons?: boolean
}) {
  const containerRef = useRef<HTMLDivElement | null>(null)
  const [active, setActive] = useState(initialActive)
  useFocusTrap(active, containerRef)
  return (
    <>
      <button type="button" data-testid="outside-before">
        outside before
      </button>
      <div ref={containerRef} tabIndex={-1} data-testid="trap-container">
        {withButtons && (
          <>
            <button type="button" data-testid="first">
              first
            </button>
            <button type="button" data-testid="middle">
              middle
            </button>
            <button type="button" data-testid="last">
              last
            </button>
          </>
        )}
      </div>
      <button type="button" data-testid="outside-after">
        outside after
      </button>
      <button type="button" data-testid="toggle" onClick={() => setActive((a) => !a)}>
        toggle
      </button>
    </>
  )
}

describe('useFocusTrap', () => {
  it('wraps Tab from the last focusable back to the first', () => {
    render(<Harness />)
    const last = screen.getByTestId('last')
    last.focus()
    expect(document.activeElement).toBe(last)
    fireEvent.keyDown(screen.getByTestId('trap-container'), { key: 'Tab' })
    expect(document.activeElement).toBe(screen.getByTestId('first'))
  })

  it('wraps Shift+Tab from the first focusable back to the last', () => {
    render(<Harness />)
    const first = screen.getByTestId('first')
    first.focus()
    fireEvent.keyDown(screen.getByTestId('trap-container'), { key: 'Tab', shiftKey: true })
    expect(document.activeElement).toBe(screen.getByTestId('last'))
  })

  it('leaves Tab alone when focus is in the middle of the chain', () => {
    render(<Harness />)
    const middle = screen.getByTestId('middle')
    middle.focus()
    fireEvent.keyDown(screen.getByTestId('trap-container'), { key: 'Tab' })
    // The hook only forces a wrap from the boundaries (first/last)
    // or when focus has drifted outside the container. Middle
    // sits in neither bucket, so focus stays where it was — the
    // browser's native Tab behaviour takes over.
    expect(document.activeElement).toBe(middle)
  })

  it('pins focus to the container when nothing is focusable', () => {
    render(<Harness withButtons={false} />)
    const container = screen.getByTestId('trap-container')
    container.focus()
    fireEvent.keyDown(container, { key: 'Tab' })
    expect(document.activeElement).toBe(container)
  })

  it('does not intercept Tab when inactive', () => {
    render(<Harness initialActive={false} />)
    const middle = screen.getByTestId('middle')
    middle.focus()
    fireEvent.keyDown(screen.getByTestId('trap-container'), { key: 'Tab' })
    // No listener attached, so the synthetic event is a no-op
    // and focus stays on middle (where it was).
    expect(document.activeElement).toBe(middle)
  })

  it('pulls outside focus back to the first focusable on Tab', () => {
    render(<Harness />)
    const outside = screen.getByTestId('outside-before')
    outside.focus()
    fireEvent.keyDown(screen.getByTestId('trap-container'), { key: 'Tab' })
    expect(document.activeElement).toBe(screen.getByTestId('first'))
  })
})
