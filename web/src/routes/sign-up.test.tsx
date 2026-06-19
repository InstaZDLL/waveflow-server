// Sign-up is paused for the 1.5.0 cut (see `sign-up.tsx` for the
// rationale). The original form-validation suite is preserved in
// git — restore it alongside the form in 1.6.0.

import type { PropsWithChildren } from 'react'
import { describe, expect, it, vi } from 'vitest'
import { render, screen } from '@testing-library/react'

vi.mock('@tanstack/react-router', () => ({
  createFileRoute: () => (config: unknown) => config,
  Link: ({ children, ...rest }: PropsWithChildren<Record<string, unknown>>) => (
    <a {...rest}>{children}</a>
  ),
}))

const { SignUp } = await import('./sign-up')

describe('sign-up (paused)', () => {
  it('renders the paused-account notice instead of the form', () => {
    render(<SignUp />)
    expect(screen.getByText(/temporarily disabled/i)).toBeTruthy()
    // No form field should be reachable while sign-ups are paused.
    expect(screen.queryByLabelText(/password/i)).toBeNull()
  })
})
